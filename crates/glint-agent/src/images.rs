//! Turns clipboard images into base64 payloads the provider APIs accept.

use crate::model::MessageImage;
use base64::Engine as _;
use gpui::ImageFormat;
use gpui_glint::components::PastedImage;

/// Matches the strictest provider limit (Anthropic: 5MB per image).
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// Media types both protocols accept as-is; anything else is re-encoded to PNG.
const PASSTHROUGH_FORMATS: [ImageFormat; 4] = [
    ImageFormat::Png,
    ImageFormat::Jpeg,
    ImageFormat::Gif,
    ImageFormat::Webp,
];

/// Converts a pasted image into a persisted `MessageImage`. macOS screenshots
/// arrive as TIFF (and copied files may be BMP/ICO), which no provider accepts,
/// so those are decoded and re-encoded as PNG.
pub fn payload(image: &PastedImage) -> Result<MessageImage, String> {
    validate(image)?;
    let (media_type, bytes) = normalize(image)?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("图片超过 5MB 限制，请缩小后重试".into());
    }
    Ok(MessageImage {
        media_type: media_type.into(),
        data: base64::engine::general_purpose::STANDARD
            .encode(bytes)
            .into(),
        file: None,
    })
}

pub fn prepare(images: &[PastedImage]) -> Result<Vec<MessageImage>, String> {
    if images.len() > 16 {
        return Err("一次最多发送 16 张图片".into());
    }
    let mut total = 0;
    images
        .iter()
        .map(|image| {
            let image = payload(image)?;
            total += image.data.len();
            if total > 28 * 1024 * 1024 {
                return Err("图片总大小超过 20MB，请减少图片数量".into());
            }
            Ok(image)
        })
        .collect()
}

fn validate(image: &PastedImage) -> Result<(), String> {
    if image.bytes().len() > 20 * 1024 * 1024 {
        return Err("图片原始数据超过 20MB".into());
    }
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(image.bytes()))
        .with_guessed_format()
        .map_err(|_| "无法识别图片格式")?
        .into_dimensions()
        .map_err(|_| "无法读取图片尺寸")?;
    if u64::from(width) * u64::from(height) > 25_000_000 {
        return Err("图片超过 2500 万像素，请缩小后重试".into());
    }
    Ok(())
}

#[derive(Default)]
pub struct PreviewCache {
    entries: std::collections::VecDeque<(String, std::sync::Arc<gpui::Image>)>,
    bytes: usize,
}

impl PreviewCache {
    const LIMIT: usize = 64 * 1024 * 1024;
    pub fn get(&mut self, key: &str) -> Option<std::sync::Arc<gpui::Image>> {
        let index = self
            .entries
            .iter()
            .position(|(candidate, _)| candidate == key)?;
        let entry = self.entries.remove(index)?;
        let image = entry.1.clone();
        self.entries.push_back(entry);
        Some(image)
    }
    pub fn insert(
        &mut self,
        key: String,
        image: std::sync::Arc<gpui::Image>,
    ) -> Vec<std::sync::Arc<gpui::Image>> {
        let mut retired = Vec::new();
        if image.bytes.len() > Self::LIMIT {
            return retired;
        }
        if let Some(index) = self
            .entries
            .iter()
            .position(|(candidate, _)| candidate == &key)
        {
            let old = self.entries.remove(index).unwrap().1;
            self.bytes -= old.bytes.len();
            retired.push(old);
        }
        while self.bytes + image.bytes.len() > Self::LIMIT || self.entries.len() >= 64 {
            let Some((_, old)) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= old.bytes.len();
            retired.push(old);
        }
        self.bytes += image.bytes.len();
        self.entries.push_back((key, image));
        retired
    }
    pub fn clear(&mut self) -> Vec<std::sync::Arc<gpui::Image>> {
        self.bytes = 0;
        self.entries.drain(..).map(|(_, image)| image).collect()
    }
    pub fn remove_session(&mut self, session: uuid::Uuid) -> Vec<std::sync::Arc<gpui::Image>> {
        let prefix = format!("{session}:");
        let mut retired = Vec::new();
        self.entries.retain(|(key, image)| {
            if key.starts_with(&prefix) {
                retired.push(image.clone());
                false
            } else {
                true
            }
        });
        self.bytes = self
            .entries
            .iter()
            .map(|(_, image)| image.bytes.len())
            .sum();
        retired
    }
}

pub fn release_previews(images: Vec<std::sync::Arc<gpui::Image>>, cx: &mut gpui::App) {
    for image in images {
        gpui::ImageSource::from(image).remove_asset(cx);
    }
}

/// Decodes a persisted message image back to raw bytes for UI previews.
pub fn decode(image: &MessageImage) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(image.data.as_bytes())
        .map_err(|_| "会话中的图片数据损坏".into())
}

fn normalize(image: &PastedImage) -> Result<(&'static str, Vec<u8>), String> {
    if PASSTHROUGH_FORMATS.contains(&image.format()) {
        return Ok((image.media_type(), image.bytes().to_vec()));
    }
    if image.format() == ImageFormat::Svg {
        return Err("暂不支持粘贴 SVG 图片".into());
    }
    let decoded =
        image::load_from_memory(image.bytes()).map_err(|_| "无法解析剪贴板中的图片".to_owned())?;
    let mut png = Vec::new();
    decoded
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|_| "无法转换剪贴板中的图片".to_owned())?;
    Ok(("image/png", png))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_cache_reuses_images_evicts_and_cleans_deleted_sessions() {
        use std::sync::Arc;
        let session = uuid::Uuid::new_v4();
        let key = format!("{session}:0:img0");
        let image = Arc::new(gpui::Image::from_bytes(ImageFormat::Png, vec![1]));
        let mut cache = PreviewCache::default();
        cache.insert(key.clone(), image.clone());
        assert!(Arc::ptr_eq(&cache.get(&key).unwrap(), &image));
        cache.remove_session(session);
        assert!(cache.get(&key).is_none());
        for index in 0..65 {
            cache.insert(index.to_string(), image.clone());
        }
        assert!(cache.get("0").is_none());
        assert!(cache.get("64").is_some());
        cache.clear();
        assert_eq!(cache.bytes, 0);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn png_passes_through_and_tiff_reencodes_to_png() {
        let mut png_bytes = Vec::new();
        image::DynamicImage::new_rgb8(1, 1)
            .write_to(
                &mut std::io::Cursor::new(&mut png_bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        let png = PastedImage::new(ImageFormat::Png, png_bytes.clone());
        let encoded = payload(&png).unwrap();
        assert_eq!(encoded.media_type, "image/png");
        assert_eq!(decode(&encoded).unwrap(), png_bytes);

        // A 1×1 TIFF; macOS screenshots land on the clipboard this way.
        let mut tiff_bytes = Vec::new();
        image::DynamicImage::new_rgb8(1, 1)
            .write_to(
                &mut std::io::Cursor::new(&mut tiff_bytes),
                image::ImageFormat::Tiff,
            )
            .unwrap();
        let tiff = PastedImage::new(ImageFormat::Tiff, tiff_bytes);
        let reencoded = payload(&tiff).unwrap();
        assert_eq!(reencoded.media_type, "image/png");
        let bytes = decode(&reencoded).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn oversized_images_are_rejected() {
        let huge = PastedImage::new(ImageFormat::Png, vec![0; MAX_IMAGE_BYTES + 1]);
        assert!(payload(&huge).is_err());
    }
}
