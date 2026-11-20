use std::fs::{self, File};
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use eframe::egui::ColorImage;
use image::{ImageReader, Limits, RgbaImage};
use reqwest::{Url, blocking::Client, redirect::Policy};
use sha2::{Digest, Sha256};

use crate::model::Device;

// These store images are smaller and correctly named for the corresponding USB
// models. Older OpenRazer builds report a Basilisk V3 photo for the V4.
const BASILISK_V3_URL: &str = "https://dl.razerzone.com/src2/14676/14676-1-en-v1.png";
const BASILISK_V4_URL: &str = "https://medias-p1.phoenix.razer.com/sys-master-phoenix-images-container/hd0/hb2/10458042105886/basilisk-v4-pro-black-500x500.png";
const VIPER_V4_SUPPORT_URL: &str = "https://dl.razerzone.com/src2/20779/20779-01-en-v1.png";
const VIPER_V4_URL: &str = "https://medias-p1.phoenix.razer.com/sys-master-phoenix-images-container/h9d/h5d/10039373168670/viper-v4-pro-black-2-500x500.png";
const MAX_BYTES: u64 = 16 * 1024 * 1024;

pub fn source(device: &Device, advertised: Option<&str>) -> Option<String> {
    let mut url = advertised
        .map(str::trim)
        .filter(|text| Url::parse(text).is_ok_and(|url| allowed(&url)));
    if device.basilisk() && matches!(url, None | Some(BASILISK_V3_URL)) {
        url = Some(BASILISK_V4_URL);
    }
    if device.vid == 0x1532
        && matches!(device.pid, 0xe5 | 0xe6)
        && matches!(url, None | Some(VIPER_V4_SUPPORT_URL))
    {
        url = Some(VIPER_V4_URL);
    }
    url.map(str::to_owned)
}

fn allowed(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.host_str().is_some_and(|host| {
            matches!(
                host,
                "assets.razerzone.com"
                    | "assets2.razerzone.com"
                    | "assets3.razerzone.com"
                    | "dl.razerzone.com"
                    | "medias-p1.phoenix.razer.com"
                    | "hybrismediaprod.blob.core.windows.net"
                    | "rzrwarranty.s3.amazonaws.com"
                    | "hwimg.nl"
            )
        })
}

pub struct Loader {
    pub requests: Sender<String>,
    pub replies: Receiver<(String, Result<ColorImage>)>,
}

impl Loader {
    pub fn start(repaint: impl Fn() + Send + 'static) -> Self {
        let (requests, incoming) = mpsc::channel::<String>();
        let (results, replies) = mpsc::channel();
        thread::spawn(move || {
            let client = http_client();
            while let Ok(url) = incoming.recv() {
                let url = incoming.try_iter().last().unwrap_or(url);
                let result = client
                    .as_ref()
                    .map_err(|error| anyhow::anyhow!("{error:#}"))
                    .and_then(|client| load(client, &url));
                if results.send((url, result)).is_err() {
                    break;
                }
                repaint();
            }
        });
        Self { requests, replies }
    }
}

fn http_client() -> Result<Client> {
    Ok(Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(10))
        .https_only(true)
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() < 4 && allowed(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("Unzulässige Weiterleitung für Produktbild")
            }
        }))
        .build()?)
}

fn load(client: &Client, source: &str) -> Result<ColorImage> {
    let url = Url::parse(source)?;
    ensure!(allowed(&url), "Unzulässige Produktbild-Adresse");
    let cache = cache_path(url.as_str());
    if let Some(path) = &cache
        && let Ok(file) = File::open(path)
        && let Ok(bytes) = read_bounded(file)
        && let Ok(image) = decode(&bytes)
    {
        return Ok(image);
    }
    let response = client
        .get(url)
        .send()
        .context("Produktbild konnte nicht geladen werden")?;
    ensure!(
        response.status().is_success(),
        "Bildserver meldet {}",
        response.status()
    );
    ensure!(
        response
            .content_length()
            .is_none_or(|size| size <= MAX_BYTES),
        "Produktbild ist zu groß"
    );
    let bytes = read_bounded(response)?;
    let image = decode(&bytes)?;
    if let Some(path) = &cache
        && let Some(parent) = path.parent()
        && fs::create_dir_all(parent).is_ok()
    {
        let temp = path.with_extension(format!("download-{}", std::process::id()));
        if fs::write(&temp, bytes).is_ok() && fs::rename(&temp, path).is_ok() {
            return Ok(image);
        }
        let _ = fs::remove_file(temp);
    }
    Ok(image)
}

fn read_bounded(input: impl Read) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    input.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_BYTES, "Produktbild ist zu groß");
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<ColorImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(192 * 1024 * 1024);
    reader.limits(limits);
    let pixels = reader
        .decode()
        .context("Bildformat konnte nicht gelesen werden")?
        .thumbnail(760, 760)
        .to_rgba8();
    let pixels = trim_transparent_padding(pixels);
    Ok(ColorImage::from_rgba_unmultiplied(
        [pixels.width() as usize, pixels.height() as usize],
        pixels.as_raw(),
    ))
}

fn trim_transparent_padding(pixels: RgbaImage) -> RgbaImage {
    let (width, height) = pixels.dimensions();
    let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);
    for (x, y, pixel) in pixels.enumerate_pixels() {
        if pixel[3] != 0 {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
    }
    if left >= right || top >= bottom || (right - left == width && bottom - top == height) {
        return pixels;
    }
    image::imageops::crop_imm(&pixels, left, top, right - left, bottom - top).to_image()
}

fn cache_path(source: &str) -> Option<PathBuf> {
    let directory = std::env::var_os("XDG_CACHE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    let hash: String = Sha256::digest(source.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Some(directory.join("pointrill/product-images").join(hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(pid: u16) -> Device {
        Device {
            serial: "test".into(),
            name: "Test mouse".into(),
            vid: 0x1532,
            pid,
        }
    }

    #[test]
    fn known_v4_devices_use_the_correct_store_photo() {
        assert_eq!(
            source(&device(0xe3), Some(BASILISK_V3_URL)).as_deref(),
            Some(BASILISK_V4_URL)
        );
        assert_eq!(
            source(&device(0xe2), None).as_deref(),
            Some(BASILISK_V4_URL)
        );
        assert_eq!(
            source(&device(0xe6), Some(VIPER_V4_SUPPORT_URL)).as_deref(),
            Some(VIPER_V4_URL)
        );
        assert_eq!(
            source(&device(0xab), Some(BASILISK_V3_URL)).as_deref(),
            Some(BASILISK_V3_URL)
        );
    }

    #[test]
    fn other_mice_keep_their_own_openrazer_url() {
        let url = "https://assets.razerzone.com/eeimages/orochi.png";
        assert_eq!(source(&device(0x94), Some(url)).as_deref(), Some(url));
        for url in [
            "",
            "file:///etc/passwd",
            "http://dl.razerzone.com/x",
            "https://dl.razerzone.com.evil.test/x",
            "https://user@dl.razerzone.com/x",
            "https://localhost/x",
        ] {
            assert_eq!(source(&device(0x94), Some(url)), None);
        }
    }

    #[test]
    #[ignore = "requires a network connection to the official Razer CDN"]
    fn both_v4_store_photos_load_and_decode() {
        let client = http_client().unwrap();
        for url in [BASILISK_V4_URL, VIPER_V4_URL] {
            let image = load(&client, url).unwrap();
            assert!(
                image.size.iter().all(|side| *side >= 200 && *side <= 760),
                "{url}"
            );
        }
    }
}
