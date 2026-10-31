//! Follow newly laid out chat content only while the reader stays at the end.
use gpui::{ScrollHandle, px};

pub fn follow_if_at_bottom(scroll: &ScrollHandle) {
    // Scroll offsets are negative; max_offset is the positive scrollable extent.
    // Allow only subpixel/layout rounding, rather than snapping nearby history.
    if scroll.max_offset().y + scroll.offset().y <= px(2.) {
        scroll.scroll_to_bottom();
    }
}
