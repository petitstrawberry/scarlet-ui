//! Text document model for the multi-line text view.

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::sync::Arc;
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
    snapshot: Arc<DocumentSnapshot>,
}

/// A revision owns one flattened view and line index. Cloning a document for
/// state, undo history, or layout retains this revision without copying text.
#[derive(Debug, Default)]
struct DocumentSnapshot {
    table: PieceTable,
    text: Arc<str>,
    line_starts: Vec<usize>,
    grapheme_boundaries: Vec<usize>,
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
        Self::from_table(PieceTable::from_str(text))
    }

    fn from_table(table: PieceTable) -> Self {
        let text = if table.added.is_empty() && table.len == table.original.len() {
            Arc::clone(&table.original)
        } else {
            Arc::from(table.as_str().as_ref())
        };
        let mut line_starts = Vec::new();
        if !text.is_empty() {
            line_starts.push(0);
            line_starts.extend(
                text.bytes()
                    .enumerate()
                    .filter_map(|(byte, ch)| (ch == b'\n').then_some(byte + 1)),
            );
        }
        let mut grapheme_boundaries: Vec<_> =
            text.grapheme_indices(true).map(|(byte, _)| byte).collect();
        grapheme_boundaries.push(text.len());
        Self {
            snapshot: Arc::new(DocumentSnapshot {
                table,
                text,
                line_starts,
                grapheme_boundaries,
            }),
        }
    }

    pub(crate) fn same_revision(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.snapshot, &other.snapshot)
    }

    pub(crate) fn clamp_position(&self, position: TextPosition) -> TextPosition {
        let byte = clamp_byte_to_char_boundary(&self.snapshot.text, position.byte);
        let boundaries = &self.snapshot.grapheme_boundaries;
        let index = boundaries.partition_point(|boundary| *boundary < byte);
        let next = boundaries.get(index).copied().unwrap_or(self.len());
        let previous = index
            .checked_sub(1)
            .and_then(|index| boundaries.get(index))
            .copied()
            .unwrap_or(0);
        TextPosition::new(if next - byte < byte - previous {
            next
        } else {
            previous
        })
    }

    /// Return the full document text.
    ///
    /// # Returns
    ///
    /// The document text as a borrowed or owned string.
    pub fn as_str(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.snapshot.text)
    }

    /// Return the document length in UTF-8 bytes.
    ///
    /// # Returns
    ///
    /// The number of bytes in the document.
    pub fn len(&self) -> usize {
        self.snapshot.text.len()
    }

    /// Return whether the document is empty.
    ///
    /// # Returns
    ///
    /// `true` when the document contains no bytes.
    pub fn is_empty(&self) -> bool {
        self.snapshot.text.is_empty()
    }

    /// Return the number of logical lines in the document.
    ///
    /// # Returns
    ///
    /// `0` for an empty document, otherwise the newline count plus one.
    pub fn line_count(&self) -> usize {
        self.snapshot.line_starts.len()
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
        let start = *self.snapshot.line_starts.get(line_index)?;
        let end = self
            .snapshot
            .line_starts
            .get(line_index + 1)
            .copied()
            .unwrap_or(self.len());
        Some(start..end)
    }

    /// Return the zero-based logical line containing a UTF-8 byte offset.
    /// Offsets past the document clamp to its end; an empty document returns 0.
    pub fn line_index_at_byte(&self, byte: usize) -> usize {
        self.snapshot
            .line_starts
            .partition_point(|start| *start <= byte.min(self.len()))
            .saturating_sub(1)
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
        Some(Cow::Borrowed(
            &self.snapshot.text[self.line_range(line_index)?],
        ))
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
        self.replace(at.byte..at.byte, text)
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
        self.replace(range, "")
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
        let start = clamp_byte_to_char_boundary(&self.snapshot.text, range.start);
        let end = clamp_byte_to_char_boundary(&self.snapshot.text, range.end).max(start);
        let range = start..end;
        let delta = EditDelta {
            replaced_range: range.clone(),
            inserted: String::from(text),
        };
        if self.snapshot.text[range.clone()] == *text {
            return (self.clone(), delta);
        }
        let mut table = self.snapshot.table.clone();
        table.delete_range(range.clone());
        table.len -= range.len();
        if !text.is_empty() {
            let added_start = table.added.len();
            Arc::make_mut(&mut table.added).push_str(text);
            table.insert_piece(
                start,
                Piece {
                    source: PieceSource::Added,
                    start: added_start,
                    length: text.len(),
                },
            );
            table.len += text.len();
        }
        table.prune_empty_pieces();
        (Self::from_table(table), delta)
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
    original: Arc<str>,
    added: Arc<String>,
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
            original: Arc::from(text),
            added: Arc::new(String::new()),
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

    fn piece_text(&self, piece: Piece) -> &str {
        let source = match piece.source {
            PieceSource::Original => self.original.as_ref(),
            PieceSource::Added => self.added.as_str(),
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
