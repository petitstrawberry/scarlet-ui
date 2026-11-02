//! Browser-independent UTF conversion and DOM edit reconciliation.
use scarlet_ui_core::event::Event;

pub(crate) fn utf16_to_byte(text: &str, offset: u32) -> u32 {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units + ch.len_utf16() as u32 > offset {
            return byte as u32;
        }
        units += ch.len_utf16() as u32;
    }
    text.len() as u32
}
pub(crate) fn byte_to_utf16(text: &str, offset: u32) -> u32 {
    text.char_indices()
        .take_while(|(byte, _)| *byte < offset as usize)
        .map(|(_, ch)| ch.len_utf16() as u32)
        .sum()
}

/// A single replacement, bounded by Unicode scalar boundaries. Unlike using
/// InputEvent.data, this also handles paste, autocorrection and mobile deletion.
pub(crate) fn replacement<'a>(old: &str, new: &'a str) -> (usize, usize, &'a str) {
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    (prefix, old.len() - suffix, &new[prefix..new.len() - suffix])
}
pub(crate) fn selection(cursor_byte: u32, anchor_byte: u32) -> Event {
    Event::TextInputSelection {
        cursor_byte,
        anchor_byte,
    }
}
pub(crate) fn preedit(text: String) -> Event {
    Event::TextInputPreedit {
        context_id: 0,
        serial: 0,
        cursor_byte: text.len() as u32,
        anchor_byte: text.len() as u32,
        text,
        spans: Vec::new(),
    }
}
pub(crate) fn edit_events(old: &str, new: &str, cursor: u32, anchor: u32) -> Vec<Event> {
    let mut events = Vec::new();
    if old != new {
        let (start, end, text) = replacement(old, new);
        if text.is_empty() {
            events.push(selection(end as u32, end as u32));
            events.push(Event::TextInputDeleteSurroundingText {
                context_id: 0,
                serial: 0,
                before_bytes: (end - start) as u32,
                after_bytes: 0,
            });
        } else {
            // Replace atomically: a temporary empty value could invoke an
            // on_empty callback or blur the editor before the insertion.
            events.push(selection(end as u32, start as u32));
            events.push(Event::TextInputCommit {
                context_id: 0,
                serial: 0,
                text: text.into(),
            });
        }
    }
    events.push(selection(cursor, anchor));
    events
}

/// Match core's circular touch slop; a diagonal scroll must not reopen a keyboard.
pub(crate) fn tap_moved(start_x: i32, start_y: i32, x: i32, y: i32) -> bool {
    let dx = i64::from(x) - i64::from(start_x);
    let dy = i64::from(y) - i64::from(start_y);
    dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy)) >= 8 * 8
}

/// Prefer the original selection for composition: repeated characters can make
/// a longest-prefix diff place the preedit at the wrong visual caret.
pub(crate) fn composition_replacement<'a>(
    old: &str,
    new: &'a str,
    cursor: u32,
    anchor: u32,
) -> (usize, usize, &'a str) {
    let start = cursor.min(anchor) as usize;
    let end = cursor.max(anchor) as usize;
    if end <= old.len() && old.is_char_boundary(start) && old.is_char_boundary(end) {
        let suffix = &old[end..];
        if new.len() >= start + suffix.len()
            && new.starts_with(&old[..start])
            && new.ends_with(suffix)
        {
            return (start, end, &new[start..new.len() - suffix.len()]);
        }
    }
    replacement(old, new)
}
