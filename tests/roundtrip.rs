use crisclip::clip::Payload;
use crisclip::proto::Session;
use std::net::{TcpListener, TcpStream};
use std::thread;

const KEY_A: [u8; 32] = [7u8; 32];
const KEY_B: [u8; 32] = [9u8; 32];
const LIMIT: usize = 64 * 1024 * 1024;

fn echo_once(key: [u8; 32]) -> (u16, thread::JoinHandle<Option<Vec<u8>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        Session::accept(stream, &key).ok()?.recv(LIMIT).ok()
    });
    (port, handle)
}

#[test]
fn transfers_payload_larger_than_one_noise_frame() {
    let (port, server) = echo_once(KEY_A);
    let payload: Vec<u8> = (0..5 * 1024 * 1024).map(|i| (i % 251) as u8).collect();

    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    Session::initiate(stream, &KEY_A)
        .unwrap()
        .send(&payload)
        .unwrap();

    assert_eq!(server.join().unwrap(), Some(payload));
}

#[test]
fn wrong_key_breaks_the_handshake() {
    let (port, server) = echo_once(KEY_A);
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();

    let sent = Session::initiate(stream, &KEY_B).and_then(|mut s| s.send(b"secret"));
    assert!(sent.is_err() || server.join().unwrap().is_none());
}

#[test]
fn image_survives_png_encoding() {
    let rgba: Vec<u8> = (0..64 * 48 * 4).map(|i| (i % 256) as u8).collect();
    let original = Payload::Image {
        width: 64,
        height: 48,
        rgba,
    };

    let decoded = Payload::decode(&original.encode().unwrap()).unwrap();

    assert_eq!(decoded.fingerprint(), original.fingerprint());
}

#[test]
fn png_compresses_far_better_than_raw_bitmap() {
    let rgba = vec![0x20u8; 1920 * 1080 * 4];
    let raw = rgba.len();
    let png = Payload::Image {
        width: 1920,
        height: 1080,
        rgba,
    }
    .encode()
    .unwrap();

    assert!(png.len() * 100 < raw, "PNG {} vs raw {raw}", png.len());
}

#[test]
fn text_and_image_fingerprints_do_not_collide() {
    let text = Payload::Text("hi".into());
    let image = Payload::Image {
        width: 1,
        height: 1,
        rgba: vec![1, 2, 3, 4],
    };

    assert_ne!(text.fingerprint(), image.fingerprint());
}
