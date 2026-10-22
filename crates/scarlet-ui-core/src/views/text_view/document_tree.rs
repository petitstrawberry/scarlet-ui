//! Persistent UTF-8 rope. Edits copy paths and at most the adjoining leaf text.
//!
//! Newline and Unicode-scalar counts are associative; grapheme counts are not.
//! Graphemes are therefore resolved by GraphemeCursor against these chunks in
//! document.rs, never by assuming a chunk boundary is a grapheme boundary.

use alloc::{string::String, sync::Arc, vec::Vec};
use core::ops::Range;

pub(super) const CHUNK_BYTES: usize = 2048;
pub(super) type Root = Option<Arc<Node>>;

#[derive(Debug)]
pub(super) struct Node {
    pub bytes: usize,
    pub chars: usize,
    pub newlines: usize,
    height: u16,
    kind: Kind,
}

#[derive(Debug)]
enum Kind {
    Leaf { text: Arc<str>, range: Range<usize> },
    Branch { left: Arc<Node>, right: Arc<Node> },
}

impl Node {
    fn leaf(text: Arc<str>, range: Range<usize>) -> Arc<Self> {
        let slice = &text[range.clone()];
        debug_assert!(!slice.is_empty() && slice.len() <= CHUNK_BYTES);
        Arc::new(Self {
            bytes: slice.len(),
            chars: slice.chars().count(),
            newlines: slice.bytes().filter(|byte| *byte == b'\n').count(),
            height: 1,
            kind: Kind::Leaf { text, range },
        })
    }

    fn branch(left: Arc<Self>, right: Arc<Self>) -> Arc<Self> {
        Arc::new(Self {
            bytes: left.bytes + right.bytes,
            chars: left.chars + right.chars,
            newlines: left.newlines + right.newlines,
            height: left.height.max(right.height) + 1,
            kind: Kind::Branch { left, right },
        })
    }

    pub fn leaf_text(&self) -> Option<&str> {
        match &self.kind {
            Kind::Leaf { text, range } => Some(&text[range.clone()]),
            Kind::Branch { .. } => None,
        }
    }

    fn children(&self) -> (&Arc<Node>, &Arc<Node>) {
        match &self.kind {
            Kind::Branch { left, right } => (left, right),
            Kind::Leaf { .. } => unreachable!("AVL rotation requires a branch"),
        }
    }

    /// A nonempty chunk containing byte, preferring the following chunk at a
    /// boundary. The document's final byte offset resolves to its last chunk.
    pub fn chunk_at(&self, mut byte: usize) -> (&str, usize) {
        debug_assert!(byte <= self.bytes);
        let mut node = self;
        let mut start = 0;
        loop {
            match &node.kind {
                Kind::Leaf { text, range } => return (&text[range.clone()], start),
                Kind::Branch { left, right } => {
                    if byte < left.bytes {
                        node = left;
                    } else {
                        byte -= left.bytes;
                        start += left.bytes;
                        node = right;
                    }
                }
            }
        }
    }

    pub fn append_range(&self, range: Range<usize>, output: &mut String) {
        if range.is_empty() {
            return;
        }
        match &self.kind {
            Kind::Leaf { text, range: leaf } => {
                output.push_str(&text[leaf.start + range.start..leaf.start + range.end]);
            }
            Kind::Branch { left, right } => {
                if range.start < left.bytes {
                    left.append_range(range.start..range.end.min(left.bytes), output);
                }
                if range.end > left.bytes {
                    right.append_range(
                        range.start.saturating_sub(left.bytes)..range.end - left.bytes,
                        output,
                    );
                }
            }
        }
    }

    pub fn newline_byte(&self, mut index: usize) -> usize {
        debug_assert!(index < self.newlines);
        let mut node = self;
        let mut byte = 0;
        loop {
            match &node.kind {
                Kind::Leaf { text, range } => {
                    return byte
                        + text[range.clone()]
                            .bytes()
                            .enumerate()
                            .filter_map(|(offset, ch)| (ch == b'\n').then_some(offset))
                            .nth(index)
                            .expect("newline summary matches leaf");
                }
                Kind::Branch { left, right } => {
                    if index < left.newlines {
                        node = left;
                    } else {
                        index -= left.newlines;
                        byte += left.bytes;
                        node = right;
                    }
                }
            }
        }
    }

    pub fn newlines_before(&self, byte: usize) -> usize {
        match &self.kind {
            Kind::Leaf { text, range } => text.as_bytes()[range.start..range.start + byte]
                .iter()
                .filter(|ch| **ch == b'\n')
                .count(),
            Kind::Branch { left, right } if byte >= left.bytes => {
                left.newlines + right.newlines_before(byte - left.bytes)
            }
            Kind::Branch { left, .. } => left.newlines_before(byte),
        }
    }

    pub fn char_to_byte(&self, index: usize) -> usize {
        if index >= self.chars {
            return self.bytes;
        }
        match &self.kind {
            Kind::Leaf { text, range } => text[range.clone()]
                .char_indices()
                .nth(index)
                .map(|(byte, _)| byte)
                .unwrap_or(self.bytes),
            Kind::Branch { left, right } if index >= left.chars => {
                left.bytes + right.char_to_byte(index - left.chars)
            }
            Kind::Branch { left, .. } => left.char_to_byte(index),
        }
    }

    /// byte must already be a scalar boundary.
    pub fn byte_to_char(&self, byte: usize) -> usize {
        match &self.kind {
            Kind::Leaf { text, range } => text[range.start..range.start + byte].chars().count(),
            Kind::Branch { left, right } if byte >= left.bytes => {
                left.chars + right.byte_to_char(byte - left.bytes)
            }
            Kind::Branch { left, .. } => left.byte_to_char(byte),
        }
    }
}

pub(super) fn from_str(text: &str) -> Root {
    let mut leaves = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + CHUNK_BYTES).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let leaf: Arc<str> = Arc::from(&text[start..end]);
        leaves.push(Node::leaf(leaf, 0..end - start));
        start = end;
    }
    fn build(leaves: &[Arc<Node>]) -> Root {
        match leaves.len() {
            0 => None,
            1 => Some(leaves[0].clone()),
            len => Some(Node::branch(
                build(&leaves[..len / 2]).unwrap(),
                build(&leaves[len / 2..]).unwrap(),
            )),
        }
    }
    build(&leaves)
}

fn balance(left: Arc<Node>, right: Arc<Node>) -> Arc<Node> {
    if left.height > right.height + 1 {
        let (ll, lr) = left.children();
        if ll.height >= lr.height {
            Node::branch(ll.clone(), Node::branch(lr.clone(), right))
        } else {
            let (lrl, lrr) = lr.children();
            Node::branch(
                Node::branch(ll.clone(), lrl.clone()),
                Node::branch(lrr.clone(), right),
            )
        }
    } else if right.height > left.height + 1 {
        let (rl, rr) = right.children();
        if rr.height >= rl.height {
            Node::branch(Node::branch(left, rl.clone()), rr.clone())
        } else {
            let (rll, rlr) = rl.children();
            Node::branch(
                Node::branch(left, rll.clone()),
                Node::branch(rlr.clone(), rr.clone()),
            )
        }
    } else {
        Node::branch(left, right)
    }
}

fn join(left: Root, right: Root) -> Root {
    match (left, right) {
        (None, root) | (root, None) => root,
        (Some(left), Some(right)) => {
            if left.height > right.height + 1 {
                let (ll, lr) = left.children();
                Some(balance(
                    ll.clone(),
                    join(Some(lr.clone()), Some(right)).unwrap(),
                ))
            } else if right.height > left.height + 1 {
                let (rl, rr) = right.children();
                Some(balance(
                    join(Some(left), Some(rl.clone())).unwrap(),
                    rr.clone(),
                ))
            } else {
                Some(Node::branch(left, right))
            }
        }
    }
}

/// Concatenate while compacting the two adjoining small leaves. This prevents
/// typing one byte repeatedly from retaining one leaf/allocation per keystroke.
pub(super) fn concat(left: Root, right: Root) -> Root {
    let (Some(l), Some(r)) = (&left, &right) else {
        return join(left, right);
    };
    let (last, last_start) = l.chunk_at(l.bytes);
    let (first, _) = r.chunk_at(0);
    if last.len() + first.len() <= CHUNK_BYTES {
        let mut merged = String::with_capacity(last.len() + first.len());
        merged.push_str(last);
        merged.push_str(first);
        let first_len = first.len();
        let (left, _) = split(left, last_start);
        let (_, right) = split(right, first_len);
        join(join(left, from_str(&merged)), right)
    } else {
        join(left, right)
    }
}

/// Split only at a validated scalar boundary. Unchanged subtrees are shared.
pub(super) fn split(root: Root, byte: usize) -> (Root, Root) {
    let Some(node) = root else {
        return (None, None);
    };
    if byte == 0 {
        return (None, Some(node));
    }
    if byte == node.bytes {
        return (Some(node), None);
    }
    debug_assert!(byte < node.bytes);
    match &node.kind {
        Kind::Leaf { text, range } => (
            Some(Node::leaf(text.clone(), range.start..range.start + byte)),
            Some(Node::leaf(text.clone(), range.start + byte..range.end)),
        ),
        Kind::Branch { left, right } => {
            if byte < left.bytes {
                let (before, after) = split(Some(left.clone()), byte);
                (before, join(after, Some(right.clone())))
            } else {
                let (before, after) = split(Some(right.clone()), byte - left.bytes);
                (join(Some(left.clone()), before), after)
            }
        }
    }
}

#[cfg(test)]
pub(super) fn validate(root: &Root) -> (usize, usize, u16) {
    let Some(node) = root else {
        return (0, 0, 0);
    };
    match &node.kind {
        Kind::Leaf { text, range } => {
            let text = &text[range.clone()];
            assert!(!text.is_empty() && text.len() <= CHUNK_BYTES);
            assert_eq!(node.bytes, text.len());
            assert_eq!(node.chars, text.chars().count());
            assert_eq!(node.newlines, text.bytes().filter(|b| *b == b'\n').count());
            assert_eq!(node.height, 1);
            (node.bytes, 1, 1)
        }
        Kind::Branch { left, right } => {
            let (lb, ll, lh) = validate(&Some(left.clone()));
            let (rb, rl, rh) = validate(&Some(right.clone()));
            assert!(lh.abs_diff(rh) <= 1, "unbalanced subtree: {lh} / {rh}");
            assert_eq!(node.bytes, lb + rb);
            assert_eq!(node.chars, left.chars + right.chars);
            assert_eq!(node.newlines, left.newlines + right.newlines);
            assert_eq!(node.height, lh.max(rh) + 1);
            (node.bytes, ll + rl, node.height)
        }
    }
}
