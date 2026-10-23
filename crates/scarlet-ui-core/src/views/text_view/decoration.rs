//! Paint-only document decorations and their interval index.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ops::Range;

use crate::color::Color;

/// A foreground color over a half-open UTF-8 byte range in the document.
///
/// Later spans in the supplied vector win overlaps. Empty or reversed ranges
/// are ignored, ranges are clipped to the document, and endpoints inside a
/// UTF-8 character expand outward to include that character. Newlines have no
/// painted width; tabs color all their expanded spaces.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyleSpan {
    /// Half-open byte range in the current document.
    pub range: Range<usize>,
    /// Color used to paint this decoration.
    pub color: Color,
}

/// A straight diagnostic underline over a half-open document UTF-8 byte range.
///
/// Range handling matches [`TextStyleSpan`]. Later spans paint over earlier
/// underlines. These marks do not change text color or reserve layout space.
#[derive(Clone, Debug, PartialEq)]
pub struct TextDiagnosticSpan {
    /// Half-open byte range in the current document.
    pub range: Range<usize>,
    /// Color used to paint this decoration.
    pub color: Color,
}

#[derive(Clone, Debug)]
pub(super) struct Decoration {
    /// Half-open byte range in the current document.
    pub range: Range<usize>,
    pub color: Color,
    pub priority: usize,
    max_end: usize,
}

/// Prefix maximum endpoints allow binary-search queries even for nested spans.
#[derive(Clone, Default)]
pub(super) struct DecorationIndex(Arc<[Decoration]>);

impl DecorationIndex {
    pub fn new(spans: impl Iterator<Item = (Range<usize>, Color)>) -> Self {
        let mut spans: Vec<_> = spans
            .enumerate()
            .filter(|(_, (range, _))| range.start < range.end)
            .map(|(priority, (range, color))| Decoration {
                range,
                color,
                priority,
                max_end: 0,
            })
            .collect();
        spans.sort_unstable_by_key(|span| span.range.start);
        let mut max_end = 0;
        for span in &mut spans {
            max_end = max_end.max(span.range.end);
            span.max_end = max_end;
        }
        Self(spans.into())
    }

    pub fn intersecting(&self, range: Range<usize>) -> impl Iterator<Item = &Decoration> {
        let start = self.0.partition_point(|span| span.max_end <= range.start);
        let end = self.0.partition_point(|span| span.range.start < range.end);
        self.0[start.min(end)..end]
            .iter()
            .filter(move |span| span.range.end > range.start)
    }
}

#[derive(Clone, Default)]
pub(super) struct TextDecorations {
    pub foreground: DecorationIndex,
    pub diagnostics: DecorationIndex,
}
