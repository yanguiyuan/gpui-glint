/// Embedded so direct `cargo run` launches have the same Dock icon as a bundle.
#[cfg(target_os = "macos")]
pub fn install() {
    use objc2::{
        msg_send,
        runtime::{AnyClass, AnyObject},
    };

    let bytes = include_bytes!("assets/glint-api-icon.png");
    // Application::run invokes this on the AppKit main thread. NSData copies
    // the bytes; NSApplication retains the decoded image after assignment.
    unsafe {
        let data: *mut AnyObject = msg_send![AnyClass::get(c"NSData").unwrap(),
            dataWithBytes: bytes.as_ptr().cast::<std::ffi::c_void>(), length: bytes.len()];
        let image: *mut AnyObject = msg_send![AnyClass::get(c"NSImage").unwrap(), alloc];
        let image: *mut AnyObject = msg_send![image, initWithData: data];
        if !image.is_null() {
            let app: *mut AnyObject =
                msg_send![AnyClass::get(c"NSApplication").unwrap(), sharedApplication];
            let _: () = msg_send![app, setApplicationIconImage: image];
            let _: () = msg_send![image, release];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn install() {}
