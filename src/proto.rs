use anyhow::{bail, Context, Result};
use snow::{Builder, TransportState};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const PATTERN: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";

const MAX_NOISE_MESSAGE: usize = 65535;
const TAG_LEN: usize = 16;
const MAX_PLAINTEXT: usize = MAX_NOISE_MESSAGE - TAG_LEN;
const IO_TIMEOUT: Duration = Duration::from_secs(20);

pub struct Session {
    transport: TransportState,
    stream: TcpStream,
}

impl Session {
    pub fn initiate(stream: TcpStream, psk: &[u8; 32]) -> Result<Self> {
        let mut stream = prepare(stream)?;
        let mut handshake = Builder::new(PATTERN.parse()?)
            .psk(0, psk)
            .build_initiator()
            .context("cannot start the Noise handshake")?;

        let mut buf = vec![0u8; MAX_NOISE_MESSAGE];
        let len = handshake.write_message(&[], &mut buf)?;
        write_raw(&mut stream, &buf[..len])?;

        let response = read_raw(&mut stream)?;
        handshake.read_message(&response, &mut buf)?;

        Ok(Self {
            transport: handshake.into_transport_mode()?,
            stream,
        })
    }

    pub fn accept(stream: TcpStream, psk: &[u8; 32]) -> Result<Self> {
        let mut stream = prepare(stream)?;
        let mut handshake = Builder::new(PATTERN.parse()?)
            .psk(0, psk)
            .build_responder()
            .context("cannot answer the Noise handshake")?;

        let mut buf = vec![0u8; MAX_NOISE_MESSAGE];
        let request = read_raw(&mut stream)?;
        handshake.read_message(&request, &mut buf)?;

        let len = handshake.write_message(&[], &mut buf)?;
        write_raw(&mut stream, &buf[..len])?;

        Ok(Self {
            transport: handshake.into_transport_mode()?,
            stream,
        })
    }

    pub fn send(&mut self, payload: &[u8]) -> Result<()> {
        let total = u32::try_from(payload.len()).context("payload too large")?;
        self.send_frame(&total.to_be_bytes())?;
        for chunk in payload.chunks(MAX_PLAINTEXT) {
            self.send_frame(chunk)?;
        }
        self.stream.flush()?;
        Ok(())
    }

    pub fn recv(&mut self, max_bytes: usize) -> Result<Vec<u8>> {
        let header = self.recv_frame()?;
        if header.len() != 4 {
            bail!("invalid length header");
        }
        let total = u32::from_be_bytes(header.try_into().unwrap()) as usize;
        if total > max_bytes {
            bail!("peer announced {total} bytes, over the {max_bytes} limit");
        }

        let mut payload = Vec::with_capacity(total);
        while payload.len() < total {
            let chunk = self.recv_frame()?;
            if chunk.is_empty() {
                bail!("transfer cut short");
            }
            payload.extend_from_slice(&chunk);
        }
        if payload.len() != total {
            bail!("received {} bytes, expected {total}", payload.len());
        }
        Ok(payload)
    }

    fn send_frame(&mut self, plaintext: &[u8]) -> Result<()> {
        let mut buf = vec![0u8; plaintext.len() + TAG_LEN];
        let len = self.transport.write_message(plaintext, &mut buf)?;
        write_raw(&mut self.stream, &buf[..len])
    }

    fn recv_frame(&mut self) -> Result<Vec<u8>> {
        let ciphertext = read_raw(&mut self.stream)?;
        let mut buf = vec![0u8; ciphertext.len()];
        let len = self.transport.read_message(&ciphertext, &mut buf)?;
        buf.truncate(len);
        Ok(buf)
    }
}

fn prepare(stream: TcpStream) -> Result<TcpStream> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

fn write_raw(stream: &mut TcpStream, message: &[u8]) -> Result<()> {
    let len = u16::try_from(message.len()).context("Noise frame too large")?;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(message)?;
    Ok(())
}

fn read_raw(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut len = [0u8; 2];
    stream.read_exact(&mut len)?;
    let mut message = vec![0u8; u16::from_be_bytes(len) as usize];
    stream.read_exact(&mut message)?;
    Ok(message)
}
