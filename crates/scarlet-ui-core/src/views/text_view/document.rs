//! Text document model for the multi-line text view.

use alloc::borrow::Cow;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

/// Persistent text document backed by a piece table.
///
/// Provides efficient edit operations by keeping the original text immutable,
/// appending inserted text to a separate buffer, and describing the visible
/// document as an ordered list of pieces. The internal representation is hidden
/// behind this abstraction so the implementation can change without affecting
/// the public API.
#[derive(Clone, Debug, Default)]
pub struct TextDocument {
    table: PieceTable,
}

impl TextDocument {
    /// Create an empty text document.
    ///
    /// # Returns
    ///
    /// A document containing no text.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a text document from a string slice.
    ///
    /// # Arguments
    ///
    /// * `text` - Initial UTF-8 text for the document.
    ///
    /// # Returns
    ///
    /// A document containing `text`.
    pub fn from_str(text: &str) -> Self {
        Self {
            table: PieceTable::from_str(text),
        }
    }

    /// Return the full document text.
    ///
    /// # Returns
    ///
    /// The document text as a borrowed or owned string.
    pub fn as_str(&self) -> Cow<'_, str> {
        self.table.as_str()
    }

    /// Return the document length in UTF-8 bytes.
    ///
    /// # Returns
    ///
    /// The number of bytes in the document.
    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// Return whether the document is empty.
    ///
    /// # Returns
    ///
    /// `true` when the document contains no bytes.
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Return the number of logical lines in the document.
    ///
    /// # Returns
    ///
    /// `0` for an empty document, otherwise the newline count plus one.
    pub fn line_count(&self) -> usize {
        self.table.line_count()
    }

    /// Return the byte range for a line.
    ///
    /// # Arguments
    ///
    /// * `line_index` - Zero-based line index.
    ///
    /// # Returns
    ///
    /// The byte range occupied by the line, or `None` when out of bounds.
    pub fn line_range(&self, line_index: usize) -> Option<Range<usize>> {
        self.table.line_range(line_index)
    }

    /// Return the text for a line.
    ///
    /// # Arguments
    ///
    /// * `line_index` - Zero-based line index.
    ///
    /// # Returns
    ///
    /// The line text, or `None` when out of bounds.
    pub fn line_text(&self, line_index: usize) -> Option<Cow<'_, str>> {
        self.table.line_text(line_index)
    }

    /// Insert text at a byte position.
    ///
    /// # Arguments
    ///
    /// * `at` - Target byte position. Invalid UTF-8 boundaries are clamped.
    /// * `text` - Text to insert.
    ///
    /// # Returns
    ///
    /// The edited document and the edit delta describing the insertion.
    pub fn insert(&self, at: TextPosition, text: &str) -> (Self, EditDelta) {
        let (table, delta) = self.table.insert(at.byte, text);
        (Self { table }, delta)
    }

    /// Delete a byte range.
    ///
    /// # Arguments
    ///
    /// * `range` - Byte range to delete. Invalid UTF-8 boundaries are clamped.
    ///
    /// # Returns
    ///
    /// The edited document and the edit delta describing the deletion.
    pub fn delete(&self, range: Range<usize>) -> (Self, EditDelta) {
        let (table, delta) = self.table.delete(range);
        (Self { table }, delta)
    }

    /// Replace a byte range with new text.
    ///
    /// # Arguments
    ///
    /// * `range` - Byte range to replace. Invalid UTF-8 boundaries are clamped.
    /// * `text` - Replacement text.
    ///
    /// # Returns
    ///
    /// The edited document and the edit delta describing the replacement.
    pub fn replace(&self, range: Range<usize>, text: &str) -> (Self, EditDelta) {
        let (table, delta) = self.table.replace(range, text);
        (Self { table }, delta)
    }

    /// Convert a character index to a byte offset.
    ///
    /// # Arguments
    ///
    /// * `char_index` - Character index to convert. Values past the end clamp.
    ///
    /// # Returns
    ///
    /// The corresponding UTF-8 byte offset.
    pub fn char_to_byte(&self, char_index: usize) -> usize {
        let text = self.as_str();
        text.char_indices()
            .map(|(byte, _)| byte)
            .nth(char_index)
            .unwrap_or(text.len())
    }

    /// Convert a byte offset to a character index.
    ///
    /// # Arguments
    ///
    /// * `byte_offset` - Byte offset to convert. Invalid boundaries clamp down.
    ///
    /// # Returns
    ///
    /// The corresponding character index.
    pub fn byte_to_char(&self, byte_offset: usize) -> usize {
        let text = self.as_str();
        let byte = clamp_byte_to_char_boundary(&text, byte_offset);
        text[..byte].chars().count()
    }
}

#[derive(Clone, Debug, Default)]
struct PieceTable {
    original: String,
    added: String,
    pieces: Vec<Piece>,
    len: usize,
}

impl PieceTable {
    fn from_str(text: &str) -> Self {
        let mut pieces = Vec::new();
        if !text.is_empty() {
            pieces.push(Piece {
                source: PieceSource::Original,
                start: 0,
                length: text.len(),
            });
        }
        Self {
            original: String::from(text),
            added: String::new(),
            pieces,
            len: text.len(),
        }
    }

    fn as_str(&self) -> Cow<'_, str> {
        if self.pieces.is_empty() {
            return Cow::Borrowed("");
        }
        if self.added.is_empty()
            && self.pieces.len() == 1
            && self.pieces[0].source == PieceSource::Original
            && self.pieces[0].start == 0
            && self.pieces[0].length == self.original.len()
        {
            return Cow::Borrowed(&self.original);
        }

        let mut text = String::with_capacity(self.len);
        for piece in &self.pieces {
            text.push_str(self.piece_text(*piece));
        }
        Cow::Owned(text)
    }

    fn len(&self) -> usize {
        self.len
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn line_count(&self) -> usize {
        if self.is_empty() {
            0
        } else {
            1 + self
                .pieces
                .iter()
                .map(|piece| {
                    self.piece_text(*piece)
                        .bytes()
                        .filter(|byte| *byte == b'\n')
                        .count()
                })
                .sum::<usize>()
        }
    }

    fn line_range(&self, line_index: usize) -> Option<Range<usize>> {
        if line_index >= self.line_count() {
            return None;
        }

        let mut current_line = 0usize;
        let mut line_start = 0usize;
        let mut byte_offset = 0usize;
        for piece in &self.pieces {
            for byte in self.piece_text(*piece).bytes() {
                byte_offset += 1;
                if byte == b'\n' {
                    if current_line == line_index {
                        return Some(line_start..byte_offset);
                    }
                    current_line += 1;
                    line_start = byte_offset;
                }
            }
        }

        Some(line_start..self.len)
    }

    fn line_text(&self, line_index: usize) -> Option<Cow<'_, str>> {
        let range = self.line_range(line_index)?;
        let text = self.as_str();
        Some(Cow::Owned(text[range].to_string()))
    }

    fn insert(&self, byte_offset: usize, text: &str) -> (Self, EditDelta) {
        let offset = self.clamp_byte(byte_offset);
        let mut table = self.clone();
        if !text.is_empty() {
            let added_start = table.added.len();
            table.added.push_str(text);
            let insertion = Piece {
                source: PieceSource::Added,
                start: added_start,
                length: text.len(),
            };
            table.insert_piece(offset, insertion);
            table.len += text.len();
        }
        table.prune_empty_pieces();
        (
            table,
            EditDelta {
                replaced_range: offset..offset,
                inserted: String::from(text),
            },
        )
    }

    fn delete(&self, range: Range<usize>) -> (Self, EditDelta) {
        let range = self.clamp_range(range);
        let mut table = self.clone();
        if !range.is_empty() {
            table.delete_range(range.clone());
            table.len -= range.end - range.start;
        }
        table.prune_empty_pieces();
        (
            table,
            EditDelta {
                replaced_range: range,
                inserted: String::new(),
            },
        )
    }

    fn replace(&self, range: Range<usize>, text: &str) -> (Self, EditDelta) {
        let range = self.clamp_range(range);
        let (deleted, _) = self.delete(range.clone());
        let (inserted, _) = deleted.insert(range.start, text);
        (
            inserted,
            EditDelta {
                replaced_range: range,
                inserted: String::from(text),
            },
        )
    }

    fn insert_piece(&mut self, byte_offset: usize, insertion: Piece) {
        if self.pieces.is_empty() {
            self.pieces.push(insertion);
            return;
        }

        let (index, inner_offset) = self.find_piece(byte_offset);
        if index == self.pieces.len() {
            self.pieces.push(insertion);
            return;
        }

        if inner_offset == 0 {
            self.pieces.insert(index, insertion);
            return;
        }

        let piece = self.pieces[index];
        if inner_offset >= piece.length {
            self.pieces.insert(index + 1, insertion);
            return;
        }

        let left = Piece {
            length: inner_offset,
            ..piece
        };
        let right = Piece {
            start: piece.start + inner_offset,
            length: piece.length - inner_offset,
            ..piece
        };
        self.pieces.splice(index..=index, [left, insertion, right]);
    }

    fn delete_range(&mut self, range: Range<usize>) {
        let mut next = Vec::with_capacity(self.pieces.len());
        let mut cursor = 0usize;
        for piece in &self.pieces {
            let piece_start = cursor;
            let piece_end = cursor + piece.length;
            if piece_end <= range.start || piece_start >= range.end {
                next.push(*piece);
            } else {
                if range.start > piece_start {
                    next.push(Piece {
                        source: piece.source,
                        start: piece.start,
                        length: range.start - piece_start,
                    });
                }
                if range.end < piece_end {
                    let right_offset = range.end - piece_start;
                    next.push(Piece {
                        source: piece.source,
                        start: piece.start + right_offset,
                        length: piece_end - range.end,
                    });
                }
            }
            cursor = piece_end;
        }
        self.pieces = next;
    }

    fn find_piece(&self, byte_offset: usize) -> (usize, usize) {
        let mut cursor = 0usize;
        for (index, piece) in self.pieces.iter().enumerate() {
            let end = cursor + piece.length;
            if byte_offset < end {
                return (index, byte_offset - cursor);
            }
            if byte_offset == end {
                return (index, piece.length);
            }
            cursor = end;
        }
        (self.pieces.len(), 0)
    }

    fn clamp_range(&self, range: Range<usize>) -> Range<usize> {
        let start = self.clamp_byte(range.start);
        let mut end = self.clamp_byte(range.end);
        if end < start {
            end = start;
        }
        start..end
    }

    fn clamp_byte(&self, byte: usize) -> usize {
        let text = self.as_str();
        clamp_byte_to_char_boundary(&text, byte)
    }

    fn piece_text(&self, piece: Piece) -> &str {
        let source = match piece.source {
            PieceSource::Original => &self.original,
            PieceSource::Added => &self.added,
        };
        &source[piece.start..piece.start + piece.length]
    }

    fn prune_empty_pieces(&mut self) {
        self.pieces.retain(|piece| piece.length > 0);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PieceSource {
    Original,
    Added,
}

#[derive(Clone, Copy, Debug)]
struct Piece {
    source: PieceSource,
    start: usize,
    length: usize,
}

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
