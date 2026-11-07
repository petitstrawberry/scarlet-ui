//! Spatial selection for immutable recordings. Index construction belongs to a
//! recording revision, not to a scroll frame. Leaves retain original paint order
//! and clip ancestry; skipping an offscreen command must not skip clip state.
use super::*;

struct Entry {
    command: usize,
    clip: Option<usize>,
}
struct Clip {
    command: usize,
    parent: Option<usize>,
}
pub(super) struct ReplayIndex {
    entries: Vec<Entry>,
    clips: Vec<Clip>,
    tree: Vec<PaintRegion>,
    leaves: usize,
}
fn union(a: PaintRegion, b: PaintRegion) -> PaintRegion {
    match (a, b) {
        (PaintRegion::Unknown, _) | (_, PaintRegion::Unknown) => PaintRegion::Unknown,
        (PaintRegion::Empty, b) => b,
        (a, PaintRegion::Empty) => a,
        (PaintRegion::Rect(a), PaintRegion::Rect(b)) => {
            let x = a.left().min(b.left());
            let y = a.top().min(b.top());
            PaintRegion::Rect(Rect::from_xywh(
                x,
                y,
                a.right().max(b.right()) - x,
                a.bottom().max(b.bottom()) - y,
            ))
        }
    }
}
fn hits(bounds: PaintRegion, region: Rect) -> bool {
    match bounds {
        PaintRegion::Unknown => true,
        PaintRegion::Empty => false,
        PaintRegion::Rect(r) => {
            r.left() < region.right()
                && region.left() < r.right()
                && r.top() < region.bottom()
                && region.top() < r.bottom()
        }
    }
}
impl ReplayIndex {
    fn new(context: &PaintContext<'_>, scale: u32) -> Self {
        let mut entries = Vec::new();
        let mut clips: Vec<Clip> = Vec::new();
        let mut clip = None;
        let mut bounds = Vec::new();
        for (command, c) in context.commands.iter().enumerate() {
            match c {
                PaintCommand::PushClip { .. } => {
                    clips.push(Clip {
                        command,
                        parent: clip,
                    });
                    clip = Some(clips.len() - 1);
                }
                PaintCommand::PopClip => clip = clip.and_then(|i| clips[i].parent),
                _ => {
                    entries.push(Entry { command, clip });
                    bounds.push(dependencies::command_dependencies(c, context, scale).writes);
                }
            }
        }
        let leaves = entries.len().max(1).next_power_of_two();
        let mut tree = alloc::vec![PaintRegion::Empty; leaves * 2];
        tree[leaves..leaves + bounds.len()].copy_from_slice(&bounds);
        for i in (1..leaves).rev() {
            tree[i] = union(tree[i * 2], tree[i * 2 + 1]);
        }
        Self {
            entries,
            clips,
            tree,
            leaves,
        }
    }
    fn query(&self, node: usize, region: Rect, result: &mut Vec<usize>) {
        if !hits(self.tree[node], region) {
            return;
        }
        if node >= self.leaves {
            result.push(node - self.leaves);
        } else {
            self.query(node * 2, region, result);
            self.query(node * 2 + 1, region, result);
        }
    }
}
impl DisplayList {
    /// Select paint intersecting a local reconstruction region, in original
    /// order and with its original clips. This does not add a clip or close
    /// backdrop dependencies: the caller must supply the complete input region.
    /// Stateful recordings conservatively retain their complete command stream.
    pub fn context_for_region(&self, region: Rect, scale: u32) -> PaintContext<'_> {
        let context = self.flat_context();
        if !self.self_contained {
            return context.flattened();
        }
        let cached = self
            .replay_indices
            .borrow()
            .iter()
            .find(|(s, _)| *s == scale)
            .map(|(_, index)| Arc::clone(index));
        let index = cached.unwrap_or_else(|| {
            let index = Arc::new(ReplayIndex::new(context, scale));
            let mut indices = self.replay_indices.borrow_mut();
            if indices.len() == 2 {
                indices.remove(0);
            }
            indices.push((scale, Arc::clone(&index)));
            index
        });
        let mut selected = Vec::new();
        index.query(1, region, &mut selected);
        let mut result = PaintContext::new();
        let mut active = Vec::new();
        let mut ancestry = Vec::new();
        for i in selected {
            let entry = &index.entries[i];
            ancestry.clear();
            let mut clip = entry.clip;
            while let Some(i) = clip {
                ancestry.push(i);
                clip = index.clips[i].parent;
            }
            ancestry.reverse();
            let common = active
                .iter()
                .zip(&ancestry)
                .take_while(|(a, b)| a == b)
                .count();
            for _ in common..active.len() {
                result.pop_clip();
            }
            active.truncate(common);
            for &i in &ancestry[common..] {
                result
                    .commands
                    .push(context.commands[index.clips[i].command].clone());
                active.push(i);
            }
            let mut command = context.commands[entry.command].clone();
            if let PaintCommand::DrawBuffer { buffer_idx, .. }
            | PaintCommand::DrawBufferRect { buffer_idx, .. } = &mut command
            {
                let Some(buffer) = context.buffers.get(*buffer_idx) else {
                    continue;
                };
                *buffer_idx = result.buffers.len();
                result.buffers.push(match buffer {
                    PaintBuffer::Shared(buffer) => PaintBuffer::Shared(Arc::clone(buffer)),
                    other => PaintBuffer::Borrowed(other.as_buffer()),
                });
            }
            result.commands.push(command);
        }
        for _ in active {
            result.pop_clip();
        }
        result
    }
}
