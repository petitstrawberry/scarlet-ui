use super::*;
use alloc::{format, vec::Vec};

fn check_document(document: &TextDocument, expected: &str) {
    tree::validate(&document.snapshot.root);
    assert_eq!(document.len(), expected.len());
    let mut start = 0;
    let mut lines = Vec::new();
    if !expected.is_empty() {
        for (byte, ch) in expected.char_indices() {
            if ch == '\n' {
                lines.push(start..byte + 1);
                start = byte + 1;
            }
        }
        lines.push(start..expected.len());
    }
    assert_eq!(document.line_count(), lines.len());
    for (line, range) in lines.iter().enumerate() {
        assert_eq!(document.line_range(line), Some(range.clone()));
        assert_eq!(
            document.line_text(line).as_deref(),
            Some(&expected[range.clone()])
        );
    }
    assert!(document.line_range(lines.len()).is_none());
    for byte in (0..=expected.len())
        .step_by(37)
        .chain([expected.len(), usize::MAX])
    {
        let clamped = clamp_byte_to_char_boundary(expected, byte);
        assert_eq!(
            document.byte_to_char(byte),
            expected[..clamped].chars().count()
        );
        assert_eq!(
            document.line_index_at_byte(byte),
            expected.as_bytes()[..byte.min(expected.len())]
                .iter()
                .filter(|&&ch| ch == b'\n')
                .count()
        );
        assert_eq!(
            document.clamp_position(TextPosition::new(byte)),
            TextPosition::new(byte).clamp_to_grapheme(expected)
        );
    }
    for (index, (byte, _)) in expected.char_indices().enumerate().step_by(31) {
        assert_eq!(document.char_to_byte(index), byte);
    }
    assert_eq!(document.char_to_byte(usize::MAX), expected.len());
    assert_eq!(document.as_str(), expected);
}

#[test]
fn chunk_edges_preserve_graphemes_and_crlf() {
    for sequence in ["\r\n", "e\u{301}", "👩‍💻", "🇯🇵🇬🇧🇺", "क्‍ष", "\u{600}a"]
    {
        for padding in 0..sequence.len() {
            let text = format!(
                "{}{}\ntail",
                "x".repeat(tree::CHUNK_BYTES - padding),
                sequence
            );
            let document = TextDocument::from_str(&text);
            for byte in tree::CHUNK_BYTES - sequence.len()..text.len() {
                assert_eq!(
                    document.clamp_position(TextPosition::new(byte)),
                    TextPosition::new(byte).clamp_to_grapheme(&text),
                    "sequence={sequence:?}, padding={padding}, byte={byte}"
                );
            }
            check_document(&document, &text);
        }
    }
    // Grapheme context can span several complete chunks, including RI parity.
    for text in [
        format!("e{}z", "\u{301}".repeat(tree::CHUNK_BYTES * 2)),
        "🇯".repeat(tree::CHUNK_BYTES),
    ] {
        check_document(&TextDocument::from_str(&text), &text);
    }
}

#[test]
fn mixed_edits_match_string_and_preserve_undo_snapshots() {
    let mut expected = "日本語 e\u{301} 👩‍💻 🇯🇵\r\n".repeat(300);
    let mut document = TextDocument::from_str(&expected);
    let mut snapshots = Vec::new();
    let mut random = 0x735ca17u64;
    for edit in 0..240 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let start = random as usize % (expected.len() + 9);
        let end = if edit % 13 == 0 {
            start.saturating_sub(3)
        } else {
            start.saturating_add((random >> 32) as usize % 90)
        };
        let replacement = ["", "x", "\n", "é日", "👨‍👩‍👧‍👦", "\r\n"][edit % 6];
        let clamped_start = clamp_byte_to_char_boundary(&expected, start);
        let clamped_end = clamp_byte_to_char_boundary(&expected, end).max(clamped_start);
        if edit % 40 == 0 {
            snapshots.push((document.clone(), expected.clone()));
        }
        let (next, delta) = document.replace(start..end, replacement);
        assert_eq!(delta.replaced_range, clamped_start..clamped_end);
        assert_eq!(
            delta.deleted_text(&expected),
            &expected[clamped_start..clamped_end]
        );
        expected.replace_range(clamped_start..clamped_end, replacement);
        document = next;
        check_document(&document, &expected);
    }
    for (snapshot, text) in snapshots {
        check_document(&snapshot, &text);
    }
    check_document(&document.delete(0..usize::MAX).0, "");
}

#[test]
fn local_edits_and_queries_do_not_flatten_the_document() {
    let original = TextDocument::from_str(&"line\n".repeat(10000));
    let (edited, _) = original.insert(TextPosition::new(25000), "日本語\n");
    assert!(!edited.same_revision(&original));
    assert_eq!(edited.line_text(5000).as_deref(), Some("日本語\n"));
    assert_eq!(edited.line_index_at_byte(25000), 5000);
    edited.clamp_position(TextPosition::new(25001));
    assert!(original.snapshot.flat.get().is_none());
    assert!(edited.snapshot.flat.get().is_none());
    let clone = edited.clone();
    let text = edited.as_str();
    assert_eq!(text.as_ptr(), clone.as_str().as_ptr());
    assert!(clone.same_revision(&edited));
    assert!(
        edited
            .replace(25000..25009, "日本語")
            .0
            .same_revision(&edited)
    );
}

#[test]
fn repeated_typing_keeps_the_tree_compact() {
    let mut document = TextDocument::from_str(&"a".repeat(tree::CHUNK_BYTES * 8));
    for edit in 0..4000 {
        document = document
            .insert(TextPosition::new(tree::CHUNK_BYTES * 4 + edit), "x")
            .0;
    }
    let (_, leaves, _) = tree::validate(&document.snapshot.root);
    assert!(
        leaves < 32,
        "typing fragmented the document into {leaves} leaves"
    );
    check_document(
        &document,
        &format!(
            "{}{}{}",
            "a".repeat(tree::CHUNK_BYTES * 4),
            "x".repeat(4000),
            "a".repeat(tree::CHUNK_BYTES * 4)
        ),
    );
}
