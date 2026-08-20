use anyhow::{bail, Context, Result};
use arboard::Clipboard;
use crisclip::clip::{self, Payload};
use crisclip::proto::Session;
use crisclip::watch;
use rand::RngCore;
use serde::Deserialize;
use std::collections::HashSet;
use std::net::{IpAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Deserialize)]
struct Config {
    peer: String,
    #[serde(default = "default_listen")]
    listen: String,
    key: String,
    #[serde(default = "default_poll_ms")]
    poll_ms: u64,
    #[serde(default = "default_max_bytes")]
    max_bytes: usize,
}

fn default_listen() -> String {
    "0.0.0.0:47777".into()
}

fn default_poll_ms() -> u64 {
    2000
}

fn default_max_bytes() -> usize {
    32 * 1024 * 1024
}

const SETTLE: Duration = Duration::from_millis(50);

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("keygen") => {
            let mut key = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut key);
            println!("{}", to_hex(&key));
            Ok(())
        }
        Some("init") => write_example_config(),
        Some("run") | None => run(),
        Some(other) => bail!("unknown command: {other}\nusage: crisclip [run|init|keygen]"),
    }
}

fn run() -> Result<()> {
    let path = config_path();
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}; run `crisclip init`", path.display()))?;
    warn_if_readable_by_others(&path);

    let config: Config = toml::from_str(&raw).context("invalid config.toml")?;
    let psk = parse_key(&config.key)?;
    let allowed = resolve_peer_addresses(&config.peer)?;

    let clipboard = Arc::new(Mutex::new(
        Clipboard::new().context("cannot open the clipboard")?,
    ));
    let seen = Arc::new(Mutex::new(
        clip::read(&mut clipboard.lock().unwrap()).map(|payload| payload.fingerprint()),
    ));

    let listener = TcpListener::bind(&config.listen)
        .with_context(|| format!("cannot listen on {}", config.listen))?;
    eprintln!(
        "crisclip listening on {}, peer {}",
        config.listen, config.peer
    );

    let max_bytes = config.max_bytes;
    {
        let clipboard = Arc::clone(&clipboard);
        let seen = Arc::clone(&seen);
        thread::spawn(move || {
            for incoming in listener.incoming() {
                let stream = match incoming {
                    Ok(stream) => stream,
                    Err(error) => {
                        eprintln!("failed to accept connection: {error}");
                        continue;
                    }
                };
                if let Err(error) = accept(&stream, &allowed, &psk, max_bytes, &clipboard, &seen) {
                    eprintln!("failed to receive from peer: {error:#}");
                }
            }
        });
    }

    let (ticks, changes) = std::sync::mpsc::sync_channel::<()>(1);
    match watch::spawn(ticks.clone()) {
        Ok(()) => eprintln!(
            "watching X11 selections, polling every {}ms as a backstop",
            config.poll_ms
        ),
        Err(error) => eprintln!(
            "selection events unavailable ({error:#}), polling every {}ms",
            config.poll_ms
        ),
    }
    let _keep_channel_open = ticks;

    let backstop = Duration::from_millis(config.poll_ms);
    let mut last_failure: Option<String> = None;
    loop {
        if changes.recv_timeout(backstop).is_ok() {
            thread::sleep(SETTLE);
        }
        let Some(payload) = take_local_change(&clipboard, &seen) else {
            continue;
        };
        match push(&config.peer, &psk, &payload, max_bytes) {
            Ok(()) => last_failure = None,
            Err(error) => {
                let reason = format!("{error:#}");
                if last_failure.as_deref() != Some(reason.as_str()) {
                    eprintln!("failed to send {}: {reason}", payload.describe());
                    last_failure = Some(reason);
                }
            }
        }
    }
}

fn take_local_change(
    clipboard: &Arc<Mutex<Clipboard>>,
    seen: &Arc<Mutex<Option<[u8; 32]>>>,
) -> Option<Payload> {
    let payload = clip::read(&mut clipboard.lock().unwrap())?;
    let fingerprint = payload.fingerprint();

    let mut seen = seen.lock().unwrap();
    if *seen == Some(fingerprint) {
        return None;
    }
    *seen = Some(fingerprint);
    Some(payload)
}

fn push(peer: &str, psk: &[u8; 32], payload: &Payload, max_bytes: usize) -> Result<()> {
    let body = payload.encode()?;
    if body.len() > max_bytes {
        bail!(
            "{} encodes to {} bytes, over the {max_bytes} limit",
            payload.describe(),
            body.len()
        );
    }
    let stream = TcpStream::connect(peer).with_context(|| format!("peer {peer} unreachable"))?;
    Session::initiate(stream, psk)?.send(&body)?;
    eprintln!("sent: {} ({} bytes)", payload.describe(), body.len());
    Ok(())
}

fn accept(
    stream: &TcpStream,
    allowed: &HashSet<IpAddr>,
    psk: &[u8; 32],
    max_bytes: usize,
    clipboard: &Arc<Mutex<Clipboard>>,
    seen: &Arc<Mutex<Option<[u8; 32]>>>,
) -> Result<()> {
    let origin = stream.peer_addr()?.ip();
    if !allowed.contains(&origin) {
        bail!("connection from {origin}, which is not the configured peer");
    }

    let body = Session::accept(stream.try_clone()?, psk)?.recv(max_bytes)?;
    let payload = Payload::decode(&body)?;

    let mut clipboard = clipboard.lock().unwrap();
    let mut seen = seen.lock().unwrap();
    *seen = Some(payload.fingerprint());
    clip::write(&mut clipboard, &payload)?;

    eprintln!("received: {}", payload.describe());
    Ok(())
}

fn resolve_peer_addresses(peer: &str) -> Result<HashSet<IpAddr>> {
    let addresses: HashSet<IpAddr> = peer
        .to_socket_addrs()
        .with_context(|| format!("cannot resolve peer {peer}"))?
        .map(|address| address.ip())
        .collect();
    if addresses.is_empty() {
        bail!("peer {peer} resolved to no address");
    }
    Ok(addresses)
}

fn config_path() -> PathBuf {
    if let Ok(path) = std::env::var("CRISCLIP_CONFIG") {
        return PathBuf::from(path);
    }
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
        });
    base.join("crisclip").join("config.toml")
}

fn write_example_config() -> Result<()> {
    let path = config_path();
    if path.exists() {
        bail!("{} already exists, refusing to overwrite", path.display());
    }
    std::fs::create_dir_all(path.parent().unwrap())?;

    let mut key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    let example = format!(
        "# Copy this file to both machines, changing only `peer`.\n\
         peer = \"192.168.15.7:47777\"\n\
         listen = \"0.0.0.0:47777\"\n\
         key = \"{}\"\n\
         poll_ms = 2000\n\
         max_bytes = 33554432\n",
        to_hex(&key)
    );
    std::fs::write(&path, example)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;

    println!("config written to {}", path.display());
    println!("set `peer` on both machines and keep the same `key`.");
    Ok(())
}

fn warn_if_readable_by_others(path: &PathBuf) {
    if let Ok(metadata) = std::fs::metadata(path) {
        if metadata.permissions().mode() & 0o077 != 0 {
            eprintln!("warning: {} is readable by other users", path.display());
        }
    }
}

fn parse_key(hex: &str) -> Result<[u8; 32]> {
    let hex = hex.trim();
    if hex.len() != 64 {
        bail!("the key must be 64 hex characters; run `crisclip keygen`");
    }
    let mut key = [0u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
            .context("the key has non-hex characters")?;
    }
    Ok(key)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
