//! Server-Sent Events framing: one event per `data:` frame, as AG-UI's
//! `HttpAgent` sends and expects.

use crate::{Event, Result};

/// The content type of an AG-UI event stream.
pub const CONTENT_TYPE: &str = "text/event-stream";

/// Encodes one event as an SSE frame: `data: <json>\n\n`.
pub fn encode(event: &Event) -> String {
    let mut frame = String::from("data: ");
    frame.push_str(&event.to_json());
    frame.push_str("\n\n");
    frame
}

/// An incremental SSE parser: feed bytes as they arrive, take events as
/// frames complete. Handles `\n`, `\r\n` and `\r` line ends, multi-line
/// `data:` fields (joined with `\n`), comments, and ignores `event:`,
/// `id:` and `retry:` fields.
#[derive(Debug, Default)]
pub struct Decoder {
    buffer: Vec<u8>,
}

impl Decoder {
    /// An empty decoder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends bytes and returns every event whose frame is now complete.
    /// A frame that is not AG-UI JSON is an error; the decoder stays
    /// usable for the frames after it.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Event>> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = frame_end(&self.buffer) {
            let frame: Vec<u8> = self.buffer.drain(..end.frame_len).collect();
            let frame = String::from_utf8_lossy(&frame[..end.body_len]);
            if let Some(data) = data_of(&frame) {
                events.push(Event::from_json(&data)?);
            }
        }
        Ok(events)
    }

    /// Flushes a final frame that ended without a blank line (a stream
    /// closed mid-frame). Returns nothing if the buffer is empty or holds
    /// no `data:` lines.
    pub fn finish(&mut self) -> Result<Option<Event>> {
        let rest = String::from_utf8_lossy(&self.buffer).into_owned();
        self.buffer.clear();
        match data_of(&rest) {
            Some(data) => Ok(Some(Event::from_json(&data)?)),
            None => Ok(None),
        }
    }
}

struct FrameEnd {
    /// Bytes of the frame body, before its blank line.
    body_len: usize,
    /// Bytes to drain, blank line included.
    frame_len: usize,
}

/// The length of the line terminator at `i`, if there is one: `\r\n`
/// counts as one terminator of two bytes.
fn terminator_at(buf: &[u8], i: usize) -> Option<usize> {
    match buf.get(i)? {
        b'\n' => Some(1),
        b'\r' => Some(if buf.get(i + 1) == Some(&b'\n') { 2 } else { 1 }),
        _ => None,
    }
}

/// Finds the first blank line: two consecutive line terminators.
fn frame_end(buf: &[u8]) -> Option<FrameEnd> {
    let mut i = 0;
    while i < buf.len() {
        let Some(first) = terminator_at(buf, i) else {
            i += 1;
            continue;
        };
        if let Some(second) = terminator_at(buf, i + first) {
            return Some(FrameEnd {
                body_len: i,
                frame_len: i + first + second,
            });
        }
        i += first;
    }
    None
}

/// The joined `data:` payload of a frame, if it has one.
fn data_of(frame: &str) -> Option<String> {
    let mut data: Option<String> = None;
    for line in frame.split(['\n', '\r']) {
        let Some(rest) = line.strip_prefix("data:") else {
            continue;
        };
        let rest = rest.strip_prefix(' ').unwrap_or(rest);
        let data = data.get_or_insert_with(String::new);
        if !data.is_empty() {
            data.push('\n');
        }
        data.push_str(rest);
    }
    data
}

/// A sanity check that `text` is a complete SSE frame, for tests and
/// debugging.
pub fn is_frame(text: &str) -> bool {
    text.starts_with("data: ") && text.ends_with("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EventKind;

    fn step(name: &str) -> Event {
        EventKind::StepStarted {
            step_name: name.into(),
        }
        .into()
    }

    #[test]
    fn encode_is_one_data_frame() {
        let frame = encode(&step("a"));
        assert!(is_frame(&frame));
        assert_eq!(
            frame,
            "data: {\"stepName\":\"a\",\"type\":\"STEP_STARTED\"}\n\n"
        );
    }

    #[test]
    fn decoder_handles_split_frames_comments_and_line_endings() {
        let mut decoder = Decoder::new();
        let a = encode(&step("a"));
        let (head, tail) = a.split_at(10);
        assert!(decoder.feed(head.as_bytes()).unwrap().is_empty());
        assert_eq!(decoder.feed(tail.as_bytes()).unwrap(), vec![step("a")]);

        let mixed = ": keep-alive\r\nid: 7\r\nevent: message\r\ndata: {\"type\":\"STEP_STARTED\",\r\ndata: \"stepName\":\"b\"}\r\n\r\n";
        assert_eq!(decoder.feed(mixed.as_bytes()).unwrap(), vec![step("b")]);

        let cr_only = "data: {\"type\":\"STEP_STARTED\",\"stepName\":\"c\"}\r\r";
        assert_eq!(decoder.feed(cr_only.as_bytes()).unwrap(), vec![step("c")]);

        let two = format!("{}{}", encode(&step("d")), encode(&step("e")));
        assert_eq!(
            decoder.feed(two.as_bytes()).unwrap(),
            vec![step("d"), step("e")]
        );
    }

    #[test]
    fn finish_flushes_an_unterminated_frame() {
        let mut decoder = Decoder::new();
        decoder
            .feed(b"data: {\"type\":\"STEP_FINISHED\",\"stepName\":\"z\"}")
            .unwrap();
        assert_eq!(
            decoder.finish().unwrap(),
            Some(
                EventKind::StepFinished {
                    step_name: "z".into()
                }
                .into()
            )
        );
        assert_eq!(decoder.finish().unwrap(), None);
    }

    #[test]
    fn a_bad_frame_errors_without_poisoning_the_decoder() {
        let mut decoder = Decoder::new();
        assert!(decoder.feed(b"data: nope\n\n").is_err());
        assert_eq!(
            decoder.feed(encode(&step("ok")).as_bytes()).unwrap(),
            vec![step("ok")]
        );
    }
}
