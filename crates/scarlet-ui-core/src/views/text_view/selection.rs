//! Selection and configuration value types for the multi-line text view.

use core::ops::Range;

use super::TextPosition;

/// Caret and selection state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextSelection {
    /// Fixed end of the selection.
    pub anchor: TextPosition,
    /// Moving caret end of the selection.
    pub caret: TextPosition,
}

impl TextSelection {
    /// Create a collapsed selection at a byte offset.
    ///
    /// # Arguments
    ///
    /// * `byte` - Byte offset for both the anchor and caret.
    ///
    /// # Returns
    ///
    /// A selection with no selected range.
    pub fn collapsed(byte: usize) -> Self {
        let position = TextPosition::new(byte);
        Self {
            anchor: position,
            caret: position,
        }
    }

    /// Return whether the selection is collapsed.
    ///
    /// # Returns
    ///
    /// `true` when anchor and caret are equal.
    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.caret
    }

    /// Return the selected byte range in forward order.
    ///
    /// # Returns
    ///
    /// `None` for a collapsed selection, otherwise the normalized byte range.
    pub fn normalized_range(&self) -> Option<Range<usize>> {
        if self.is_collapsed() {
            None
        } else {
            Some(self.start().byte..self.end().byte)
        }
    }

    /// Return the lower byte position of the selection.
    ///
    /// # Returns
    ///
    /// The minimum of anchor and caret.
    pub fn start(&self) -> TextPosition {
        self.anchor.min(self.caret)
    }

    /// Return the higher byte position of the selection.
    ///
    /// # Returns
    ///
    /// The maximum of anchor and caret.
    pub fn end(&self) -> TextPosition {
        self.anchor.max(self.caret)
    }
}

/// Scroll offset for a text view.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextViewScroll {
    /// Horizontal scroll offset in logical pixels.
    pub x: f32,
    /// Vertical scroll offset in logical pixels.
    pub y: f32,
}

/// Line wrapping behavior for a text view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WrapMode {
    /// Do not wrap lines; horizontal scrolling is used instead.
    #[default]
    None,
    /// Soft-wrap visual lines without inserting newline characters.
    Soft,
}

/// Tab insertion behavior for a text view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TabMode {
    /// Insert a tab character.
    #[default]
    Tab,
    /// Insert the given number of space characters.
    Spaces(u8),
}
