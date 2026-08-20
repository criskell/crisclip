use anyhow::{bail, Context, Result};
use arboard::{Clipboard, ImageData};
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder, ImageFormat};
use std::borrow::Cow;

const KIND_TEXT: u8 = 0;
const KIND_IMAGE: u8 = 1;

pub enum Payload {
    Text(String),
    Image {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    },
}

impl Payload {
    pub fn fingerprint(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        match self {
            Payload::Text(text) => {
                hasher.update(&[KIND_TEXT]);
                hasher.update(text.as_bytes());
            }
            Payload::Image {
                width,
                height,
                rgba,
            } => {
                hasher.update(&[KIND_IMAGE]);
                hasher.update(&(*width as u64).to_be_bytes());
                hasher.update(&(*height as u64).to_be_bytes());
                hasher.update(rgba);
            }
        }
        *hasher.finalize().as_bytes()
    }

    pub fn describe(&self) -> String {
        match self {
            Payload::Text(text) => format!("text ({} bytes)", text.len()),
            Payload::Image { width, height, .. } => format!("image {width}x{height}"),
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        match self {
            Payload::Text(text) => {
                let mut out = Vec::with_capacity(text.len() + 1);
                out.push(KIND_TEXT);
                out.extend_from_slice(text.as_bytes());
                Ok(out)
            }
            Payload::Image {
                width,
                height,
                rgba,
            } => {
                let mut png = Vec::new();
                PngEncoder::new(&mut png)
                    .write_image(
                        rgba,
                        *width as u32,
                        *height as u32,
                        ExtendedColorType::Rgba8,
                    )
                    .context("failed to encode PNG")?;
                let mut out = Vec::with_capacity(png.len() + 1);
                out.push(KIND_IMAGE);
                out.extend_from_slice(&png);
                Ok(out)
            }
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let (kind, body) = bytes.split_first().context("empty payload")?;
        match *kind {
            KIND_TEXT => Ok(Payload::Text(
                String::from_utf8(body.to_vec()).context("text is not valid UTF-8")?,
            )),
            KIND_IMAGE => {
                let decoded = image::load_from_memory_with_format(body, ImageFormat::Png)
                    .context("failed to decode PNG")?
                    .to_rgba8();
                Ok(Payload::Image {
                    width: decoded.width() as usize,
                    height: decoded.height() as usize,
                    rgba: decoded.into_raw(),
                })
            }
            other => bail!("unknown payload kind: {other}"),
        }
    }
}

pub fn read(clipboard: &mut Clipboard) -> Option<Payload> {
    if let Ok(image) = clipboard.get_image() {
        return Some(Payload::Image {
            width: image.width,
            height: image.height,
            rgba: image.bytes.into_owned(),
        });
    }
    match clipboard.get_text() {
        Ok(text) if !text.is_empty() => Some(Payload::Text(text)),
        _ => None,
    }
}

pub fn write(clipboard: &mut Clipboard, payload: &Payload) -> Result<()> {
    match payload {
        Payload::Text(text) => clipboard.set_text(text.clone())?,
        Payload::Image {
            width,
            height,
            rgba,
        } => clipboard.set_image(ImageData {
            width: *width,
            height: *height,
            bytes: Cow::Borrowed(rgba),
        })?,
    }
    Ok(())
}
