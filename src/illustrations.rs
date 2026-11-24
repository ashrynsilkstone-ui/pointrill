use crate::model::Device;
use eframe::egui::ColorImage;

pub struct Illustration {
    pub id: &'static str,
    bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/mouse_illustrations.rs"));

pub fn for_device(device: &Device) -> Option<&'static Illustration> {
    if device.vid == 0x1532 {
        by_pid(device.pid)
    } else {
        None
    }
}

impl Illustration {
    pub fn decode(&self) -> image::ImageResult<ColorImage> {
        let pixels =
            image::load_from_memory_with_format(self.bytes, image::ImageFormat::WebP)?.into_rgba8();
        Ok(ColorImage::from_rgba_unmultiplied(
            [pixels.width() as usize, pixels.height() as usize],
            pixels.as_raw(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn all_catalog_images_decode() {
        let mut checked = HashSet::new();
        for &pid in PRODUCT_IDS {
            let illustration = by_pid(pid).expect("catalog ID resolves");
            if checked.insert(illustration.id) {
                let image = illustration.decode().expect(illustration.id);
                assert!(image.size.iter().all(|axis| (100..=512).contains(axis)));
                assert!(image.pixels.iter().any(|pixel| pixel.a() == 0));
                assert!(image.pixels.iter().any(|pixel| pixel.a() == 255));
            }
        }
    }

    #[test]
    fn v4_connections_use_the_matching_mouse() {
        for (wired, wireless, model) in [
            (0xe2, 0xe3, "basilisk-v4-pro"),
            (0xe5, 0xe6, "viper-v4-pro"),
        ] {
            assert_eq!(by_pid(wired).unwrap().id, model);
            assert_eq!(by_pid(wireless).unwrap().id, model);
        }
        assert!(by_pid(0xffff).is_none());
    }
}
