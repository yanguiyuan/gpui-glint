//! Keep a bounded, incrementally decoded view of a UTF-8 event stream.
const DISPLAY_LIMIT: usize = 1024 * 1024;
pub struct Buffer {
    decoder: encoding_rs::Decoder,
    text: String,
    truncated: bool,
    pub bytes: usize,
}
impl Buffer {
    pub fn new() -> Self {
        Self {
            decoder: encoding_rs::UTF_8.new_decoder(),
            text: String::new(),
            truncated: false,
            bytes: 0,
        }
    }
    pub fn push(&mut self, bytes: &[u8], last: bool) {
        self.bytes = self.bytes.saturating_add(bytes.len());
        let mut decoded = String::with_capacity(bytes.len().saturating_mul(3) + 8);
        let (_, read, _) = self.decoder.decode_to_string(bytes, &mut decoded, last);
        debug_assert_eq!(read, bytes.len());
        self.text.push_str(&decoded);
        if self.text.len() > DISPLAY_LIMIT {
            let mut start = self.text.len() - DISPLAY_LIMIT;
            while !self.text.is_char_boundary(start) {
                start += 1;
            }
            self.text.drain(..start);
            self.truncated = true;
        }
    }
    pub fn text(&self) -> String {
        if self.truncated {
            format!(": 早期内容已省略，仅显示最近 1 MiB\n\n{}", self.text)
        } else {
            self.text.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sse_split_utf8_bom_and_event_fields_stay_intact() {
        let text = "\u{feff}: heartbeat\r\nid: 7\r\nevent: message\r\ndata: 你好🌏\r\ndata: second line\r\n\r\n";
        let mut buffer = Buffer::new();
        for byte in text.as_bytes() {
            buffer.push(&[*byte], false);
        }
        buffer.push(&[], true);
        assert_eq!(buffer.text(), text.trim_start_matches('\u{feff}'));
        assert_eq!(buffer.bytes, text.len());
    }
    #[test]
    fn sse_tail_stays_bounded_at_character_boundaries() {
        let mut buffer = Buffer::new();
        let text = "你好".repeat(DISPLAY_LIMIT / 3);
        buffer.push(text.as_bytes(), false);
        assert!(buffer.text.len() <= DISPLAY_LIMIT);
        assert!(buffer.text().contains("早期内容已省略"));
        assert!(!buffer.text().contains('\u{fffd}'));
        assert_eq!(buffer.bytes, text.len());
    }
    #[gpui::test]
    fn sse_response_remains_visible_during_receiving_and_after_cancellation(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::*;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("API", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.show_activity = false;
            host.sending = Some(cx.spawn(async |_, _| std::future::pending::<()>().await));
            host.apply_stream_response(
                0,
                HttpResponse {
                    status: 200,
                    headers: vec![("Content-Type".into(), "text/event-stream".into())],
                    text: "data: first\n\n".into(),
                    bytes: 13,
                    json: None,
                },
                cx,
            );
            host
        });
        window.update(|window, _| window.refresh());
        assert!(window.debug_bounds("response-body-toolbar").is_some());
        host.update(window, |host, cx| {
            assert!(host.sending.is_some());
            assert_eq!(host.response.read(cx).content(), "data: first\n\n");
            let header_id = host.response_header_entries[0][0].entity_id();
            let mut response = host.response_data.clone().unwrap();
            response.text.push_str("data: second\n\n");
            response.bytes = response.text.len();
            host.apply_stream_response(0, response, cx);
            assert_eq!(host.response_header_entries[0][0].entity_id(), header_id);
            assert!(host.response.read(cx).content().contains("data: second"));
            host.response_tab = 1;
            cx.notify();
        });
        window.update(|window, _| window.refresh());
        assert!(window.debug_bounds("response-header-0-0").is_some());
        host.update(window, |host, cx| {
            host.send(cx);
            assert!(host.sending.is_none());
            assert!(
                host.response_data
                    .as_ref()
                    .unwrap()
                    .text
                    .contains("data: second")
            );
            host.persist(cx);
        });
        let saved = storage::Store::open(&path).unwrap().load().unwrap();
        assert!(
            saved.responses[0]
                .as_ref()
                .unwrap()
                .text
                .contains("data: second")
        );
    }
}
