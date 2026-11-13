//! Fitting a label to a width: shortening it with an ellipsis, or wrapping it
//! onto lines up to a limit. Widths come from the same shaped advances that
//! drawing uses, so a fitted line never draws wider than its width.

use crate::graphics::TextMeasure;
use alloc::string::String;
use alloc::vec::Vec;
use core::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Where [`Text`](super::Text) shortens a line that does not fit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Truncation {
    /// `…ong name.rs`: keeps the end.
    Head,
    /// `src/…/name.rs`: keeps both ends, for paths and identifiers.
    Middle,
    /// `A long na…`: keeps the start.
    #[default]
    Tail,
}

pub(crate) const ELLIPSIS: &str = "…";

fn width(measure: &TextMeasure, text: &str) -> f32 {
    measure.size(text).0 as f32
}

/// Lines of `text` that fit in `max_width`: wrapped at break opportunities up
/// to `line_limit` lines (`None` for no limit), the last one shortened at its
/// end if text remains. A single line is shortened as `truncation` says.
pub(crate) fn fit(
    text: &str,
    measure: &TextMeasure,
    max_width: f32,
    line_limit: Option<usize>,
    truncation: Truncation,
) -> Vec<String> {
    let limit = line_limit.unwrap_or(usize::MAX).max(1);
    if limit == 1 {
        return alloc::vec![shorten(text, measure, max_width, truncation)];
    }
    let mut ranges = Vec::new();
    let mut start = 0;
    for paragraph in text.split('\n') {
        let end = start + paragraph.len();
        for line in wrap(paragraph, measure, max_width) {
            ranges.push(start + line.start..start + line.end);
        }
        start = end + 1;
    }
    if ranges.len() <= limit {
        return ranges
            .into_iter()
            .map(|range| text[range].trim_end().into())
            .collect();
    }
    let mut lines: Vec<String> = ranges[..limit - 1]
        .iter()
        .map(|range| text[range.clone()].trim_end().into())
        .collect();
    let rest = text[ranges[limit - 1].start..].replace('\n', " ");
    lines.push(shorten(&rest, measure, max_width, Truncation::Tail));
    lines
}

/// Ranges of `paragraph` (no line breaks) that fit `max_width` each, broken
/// where Unicode allows and inside a word only when the word alone is wider.
fn wrap(paragraph: &str, measure: &TextMeasure, max_width: f32) -> Vec<Range<usize>> {
    if paragraph.is_empty() {
        return alloc::vec![0..0];
    }
    let widths = measure.prefix_widths(paragraph);
    let mut opportunities = unicode_linebreak::linebreaks(paragraph).peekable();
    let graphemes: Vec<(usize, usize, bool)> = paragraph
        .grapheme_indices(true)
        .map(|(start, grapheme)| {
            let end = start + grapheme.len();
            while opportunities.peek().is_some_and(|(byte, _)| *byte < end) {
                opportunities.next();
            }
            let may_break = opportunities.peek().is_some_and(|(byte, _)| *byte == end);
            (start, end, may_break)
        })
        .collect();
    let mut lines = Vec::new();
    let mut first = 0;
    while first < graphemes.len() {
        // Spaces where a line was broken do not start the next one.
        while first < graphemes.len()
            && first > 0
            && paragraph[graphemes[first].0..graphemes[first].1]
                .trim()
                .is_empty()
        {
            first += 1;
        }
        if first == graphemes.len() {
            break;
        }
        let origin = widths[graphemes[first].0];
        let mut end = first;
        let mut last_break = None;
        while end < graphemes.len() {
            // Trailing spaces may hang past the edge, as they are not drawn.
            let line = paragraph[graphemes[first].0..graphemes[end].1].trim_end();
            let right = widths[graphemes[first].0 + line.len()] - origin;
            if TextMeasure::rounded_width(right) > max_width && end > first {
                end = last_break.unwrap_or(end);
                break;
            }
            if graphemes[end].2 {
                last_break = Some(end + 1);
            }
            end += 1;
        }
        lines.push(graphemes[first].0..graphemes[end - 1].1);
        first = end;
    }
    lines
}

/// `text` shortened with an ellipsis so it fits `max_width`, or unchanged if
/// it already does. Empty when not even the ellipsis fits.
pub(crate) fn shorten(
    text: &str,
    measure: &TextMeasure,
    max_width: f32,
    truncation: Truncation,
) -> String {
    if width(measure, text) <= max_width {
        return text.into();
    }
    let ellipsis = width(measure, ELLIPSIS);
    if ellipsis > max_width {
        return String::new();
    }
    let budget = max_width - ellipsis;
    let widths = measure.prefix_widths(text);
    let span = |range: Range<usize>| widths[range.end] - widths[range.start];
    let bounds: Vec<usize> = text
        .grapheme_indices(true)
        .map(|(start, _)| start)
        .chain(core::iter::once(text.len()))
        .collect();
    // The longest start that fits, the longest end that fits.
    let head = |budget: f32| {
        bounds
            .iter()
            .rev()
            .copied()
            .find(|end| TextMeasure::rounded_width(span(0..*end)) <= budget)
            .unwrap_or(0)
    };
    let tail = |budget: f32| {
        bounds
            .iter()
            .copied()
            .find(|start| TextMeasure::rounded_width(span(*start..text.len())) <= budget)
            .unwrap_or(text.len())
    };
    let compose = |end: usize, start: usize| {
        let mut out = String::new();
        out.push_str(text[..end].trim_end());
        out.push_str(ELLIPSIS);
        out.push_str(text[start..].trim_start());
        out
    };
    let (mut end, mut start) = match truncation {
        Truncation::Tail => (head(budget), text.len()),
        Truncation::Head => (0, tail(budget)),
        Truncation::Middle => {
            let end = head(budget / 2.0);
            let start = tail(budget - span(0..end));
            // Give what the end left over back to the start.
            let end = head(budget - span(start..text.len())).max(end);
            (end, start.max(end))
        }
    };
    // Shaping across the cut can differ slightly from the prefix sums; drop
    // one more grapheme until the drawn width fits.
    loop {
        let candidate = compose(end, start);
        if width(measure, &candidate) <= max_width || (end == 0 && start == text.len()) {
            return candidate;
        }
        let drop_end = match truncation {
            Truncation::Tail => true,
            Truncation::Head => false,
            Truncation::Middle => span(0..end) >= span(start..text.len()),
        };
        if drop_end && end > 0 {
            end = bounds[bounds.partition_point(|b| *b < end) - 1];
        } else if start < text.len() {
            start = bounds[bounds.partition_point(|b| *b <= start)];
        } else {
            end = bounds[bounds.partition_point(|b| *b < end) - 1];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure() -> TextMeasure {
        TextMeasure::new("abc", 13.0)
    }

    #[test]
    fn short_text_is_unchanged_and_long_text_is_shortened_where_asked() {
        let m = measure();
        let text = "src/views/text_fit.rs";
        let full = width(&m, text);
        assert_eq!(shorten(text, &m, full, Truncation::Tail), text);
        let narrow = full * 0.6;
        for truncation in [Truncation::Tail, Truncation::Head, Truncation::Middle] {
            let short = shorten(text, &m, narrow, truncation);
            assert!(width(&m, &short) <= narrow, "{truncation:?}: {short}");
            assert!(short.contains(ELLIPSIS), "{truncation:?}: {short}");
            assert!(
                short.chars().count() > 4,
                "{truncation:?} kept too little: {short}"
            );
            match truncation {
                Truncation::Tail => assert!(short.starts_with("src/") && short.ends_with(ELLIPSIS)),
                Truncation::Head => assert!(short.starts_with(ELLIPSIS) && short.ends_with(".rs")),
                Truncation::Middle => assert!(short.starts_with("src") && short.ends_with(".rs")),
            }
        }
        assert_eq!(shorten(text, &m, 0.5, Truncation::Tail), "");
        assert_eq!(
            shorten(text, &m, width(&m, ELLIPSIS), Truncation::Tail),
            ELLIPSIS
        );
    }

    #[test]
    fn graphemes_are_never_split() {
        let m = measure();
        let text = "👩‍👩‍👧 family e\u{301}te\u{301}";
        for cut in 1..40 {
            let short = shorten(text, &m, cut as f32, Truncation::Middle);
            let kept: String = short.replace(ELLIPSIS, "");
            assert!(
                kept.graphemes(true).all(|g| text.contains(g)),
                "split a grapheme at {cut}: {short:?}"
            );
        }
    }

    #[test]
    fn wrapping_breaks_between_words_and_limits_lines() {
        let m = measure();
        let text = "one two three four five six seven";
        let line = width(&m, "one two three");
        let lines = fit(text, &m, line, None, Truncation::Tail);
        assert_eq!(lines, ["one two three", "four five six", "seven"]);
        for l in &lines {
            assert!(width(&m, l) <= line);
        }
        let limited = fit(text, &m, line, Some(2), Truncation::Tail);
        assert_eq!(limited[0], "one two three");
        assert!(limited[1].starts_with("four") && limited[1].ends_with(ELLIPSIS));
        assert!(width(&m, &limited[1]) <= line);
        // Explicit line breaks stay; a word wider than the line is cut.
        assert_eq!(fit("a\nb", &m, 100.0, None, Truncation::Tail), ["a", "b"]);
        let long = fit(
            "abcdefghijklmnop",
            &m,
            width(&m, "abcdef"),
            None,
            Truncation::Tail,
        );
        assert!(long.len() > 1 && long.iter().all(|l| width(&m, l) <= width(&m, "abcdef")));
        assert_eq!(long.concat(), "abcdefghijklmnop");
    }
}
