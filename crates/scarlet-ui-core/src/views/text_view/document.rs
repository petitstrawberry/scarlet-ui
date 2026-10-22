//! Persistent text document model for the multi-line text view.

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use core::ops::Range;

use once_cell::race::OnceBox;
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation};

#[path = "document_tree.rs"]
mod tree;

/// Persistent UTF-8 document with indexed, shared text chunks.
///
/// Local edits copy only the affected tree paths and adjoining chunks. Cloning
/// for state or undo retains an immutable revision. APIs requiring a contiguous
/// string materialize it lazily once per revision; edits and indexed queries do
/// not require flattening the document.
#[derive(Clone, Debug, Default)]
pub struct TextDocument {
    snapshot: Arc<DocumentSnapshot>,
}

#[derive(Debug, Default)]
struct DocumentSnapshot {
    root: tree::Root,
    // Race-safe, alloc-only initialization also supports legacy Scarlet. Two
    // concurrent first readers may compute equivalent text; one result is kept.
    flat: OnceBox<String>,
}

impl TextDocument {
    /// Create an empty document. Its logical line count is zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a UTF-8 document. Newlines are retained exactly, including CRLF.
    pub fn from_str(text: &str) -> Self {
        Self::from_root(tree::from_str(text))
    }

    fn from_root(root: tree::Root) -> Self {
        Self {
            snapshot: Arc::new(DocumentSnapshot {
                root,
                flat: OnceBox::new(),
            }),
        }
    }

    pub(crate) fn same_revision(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.snapshot, &other.snapshot)
    }

    fn chunk_at(&self, byte: usize) -> (&str, usize) {
        self.snapshot
            .root
            .as_ref()
            .map(|root| root.chunk_at(byte))
            .unwrap_or(("", 0))
    }

    fn clamp_byte(&self, byte: usize) -> usize {
        let byte = byte.min(self.len());
        let (chunk, start) = self.chunk_at(byte);
        start + clamp_byte_to_char_boundary(chunk, byte - start)
    }

    fn provide_grapheme_context(&self, cursor: &mut GraphemeCursor, end: usize) {
        let (chunk, start) = self.chunk_at(end - 1);
        cursor.provide_context(&chunk[..end - start], start);
    }

    fn is_grapheme_boundary(&self, byte: usize) -> bool {
        let (chunk, start) = self.chunk_at(byte);
        let mut cursor = GraphemeCursor::new(byte, self.len(), true);
        loop {
            match cursor.is_boundary(chunk, start) {
                Ok(boundary) => return boundary,
                Err(GraphemeIncomplete::PreContext(end)) => {
                    self.provide_grapheme_context(&mut cursor, end)
                }
                Err(other) => unreachable!("valid chunk for boundary query: {other:?}"),
            }
        }
    }

    fn adjacent_grapheme_boundary(&self, byte: usize, forward: bool) -> usize {
        let mut cursor = GraphemeCursor::new(byte, self.len(), true);
        let (mut chunk, mut start) = self.chunk_at(if forward {
            byte
        } else {
            byte.saturating_sub(1)
        });
        loop {
            let result = if forward {
                cursor.next_boundary(chunk, start)
            } else {
                cursor.prev_boundary(chunk, start)
            };
            match result {
                Ok(Some(boundary)) => return boundary,
                Ok(None) => return if forward { self.len() } else { 0 },
                Err(GraphemeIncomplete::PreContext(end)) => {
                    self.provide_grapheme_context(&mut cursor, end)
                }
                Err(GraphemeIncomplete::NextChunk) => {
                    (chunk, start) = self.chunk_at(start + chunk.len())
                }
                Err(GraphemeIncomplete::PrevChunk) => (chunk, start) = self.chunk_at(start - 1),
                Err(GraphemeIncomplete::InvalidOffset) => {
                    unreachable!("valid chunk for grapheme traversal")
                }
            }
        }
    }

    pub(crate) fn clamp_position(&self, position: TextPosition) -> TextPosition {
        let byte = self.clamp_byte(position.byte);
        if byte == 0 || byte == self.len() || self.is_grapheme_boundary(byte) {
            return TextPosition::new(byte);
        }
        let previous = self.adjacent_grapheme_boundary(byte, false);
        let next = self.adjacent_grapheme_boundary(byte, true);
        TextPosition::new(if next - byte < byte - previous {
            next
        } else {
            previous
        })
    }

    /// Return contiguous text, materializing it once for this immutable revision.
    /// Subsequent calls and document clones borrow the same cached text.
    pub fn as_str(&self) -> Cow<'_, str> {
        let Some(root) = &self.snapshot.root else {
            return Cow::Borrowed("");
        };
        if let Some(text) = root.leaf_text() {
            return Cow::Borrowed(text);
        }
        Cow::Borrowed(
            self.snapshot
                .flat
                .get_or_init(|| {
                    let mut text = String::with_capacity(root.bytes);
                    root.append_range(0..root.bytes, &mut text);
                    Box::new(text)
                })
                .as_str(),
        )
    }

    /// Return the length in UTF-8 bytes.
    pub fn len(&self) -> usize {
        self.snapshot.root.as_ref().map_or(0, |root| root.bytes)
    }

    /// Whether the document contains no bytes.
    pub fn is_empty(&self) -> bool {
        self.snapshot.root.is_none()
    }

    /// Return zero for an empty document, otherwise the LF count plus one.
    /// A trailing LF therefore creates a final empty logical line.
    pub fn line_count(&self) -> usize {
        self.snapshot
            .root
            .as_ref()
            .map_or(0, |root| root.newlines + 1)
    }

    /// Byte range of a logical line, including its line ending when present.
    pub fn line_range(&self, line_index: usize) -> Option<Range<usize>> {
        let root = self.snapshot.root.as_ref()?;
        if line_index > root.newlines {
            return None;
        }
        let start = if line_index == 0 {
            0
        } else {
            root.newline_byte(line_index - 1) + 1
        };
        let end = if line_index == root.newlines {
            root.bytes
        } else {
            root.newline_byte(line_index) + 1
        };
        Some(start..end)
    }

    /// Zero-based logical line containing a byte; offsets past the end clamp.
    /// Empty documents return zero. An LF byte belongs to its preceding line.
    pub fn line_index_at_byte(&self, byte: usize) -> usize {
        self.snapshot
            .root
            .as_ref()
            .map_or(0, |root| root.newlines_before(byte.min(root.bytes)))
    }

    /// Return one logical line, borrowing an existing contiguous slice when
    /// possible. A line crossing chunks copies that line without flattening the
    /// rest of the document. Once as_str is cached every line can be borrowed.
    pub fn line_text(&self, line_index: usize) -> Option<Cow<'_, str>> {
        let range = self.line_range(line_index)?;
        if let Some(text) = self.snapshot.flat.get() {
            return Some(Cow::Borrowed(&text[range]));
        }
        let (chunk, start) = self.chunk_at(range.start);
        if range.end <= start + chunk.len() {
            return Some(Cow::Borrowed(
                &chunk[range.start - start..range.end - start],
            ));
        }
        let mut text = String::with_capacity(range.len());
        self.snapshot.root.as_ref()?.append_range(range, &mut text);
        Some(Cow::Owned(text))
    }

    /// Insert at a byte offset, clamped down to a Unicode scalar boundary.
    pub fn insert(&self, at: TextPosition, text: &str) -> (Self, EditDelta) {
        self.replace(at.byte..at.byte, text)
    }

    /// Delete a range, clamping each endpoint down to a scalar boundary.
    pub fn delete(&self, range: Range<usize>) -> (Self, EditDelta) {
        self.replace(range, "")
    }

    fn range_equals(&self, range: Range<usize>, text: &str) -> bool {
        if range.len() != text.len() {
            return false;
        }
        let mut byte = range.start;
        while byte < range.end {
            let (chunk, start) = self.chunk_at(byte);
            let end = range.end.min(start + chunk.len());
            if chunk.as_bytes()[byte - start..end - start]
                != text.as_bytes()[byte - range.start..end - range.start]
            {
                return false;
            }
            byte = end;
        }
        true
    }

    /// Replace bytes without normalizing text or forcing grapheme-aligned edits.
    /// Endpoints clamp down to scalar boundaries; reversed ranges become empty
    /// at their clamped start. A byte-identical replacement retains its revision.
    pub fn replace(&self, range: Range<usize>, text: &str) -> (Self, EditDelta) {
        let start = self.clamp_byte(range.start);
        let end = self.clamp_byte(range.end).max(start);
        let range = start..end;
        let delta = EditDelta {
            replaced_range: range.clone(),
            inserted: String::from(text),
        };
        if self.range_equals(range.clone(), text) {
            return (self.clone(), delta);
        }
        let (before, rest) = tree::split(self.snapshot.root.clone(), start);
        let (_, after) = tree::split(rest, end - start);
        let root = tree::concat(tree::concat(before, tree::from_str(text)), after);
        (Self::from_root(root), delta)
    }

    /// Convert a Unicode scalar index to a byte offset, clamping past the end.
    pub fn char_to_byte(&self, char_index: usize) -> usize {
        self.snapshot
            .root
            .as_ref()
            .map_or(0, |root| root.char_to_byte(char_index))
    }

    /// Convert a byte to a Unicode scalar index; invalid boundaries clamp down.
    pub fn byte_to_char(&self, byte_offset: usize) -> usize {
        let byte = self.clamp_byte(byte_offset);
        self.snapshot
            .root
            .as_ref()
            .map_or(0, |root| root.byte_to_char(byte))
    }
}

#[cfg(test)]
#[path = "document_tests.rs"]
mod tests;

/// A UTF-8 byte position in a text document.
///
/// Positions should be clamped to extended grapheme cluster boundaries before
/// use in cursor movement. Use [`TextPosition::clamp_to_grapheme`] when
/// constructing from user input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPosition {
    /// Byte offset from the start of the document.
    pub byte: usize,
}

impl TextPosition {
    /// Create a byte position.
    ///
    /// # Arguments
    ///
    /// * `byte` - UTF-8 byte offset from the start of a document.
    ///
    /// # Returns
    ///
    /// A text position storing `byte` unchanged.
    pub fn new(byte: usize) -> Self {
        Self { byte }
    }

    /// Clamp this position to the nearest grapheme cluster boundary.
    ///
    /// # Arguments
    ///
    /// * `text` - Text that defines valid grapheme boundaries.
    ///
    /// # Returns
    ///
    /// A position at the closest valid extended grapheme boundary.
    pub fn clamp_to_grapheme(self, text: &str) -> Self {
        let byte = clamp_byte_to_char_boundary(text, self.byte);
        if text.is_empty() || byte == text.len() {
            return Self { byte };
        }

        let mut previous = 0usize;
        for (boundary, _) in text.grapheme_indices(true) {
            if boundary == byte {
                return Self { byte };
            }
            if boundary > byte {
                let prev_distance = byte.saturating_sub(previous);
                let next_distance = boundary.saturating_sub(byte);
                return if next_distance < prev_distance {
                    Self { byte: boundary }
                } else {
                    Self { byte: previous }
                };
            }
            previous = boundary;
        }

        let prev_distance = byte.saturating_sub(previous);
        let next_distance = text.len().saturating_sub(byte);
        if next_distance < prev_distance {
            Self { byte: text.len() }
        } else {
            Self { byte: previous }
        }
    }
}

/// Describes a single edit operation for undo/redo support.
///
/// Sufficient to invert any edit: store the deleted text, then re-insert it at
/// the same range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditDelta {
    /// Byte range in the pre-edit document that was replaced.
    pub replaced_range: Range<usize>,
    /// Text inserted in place of `replaced_range`.
    pub inserted: String,
}

impl EditDelta {
    /// Return whether this edit inserted text without deleting anything.
    ///
    /// # Returns
    ///
    /// `true` when the replaced range is empty and inserted text is non-empty.
    pub fn is_insertion(&self) -> bool {
        self.replaced_range.is_empty() && !self.inserted.is_empty()
    }

    /// Return whether this edit deleted text without inserting anything.
    ///
    /// # Returns
    ///
    /// `true` when the replaced range is non-empty and inserted text is empty.
    pub fn is_deletion(&self) -> bool {
        !self.replaced_range.is_empty() && self.inserted.is_empty()
    }

    /// Return whether this edit replaced text with different text.
    ///
    /// # Returns
    ///
    /// `true` when both the replaced range and inserted text are non-empty.
    pub fn is_replacement(&self) -> bool {
        !self.replaced_range.is_empty() && !self.inserted.is_empty()
    }

    /// Return the text deleted from the original document.
    ///
    /// # Arguments
    ///
    /// * `original` - The pre-edit text that this delta was produced from.
    ///
    /// # Returns
    ///
    /// The original text covered by `replaced_range`.
    pub fn deleted_text<'a>(&'a self, original: &'a str) -> &'a str {
        let start = clamp_byte_to_char_boundary(original, self.replaced_range.start);
        let mut end = clamp_byte_to_char_boundary(original, self.replaced_range.end);
        if end < start {
            end = start;
        }
        &original[start..end]
    }
}

fn clamp_byte_to_char_boundary(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}
