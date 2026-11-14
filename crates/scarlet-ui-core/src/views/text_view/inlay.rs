//! Virtual text projection: layout sees labels, all public positions stay source bytes.
use super::{TextDocument, TextViewLayout, TextViewScroll, WrapMode};
use crate::{Color, Size, graphics};
use alloc::{string::String, sync::Arc, vec, vec::Vec};

/// Non-editable text inserted before the source character at `byte`.
#[derive(Clone, Debug, PartialEq)]
pub struct TextInlayHint {
    /// UTF-8 boundary in the original document (EOF is allowed).
    pub byte: usize,
    /// Single-line label. Invalid positions and multiline labels are ignored.
    pub label: String,
    /// Foreground color independent of the source's syntax decorations.
    pub color: Color,
}

impl TextViewLayout {
    /// Lay out display-only labels while preserving source-byte coordinates for
    /// caret movement, selections, IME, syntax decorations and pointer hits.
    pub fn compute_with_inlay_hints(
        document: &TextDocument,
        font_size: f32,
        padding: f32,
        wrap_mode: WrapMode,
        size: Size,
        scroll: TextViewScroll,
        show_line_numbers: bool,
        font_stack: Option<&Arc<graphics::FontStack>>,
        hints: &[TextInlayHint],
    ) -> Self {
        if hints.is_empty() {
            return Self::compute_with_font_stack(
                document,
                font_size,
                padding,
                wrap_mode,
                size,
                scroll,
                show_line_numbers,
                font_stack,
            );
        }
        let source = document.as_str();
        let mut ordered: Vec<_> = hints
            .iter()
            .filter(|hint| {
                source.is_char_boundary(hint.byte)
                    && !hint.label.is_empty()
                    && !hint.label.chars().any(char::is_control)
            })
            .collect();
        ordered.sort_by_key(|hint| hint.byte);
        let mut text = String::new();
        let mut to_source = vec![0];
        let mut to_display = vec![0; source.len() + 1];
        let mut spans = Vec::new();
        let mut next = ordered.into_iter().peekable();
        for byte in 0..=source.len() {
            while next.peek().is_some_and(|hint| hint.byte == byte) {
                let hint = next.next().unwrap();
                let start = text.len();
                text.push_str(&hint.label);
                to_source.extend(core::iter::repeat_n(byte, hint.label.len()));
                spans.push((start..text.len(), hint.color));
            }
            to_display[byte] = text.len();
            if let Some(ch) = source.get(byte..).and_then(|tail| tail.chars().next()) {
                text.push(ch);
                for offset in 1..=ch.len_utf8() {
                    to_source.push(byte + offset);
                }
            }
            // Interior UTF-8 bytes were already appended with the preceding scalar.
            // Record their projected offsets for decoration endpoints.
            if !source.is_char_boundary(byte) {
                to_display[byte] = to_display[byte - 1] + 1;
            }
        }
        let projected = TextDocument::from_str(&text);
        let mut layout = Self::compute_with_font_stack(
            &projected,
            font_size,
            padding,
            wrap_mode,
            size,
            scroll,
            show_line_numbers,
            font_stack,
        );
        for hint in hints {
            if hint.byte <= source.len() && source.is_char_boundary(hint.byte) {
                let projected_byte = to_display[hint.byte];
                let row = layout
                    .visual_lines
                    .partition_point(|line| line.text_range.end < projected_byte)
                    .min(layout.visual_lines.len().saturating_sub(1));
                layout.inlay_carets.push((
                    hint.byte,
                    row,
                    layout.visual_lines[row].width_to_byte(projected_byte),
                ));
            }
        }
        layout.inlay_carets.sort_by_key(|entry| entry.0);
        layout.inlay_carets.dedup_by_key(|entry| entry.0);
        for line in &mut layout.visual_lines {
            let projected_range = line.text_range.clone();
            let first = spans.partition_point(|(range, _)| range.end <= projected_range.start);
            for (range, color) in spans[first..]
                .iter()
                .take_while(|(range, _)| range.start < projected_range.end)
            {
                let start = range.start.max(projected_range.start);
                let end = range.end.min(projected_range.end);
                if start < end {
                    line.inlay_spans.push((
                        line.display_byte_for_document_byte(start)
                            ..line.display_byte_for_document_byte(end),
                        *color,
                    ));
                }
            }
            let source_range = to_source[projected_range.start]..to_source[projected_range.end];
            let offsets = (source_range.start..=source_range.end)
                .map(|byte| {
                    line.display_byte_for_document_byte(
                        to_display[byte].clamp(projected_range.start, projected_range.end),
                    )
                })
                .collect();
            for (byte, _) in &mut line.caret_stops {
                *byte = to_source[*byte];
            }
            line.text_range = source_range;
            line.document_to_display_offsets = offsets;
        }
        layout.document = document.clone();
        layout.inlay_hints = hints.to_vec();
        layout
    }
}
