use arboard::Clipboard;
use crisclip::clip::{self, Payload};

#[test]
#[ignore]
fn fingerprint_survives_the_real_clipboard() {
    let mut clipboard = Clipboard::new().unwrap();
    let previous = clip::read(&mut clipboard);

    let rgba: Vec<u8> = (0..320 * 200 * 4).map(|i| (i % 256) as u8).collect();
    let original = Payload::Image {
        width: 320,
        height: 200,
        rgba,
    };

    clip::write(&mut clipboard, &original).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let reread = clip::read(&mut clipboard).expect("clipboard came back empty");

    let matches = reread.fingerprint() == original.fingerprint();
    if let Some(previous) = previous {
        let _ = clip::write(&mut clipboard, &previous);
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    assert!(
        matches,
        "read back as {}, the daemon would loop",
        reread.describe()
    );
}

#[test]
#[ignore]
fn daemon_applies_received_image_without_echoing_it() {
    use crisclip::proto::Session;
    use std::io::Read;
    use std::net::TcpStream;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    let psk = [0x55u8; 32];
    let dir = std::env::temp_dir().join("crisclip-e2e");
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("config.toml");
    std::fs::write(
        &config,
        format!(
            "peer = \"127.0.0.1:47811\"\nlisten = \"127.0.0.1:47811\"\nkey = \"{}\"\npoll_ms = 200\n",
            "55".repeat(32)
        ),
    )
    .unwrap();

    let mut clipboard = Clipboard::new().unwrap();
    let previous = clip::read(&mut clipboard);

    let mut daemon = Command::new(env!("CARGO_BIN_EXE_crisclip"))
        .env("CRISCLIP_CONFIG", &config)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(700));

    let rgba: Vec<u8> = (0..200 * 150 * 4).map(|i| ((i * 7) % 256) as u8).collect();
    let sent = Payload::Image {
        width: 200,
        height: 150,
        rgba,
    };
    let stream = TcpStream::connect("127.0.0.1:47811").unwrap();
    Session::initiate(stream, &psk)
        .unwrap()
        .send(&sent.encode().unwrap())
        .unwrap();

    std::thread::sleep(Duration::from_millis(1200));
    let on_clipboard = clip::read(&mut clipboard);

    daemon.kill().unwrap();
    daemon.wait().unwrap();
    let mut log = String::new();
    daemon
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut log)
        .unwrap();
    if let Some(previous) = previous {
        let _ = clip::write(&mut clipboard, &previous);
    }

    println!("--- daemon log ---\n{log}");
    assert_eq!(
        on_clipboard.map(|p| p.fingerprint()),
        Some(sent.fingerprint()),
        "the received image never reached the clipboard"
    );
    assert!(
        !log.contains("sent:"),
        "the daemon echoed back what it just received:\n{log}"
    );
}

#[test]
#[ignore]
fn selection_events_beat_the_polling_backstop() {
    use crisclip::proto::Session;
    use std::net::TcpListener;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let psk = [0x55u8; 32];
    let inbox = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = inbox.local_addr().unwrap().port();

    let dir = std::env::temp_dir().join("crisclip-events");
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("config.toml");
    std::fs::write(
        &config,
        format!(
            "peer = \"127.0.0.1:{port}\"\nlisten = \"127.0.0.1:47814\"\nkey = \"{}\"\npoll_ms = 5000\n",
            "55".repeat(32)
        ),
    )
    .unwrap();

    let mut clipboard = Clipboard::new().unwrap();
    let previous = clip::read(&mut clipboard);

    let mut daemon = Command::new(env!("CARGO_BIN_EXE_crisclip"))
        .env("CRISCLIP_CONFIG", &config)
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(900));

    let marker = "crisclip-event-probe";
    clipboard.set_text(marker).unwrap();
    let copied_at = Instant::now();

    let (stream, _) = inbox.accept().unwrap();
    let body = Session::accept(stream, &psk)
        .unwrap()
        .recv(1 << 20)
        .unwrap();
    let elapsed = copied_at.elapsed();

    daemon.kill().unwrap();
    daemon.wait().unwrap();
    if let Some(previous) = previous {
        let _ = clip::write(&mut clipboard, &previous);
    }

    match Payload::decode(&body).unwrap() {
        Payload::Text(text) => assert_eq!(text, marker),
        other => panic!("expected text, got {}", other.describe()),
    }
    assert!(
        elapsed < Duration::from_millis(1500),
        "took {elapsed:?} with a 5000ms backstop, so the selection event never fired"
    );
    println!("reacted in {elapsed:?}");
}

#[test]
#[ignore]
fn refuses_connections_from_anyone_but_the_peer() {
    use crisclip::proto::Session;
    use std::io::Read;
    use std::net::TcpStream;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    let psk = [0x55u8; 32];
    let dir = std::env::temp_dir().join("crisclip-stranger");
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("config.toml");
    std::fs::write(
        &config,
        format!(
            "peer = \"192.0.2.1:47777\"\nlisten = \"127.0.0.1:47816\"\nkey = \"{}\"\npoll_ms = 5000\n",
            "55".repeat(32)
        ),
    )
    .unwrap();

    let mut clipboard = Clipboard::new().unwrap();
    clipboard.set_text("untouched").unwrap();

    let mut daemon = Command::new(env!("CARGO_BIN_EXE_crisclip"))
        .env("CRISCLIP_CONFIG", &config)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(900));

    let intruder = Payload::Text("injected".into()).encode().unwrap();
    let stream = TcpStream::connect("127.0.0.1:47816").unwrap();
    let delivered = Session::initiate(stream, &psk).and_then(|mut s| s.send(&intruder));

    std::thread::sleep(Duration::from_millis(400));
    let on_clipboard = clipboard.get_text().unwrap_or_default();

    daemon.kill().unwrap();
    daemon.wait().unwrap();
    let mut log = String::new();
    daemon
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut log)
        .unwrap();

    assert!(
        delivered.is_err(),
        "the daemon completed a handshake with a stranger"
    );
    assert_eq!(
        on_clipboard, "untouched",
        "a stranger wrote to the clipboard"
    );
    assert!(
        log.contains("is not the configured peer"),
        "expected a rejection in the log, got:\n{log}"
    );
}
