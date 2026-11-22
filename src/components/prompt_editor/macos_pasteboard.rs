use super::{MAX_PASTE_IMAGE_BYTES, PastedImage};
use gpui::{ImageFormat, Window};
use objc2::{MainThreadMarker, rc::autoreleasepool};
use objc2_app_kit::NSPasteboard;
use objc2_foundation::NSString;

const IMAGE_TYPES: &[(ImageFormat, &str)] = &[
    (ImageFormat::Png, "public.png"),
    (ImageFormat::Jpeg, "public.jpeg"),
    (ImageFormat::Webp, "org.webmproject.webp"),
    (ImageFormat::Gif, "com.compuserve.gif"),
    (ImageFormat::Svg, "public.svg-image"),
    (ImageFormat::Bmp, "com.microsoft.bmp"),
    (ImageFormat::Tiff, "public.tiff"),
    (ImageFormat::Ico, "com.microsoft.ico"),
    (ImageFormat::Pnm, "public.pbm"),
];

pub(super) struct ImageSource {
    change_count: isize,
    formats: Vec<(ImageFormat, &'static str)>,
}

pub(super) fn image_source(window: &Window) -> Option<ImageSource> {
    // Only an active AppKit window handling a user action uses the system pasteboard.
    if MainThreadMarker::new().is_none() || !window.is_window_active() {
        return None;
    }
    autoreleasepool(|_| ImageSource::probe(&NSPasteboard::generalPasteboard()))
}

impl ImageSource {
    fn probe(board: &NSPasteboard) -> Option<Self> {
        let change_count = board.changeCount();
        let types = board.types()?;
        // Keep GPUI's file precedence. Only inspect type metadata here.
        // Do not bail out on plain-text metadata: browsers and chat apps often
        // attach text/URL alongside image data, and PromptEditor prefers images.
        if ["NSFilenamesPboardType", "public.file-url"]
            .iter()
            .any(|kind| types.containsObject(&NSString::from_str(kind)))
        {
            return None;
        }
        let formats = IMAGE_TYPES
            .iter()
            .copied()
            .filter(|(_, kind)| types.containsObject(&NSString::from_str(kind)))
            .collect::<Vec<_>>();
        (!formats.is_empty()).then_some(Self {
            change_count,
            formats,
        })
    }

    pub(super) fn read(self) -> Result<Vec<PastedImage>, String> {
        // Acquire native objects on the worker. Only the requested types and
        // pasteboard revision cross threads.
        autoreleasepool(|_| self.read_from(&NSPasteboard::generalPasteboard()))
    }

    fn read_from(self, board: &NSPasteboard) -> Result<Vec<PastedImage>, String> {
        for (format, kind) in self.formats {
            if board.changeCount() != self.change_count {
                return Err("剪贴板内容已变化，请重新粘贴".into());
            }
            let data = board.dataForType(&NSString::from_str(kind));
            if board.changeCount() != self.change_count {
                return Err("剪贴板内容已变化，请重新粘贴".into());
            }
            if let Some(data) = data {
                if data.len() > MAX_PASTE_IMAGE_BYTES {
                    return Err("图片原始数据超过 20MB".into());
                }
                return Ok(vec![PastedImage::new(format, data.to_vec())]);
            }
        }
        Err("无法读取剪贴板图片，请重新粘贴".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_foundation::NSData;

    #[test]
    fn image_reads_on_a_worker_and_does_not_paste_a_new_clipboard_value() {
        autoreleasepool(|_| {
            let board = NSPasteboard::pasteboardWithUniqueName();
            let name = board.name().to_string();
            let png_type = NSString::from_str("public.png");
            let png = NSData::with_bytes(&[1, 2, 3]);
            board.clearContents();
            assert!(board.setData_forType(Some(&png), &png_type));
            let source = ImageSource::probe(&board).unwrap();
            let images = std::thread::spawn(move || {
                autoreleasepool(|_| {
                    source.read_from(&NSPasteboard::pasteboardWithName(&NSString::from_str(
                        &name,
                    )))
                })
            })
            .join()
            .unwrap()
            .unwrap();
            assert_eq!(images[0].format(), ImageFormat::Png);
            assert_eq!(images[0].bytes(), &[1, 2, 3]);

            let old = ImageSource::probe(&board).unwrap();
            board.clearContents();
            assert!(board.setString_forType(
                &NSString::from_str("text"),
                &NSString::from_str("public.utf8-plain-text"),
            ));
            assert!(ImageSource::probe(&board).is_none());
            assert!(old.read_from(&board).is_err());
        });
    }
}
