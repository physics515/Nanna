//! Server-sent-event framing shared by the SSE-based listeners.

/// Split the first complete SSE event (terminated by a blank line) off the
/// front of `buffer` and decode it, without the terminator. `None` until one
/// is complete.
///
/// The listeners used to decode each network chunk on its own with
/// `from_utf8_lossy`, so an incoming message whose emoji or accented letter a
/// chunk boundary split reached the agent as `��`. Framing the raw bytes first
/// and decoding one complete event at a time cannot split a character: the
/// terminator is ASCII, and UTF-8 never uses ASCII bytes inside a multibyte
/// sequence. An event that is itself invalid UTF-8 is decoded lossily, as
/// before — a malformed sender should not stall the stream.
pub fn take_event(buffer: &mut Vec<u8>) -> Option<String> {
    const TERMINATOR: &[u8] = b"\n\n";
    let at = buffer
        .windows(TERMINATOR.len())
        .position(|window| window == TERMINATOR)?;
    let mut event: Vec<u8> = buffer.drain(..at + TERMINATOR.len()).collect();
    event.truncate(at);
    debug_assert!(
        !event.ends_with(TERMINATOR),
        "the terminator is not part of the event"
    );
    Some(
        String::from_utf8(event)
            .unwrap_or_else(|invalid| String::from_utf8_lossy(invalid.as_bytes()).into_owned()),
    )
}

#[cfg(test)]
mod tests {
    use super::take_event;

    /// Every split point of an event carrying multibyte text yields the
    /// same, uncorrupted event.
    #[test]
    fn an_event_split_mid_character_arrives_uncorrupted() {
        let wire = "data: {\"message\":\"Hallo 👋 Grüße 月\"}\n\n".as_bytes();
        for split in 0..wire.len() {
            let mut buffer = wire[..split].to_vec();
            let early = take_event(&mut buffer);
            buffer.extend_from_slice(&wire[split..]);
            let event = early
                .or_else(|| take_event(&mut buffer))
                .expect("one event");
            assert_eq!(
                event, "data: {\"message\":\"Hallo 👋 Grüße 月\"}",
                "split at byte {split}"
            );
            assert!(buffer.is_empty(), "nothing left over");
        }
    }

    #[test]
    fn events_are_taken_one_at_a_time() {
        let mut buffer = b"a\n\nb\n\nc".to_vec();
        assert_eq!(take_event(&mut buffer).as_deref(), Some("a"));
        assert_eq!(take_event(&mut buffer).as_deref(), Some("b"));
        assert_eq!(take_event(&mut buffer), None, "c is incomplete");
        assert_eq!(buffer, b"c");
    }
}
