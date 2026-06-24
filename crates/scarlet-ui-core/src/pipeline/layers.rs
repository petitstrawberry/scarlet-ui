use crate::buffer::Buffer;
use crate::color::Color;
use crate::element::ElementId;
use crate::geometry::{Point, Rect, Size};
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum LayerId {
    Root,
    Boundary(ElementId),
    Chunk { owner: ElementId, ordinal: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerClip {
    pub(crate) rect: Rect,
    pub(crate) corner_radius: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LayerChild {
    Boundary {
        id: LayerId,
        offset: Point,
        clip: Option<LayerClip>,
    },
    Chunk {
        id: LayerId,
        offset: Point,
        clip: Option<LayerClip>,
    },
    Primitive(LayerPrimitive),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayerPrimitiveKind {
    RoundedRect { rect: Rect, corner_radius: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerPrimitive {
    pub(crate) owner: ElementId,
    pub(crate) kind: LayerPrimitiveKind,
    pub(crate) color: Color,
    pub(crate) clip: Option<LayerClip>,
}

pub(crate) struct LayerContainer {
    pub(crate) id: LayerId,
    pub(crate) owner: Option<ElementId>,
    pub(crate) logical_size: Size,
    pub(crate) scale_milli: u32,
    pub(crate) children: Vec<LayerChild>,
    primitive_ranges: BTreeMap<ElementId, (usize, usize)>,
    pub(crate) generation: u64,
    pub(crate) valid: bool,
    pub(crate) invalidated_by: Option<ElementId>,
}

pub(crate) struct PictureChunk {
    pub(crate) id: LayerId,
    pub(crate) owner: ElementId,
    pub(crate) ordinal: u16,
    pub(crate) logical_bounds: Rect,
    pub(crate) buffer: Arc<Buffer>,
    pub(crate) generation: u64,
    pub(crate) valid: bool,
}

pub(crate) struct LayerStore {
    generation: u64,
    root: LayerContainer,
    containers: BTreeMap<LayerId, LayerContainer>,
    chunks: BTreeMap<LayerId, PictureChunk>,
    invalid_layers: usize,
}

impl LayerContainer {
    pub(crate) fn new_root() -> Self {
        Self {
            id: LayerId::Root,
            owner: None,
            logical_size: Size::ZERO,
            scale_milli: 1000,
            children: Vec::new(),
            primitive_ranges: BTreeMap::new(),
            generation: 0,
            valid: false,
            invalidated_by: None,
        }
    }

    pub(crate) fn new(
        id: LayerId,
        owner: Option<ElementId>,
        logical_size: Size,
        scale_milli: u32,
        generation: u64,
    ) -> Self {
        Self {
            id,
            owner,
            logical_size,
            scale_milli,
            children: Vec::new(),
            primitive_ranges: BTreeMap::new(),
            generation,
            valid: true,
            invalidated_by: None,
        }
    }

    fn note_appended_child(&mut self, child: LayerChild) {
        let index = self.children.len();
        self.children.push(child);
        if let LayerChild::Primitive(primitive) = child {
            self.note_primitive_at(primitive.owner, index);
        }
    }

    fn note_primitive_at(&mut self, owner: ElementId, index: usize) {
        if let Some((start, len)) = self.primitive_ranges.get_mut(&owner)
            && start.saturating_add(*len) == index
        {
            *len = len.saturating_add(1);
            return;
        }
        self.primitive_ranges.insert(owner, (index, 1));
    }

    fn rebuild_primitive_ranges(&mut self) {
        self.primitive_ranges.clear();
        let mut index = 0usize;
        while index < self.children.len() {
            if let LayerChild::Primitive(primitive) = self.children[index] {
                self.note_primitive_at(primitive.owner, index);
            }
            index += 1;
        }
    }

    fn replace_stable_primitive_range(
        &mut self,
        owner: ElementId,
        primitives: &[Option<LayerPrimitive>; 2],
    ) -> bool {
        let replacement_count = primitives.iter().filter(|primitive| primitive.is_some()).count();
        let Some((start, len)) = self.primitive_ranges.get(&owner).copied() else {
            return false;
        };
        if replacement_count == 0 || replacement_count != len {
            return false;
        }
        let end = start.saturating_add(len);
        if end > self.children.len() {
            return false;
        }
        if !self.children[start..end].iter().all(|child| {
            matches!(*child, LayerChild::Primitive(LayerPrimitive { owner: child_owner, .. }) if child_owner == owner)
        }) {
            return false;
        }

        let mut index = start;
        for primitive in primitives.iter().flatten().copied() {
            self.children[index] = LayerChild::Primitive(primitive);
            index += 1;
        }
        true
    }
}

impl PictureChunk {
    pub(crate) fn new(
        owner: ElementId,
        ordinal: u16,
        logical_bounds: Rect,
        buffer: Buffer,
        generation: u64,
    ) -> Self {
        Self {
            id: LayerId::Chunk { owner, ordinal },
            owner,
            ordinal,
            logical_bounds,
            buffer: Arc::new(buffer),
            generation,
            valid: true,
        }
    }
}

impl LayerStore {
    pub(crate) fn new() -> Self {
        Self {
            generation: 0,
            root: LayerContainer::new_root(),
            containers: BTreeMap::new(),
            chunks: BTreeMap::new(),
            invalid_layers: 0,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.generation = 0;
        self.root = LayerContainer::new_root();
        self.containers.clear();
        self.chunks.clear();
        self.invalid_layers = 0;
    }

    pub(crate) fn begin_rebuild(&mut self) -> u64 {
        self.generation = self.generation.saturating_add(1);
        self.root.generation = self.generation;
        self.generation
    }

    pub(crate) fn mark_container(&mut self, id: LayerId, generation: u64) {
        if id == LayerId::Root {
            self.root.generation = generation;
            return;
        }
        if let Some(container) = self.containers.get_mut(&id) {
            container.generation = generation;
        }
    }

    pub(crate) fn mark_container_subtree(&mut self, id: LayerId, generation: u64) {
        self.mark_container(id, generation);
        let mut index = 0;
        loop {
            let Some(child) = self
                .container(id)
                .and_then(|container| container.children.get(index).copied())
            else {
                break;
            };
            match child {
                LayerChild::Boundary { id, .. } => self.mark_container_subtree(id, generation),
                LayerChild::Chunk { id, .. } => self.mark_chunk(id, generation),
                LayerChild::Primitive(_) => {}
            }
            index += 1;
        }
    }

    pub(crate) fn mark_chunk(&mut self, id: LayerId, generation: u64) {
        if let Some(chunk) = self.chunks.get_mut(&id) {
            chunk.generation = generation;
        }
    }

    pub(crate) fn prune_unmarked(&mut self, generation: u64) {
        let mut dropped_invalid = 0usize;
        self.containers.retain(|_, container| {
            let keep = container.generation >= generation;
            if !keep && !container.valid {
                dropped_invalid = dropped_invalid.saturating_add(1);
            }
            keep
        });
        self.chunks.retain(|_, chunk| {
            let keep = chunk.generation >= generation;
            if !keep && !chunk.valid {
                dropped_invalid = dropped_invalid.saturating_add(1);
            }
            keep
        });
        self.invalid_layers = self.invalid_layers.saturating_sub(dropped_invalid);
        self.root.children.retain(|child| match child {
            LayerChild::Boundary { id, .. } => self.containers.contains_key(id),
            LayerChild::Chunk { id, .. } => self.chunks.contains_key(id),
            LayerChild::Primitive(_) => true,
        });
        self.root.rebuild_primitive_ranges();
    }

    pub(crate) fn begin_container_rebuild(
        &mut self,
        id: LayerId,
        owner: Option<ElementId>,
        logical_size: Size,
        scale_milli: u32,
        generation: u64,
    ) {
        let container = if id == LayerId::Root {
            &mut self.root
        } else {
            self.containers.entry(id).or_insert_with(|| {
                LayerContainer::new(id, owner, logical_size, scale_milli, generation)
            })
        };
        if container.valid {
            self.invalid_layers = self.invalid_layers.saturating_add(1);
        }
        container.owner = owner;
        container.logical_size = logical_size;
        container.scale_milli = scale_milli;
        container.children.clear();
        container.primitive_ranges.clear();
        container.generation = generation;
        container.valid = false;
        container.invalidated_by = None;
    }

    pub(crate) fn finish_container_rebuild(&mut self, id: LayerId) {
        if let Some(container) = self.container_mut(id) {
            let was_invalid = !container.valid;
            container.valid = true;
            container.invalidated_by = None;
            if was_invalid {
                self.invalid_layers = self.invalid_layers.saturating_sub(1);
            }
        }
    }

    pub(crate) fn next_chunk_id(&self, owner: ElementId) -> LayerId {
        let ordinal = self
            .chunks
            .keys()
            .filter_map(|id| match *id {
                LayerId::Chunk {
                    owner: chunk_owner,
                    ordinal,
                } if chunk_owner == owner => Some(ordinal),
                _ => None,
            })
            .max()
            .map_or(0, |ordinal| ordinal.saturating_add(1));
        LayerId::Chunk { owner, ordinal }
    }

    pub(crate) fn append_child(&mut self, container: LayerId, child: LayerChild) {
        if let Some(container) = self.container_mut(container) {
            container.note_appended_child(child);
        }
    }

    pub(crate) fn replace_stable_primitive_range(
        &mut self,
        container: LayerId,
        owner: ElementId,
        primitives: &[Option<LayerPrimitive>; 2],
    ) -> bool {
        self.container_mut(container)
            .is_some_and(|container| container.replace_stable_primitive_range(owner, primitives))
    }

    pub(crate) fn rebuild_primitive_ranges(&mut self, container: LayerId) {
        if let Some(container) = self.container_mut(container) {
            container.rebuild_primitive_ranges();
        }
    }

    pub(crate) fn invalidate_layer(&mut self, id: LayerId, by: ElementId) {
        let mut became_invalid = false;
        match id {
            LayerId::Root | LayerId::Boundary(_) => {
                if let Some(container) = self.container_mut(id) {
                    if container.valid {
                        became_invalid = true;
                    }
                    container.valid = false;
                    container.invalidated_by = Some(by);
                }
            }
            LayerId::Chunk { .. } => {
                if let Some(chunk) = self.chunk_mut(id) {
                    if chunk.valid {
                        became_invalid = true;
                    }
                    chunk.valid = false;
                }
            }
        }
        if became_invalid {
            self.invalid_layers = self.invalid_layers.saturating_add(1);
        }
    }

    pub(crate) fn is_valid_for(&self, id: LayerId, logical_size: Size, scale_milli: u32) -> bool {
        let Some(container) = self.container(id) else {
            return false;
        };
        container.valid
            && container.logical_size == logical_size
            && container.scale_milli == scale_milli
    }

    pub(crate) fn root_graph_valid_for(&self, logical_size: Size, scale_milli: u32) -> bool {
        self.invalid_layers == 0 && self.is_valid_for(LayerId::Root, logical_size, scale_milli)
    }

    pub(crate) fn root(&self) -> &LayerContainer {
        &self.root
    }

    pub(crate) fn root_mut(&mut self) -> &mut LayerContainer {
        &mut self.root
    }

    pub(crate) fn insert_container(&mut self, container: LayerContainer) {
        if container.id == LayerId::Root {
            self.invalid_layers = self
                .invalid_layers
                .saturating_sub(usize::from(!self.root.valid));
            if !container.valid {
                self.invalid_layers = self.invalid_layers.saturating_add(1);
            }
            self.root = container;
        } else {
            if let Some(old) = self.containers.get(&container.id)
                && !old.valid
            {
                self.invalid_layers = self.invalid_layers.saturating_sub(1);
            }
            if !container.valid {
                self.invalid_layers = self.invalid_layers.saturating_add(1);
            }
            self.containers.insert(container.id, container);
        }
    }

    pub(crate) fn insert_chunk(&mut self, chunk: PictureChunk) {
        if let Some(old) = self.chunks.get(&chunk.id)
            && !old.valid
        {
            self.invalid_layers = self.invalid_layers.saturating_sub(1);
        }
        if !chunk.valid {
            self.invalid_layers = self.invalid_layers.saturating_add(1);
        }
        self.chunks.insert(chunk.id, chunk);
    }

    pub(crate) fn finish_chunk_rebuild(&mut self, id: LayerId) {
        if let Some(chunk) = self.chunk_mut(id) {
            let was_invalid = !chunk.valid;
            chunk.valid = true;
            if was_invalid {
                self.invalid_layers = self.invalid_layers.saturating_sub(1);
            }
        }
    }

    pub(crate) fn container(&self, id: LayerId) -> Option<&LayerContainer> {
        if id == LayerId::Root {
            Some(&self.root)
        } else {
            self.containers.get(&id)
        }
    }

    pub(crate) fn container_mut(&mut self, id: LayerId) -> Option<&mut LayerContainer> {
        if id == LayerId::Root {
            Some(&mut self.root)
        } else {
            self.containers.get_mut(&id)
        }
    }

    pub(crate) fn chunk(&self, id: LayerId) -> Option<&PictureChunk> {
        self.chunks.get(&id)
    }

    pub(crate) fn chunk_mut(&mut self, id: LayerId) -> Option<&mut PictureChunk> {
        self.chunks.get_mut(&id)
    }

    pub(crate) fn boundary_ids(&self) -> Vec<ElementId> {
        self.containers
            .keys()
            .filter_map(|id| match *id {
                LayerId::Boundary(owner) => Some(owner),
                _ => None,
            })
            .collect()
    }
}

impl Default for LayerStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_chunk(owner: ElementId, ordinal: u16, generation: u64) -> PictureChunk {
        PictureChunk::new(
            owner,
            ordinal,
            Rect::from_xywh(0.0, 0.0, 10.0, 10.0),
            Buffer::new(Size::new(10.0, 10.0)),
            generation,
        )
    }

    #[test]
    fn layer_id_distinguishes_multiple_chunks_for_same_owner() {
        let owner = ElementId::new(1);
        let first = LayerId::Chunk { owner, ordinal: 0 };
        let second = LayerId::Chunk { owner, ordinal: 1 };
        assert_ne!(first, second);

        let mut chunks = BTreeMap::new();
        chunks.insert(first, "first");
        chunks.insert(second, "second");

        assert_eq!(chunks[&first], "first");
        assert_eq!(chunks[&second], "second");
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn layer_container_preserves_child_order() {
        let owner = ElementId::new(2);
        let child_owner = ElementId::new(3);
        let mut container = LayerContainer::new(
            LayerId::Boundary(owner),
            Some(owner),
            Size::new(100.0, 100.0),
            1000,
            1,
        );
        let chunk_a = LayerId::Chunk { owner, ordinal: 0 };
        let boundary = LayerId::Boundary(child_owner);
        let chunk_b = LayerId::Chunk { owner, ordinal: 1 };

        container.children.push(LayerChild::Chunk {
            id: chunk_a,
            offset: Point::ZERO,
            clip: None,
        });
        container.children.push(LayerChild::Boundary {
            id: boundary,
            offset: Point::new(5.0, 6.0),
            clip: None,
        });
        container.children.push(LayerChild::Chunk {
            id: chunk_b,
            offset: Point::new(8.0, 9.0),
            clip: None,
        });

        assert!(matches!(container.children[0], LayerChild::Chunk { id, .. } if id == chunk_a));
        assert!(matches!(container.children[1], LayerChild::Boundary { id, .. } if id == boundary));
        assert!(matches!(container.children[2], LayerChild::Chunk { id, .. } if id == chunk_b));
    }

    #[test]
    fn layer_store_prunes_unmarked_chunks_and_containers() {
        let mut store = LayerStore::new();
        let kept_owner = ElementId::new(4);
        let dropped_owner = ElementId::new(5);
        let kept_container_id = LayerId::Boundary(kept_owner);
        let dropped_container_id = LayerId::Boundary(dropped_owner);
        let kept_chunk_id = LayerId::Chunk {
            owner: kept_owner,
            ordinal: 0,
        };
        let dropped_chunk_id = LayerId::Chunk {
            owner: dropped_owner,
            ordinal: 0,
        };
        let old_generation = store.begin_rebuild();
        store.insert_container(LayerContainer::new(
            kept_container_id,
            Some(kept_owner),
            Size::new(10.0, 10.0),
            1000,
            old_generation,
        ));
        store.insert_container(LayerContainer::new(
            dropped_container_id,
            Some(dropped_owner),
            Size::new(10.0, 10.0),
            1000,
            old_generation,
        ));
        store.insert_chunk(test_chunk(kept_owner, 0, old_generation));
        store.insert_chunk(test_chunk(dropped_owner, 0, old_generation));
        store.root_mut().children.push(LayerChild::Boundary {
            id: kept_container_id,
            offset: Point::ZERO,
            clip: None,
        });
        store.root_mut().children.push(LayerChild::Boundary {
            id: dropped_container_id,
            offset: Point::ZERO,
            clip: None,
        });
        store.root_mut().children.push(LayerChild::Chunk {
            id: kept_chunk_id,
            offset: Point::ZERO,
            clip: None,
        });
        store.root_mut().children.push(LayerChild::Chunk {
            id: dropped_chunk_id,
            offset: Point::ZERO,
            clip: None,
        });

        let generation = store.begin_rebuild();
        store.mark_container(kept_container_id, generation);
        store.mark_chunk(kept_chunk_id, generation);
        store.prune_unmarked(generation);

        assert!(store.container(kept_container_id).is_some());
        assert!(store.container(dropped_container_id).is_none());
        assert!(store.chunk(kept_chunk_id).is_some());
        assert!(store.chunk(dropped_chunk_id).is_none());
        assert_eq!(store.root().children.len(), 2);
    }

    #[test]
    fn layer_store_clear_drops_root_replace_state() {
        let mut store = LayerStore::new();
        let owner = ElementId::new(6);
        let generation = store.begin_rebuild();
        store.root_mut().valid = true;
        store.root_mut().children.push(LayerChild::Boundary {
            id: LayerId::Boundary(owner),
            offset: Point::ZERO,
            clip: None,
        });
        store.insert_container(LayerContainer::new(
            LayerId::Boundary(owner),
            Some(owner),
            Size::new(10.0, 10.0),
            1000,
            generation,
        ));
        store.insert_chunk(test_chunk(owner, 0, generation));

        store.clear();

        assert_eq!(store.root().id, LayerId::Root);
        assert_eq!(store.root().children.len(), 0);
        assert!(!store.root().valid);
        assert!(store.containers.is_empty());
        assert!(store.chunks.is_empty());
        assert_eq!(store.begin_rebuild(), 1);
    }

    #[test]
    fn root_graph_validity_tracks_child_invalidation_eagerly() {
        let mut store = LayerStore::new();
        let boundary_owner = ElementId::new(7);
        let chunk_owner = ElementId::new(8);
        let boundary_id = LayerId::Boundary(boundary_owner);
        let chunk_id = LayerId::Chunk {
            owner: chunk_owner,
            ordinal: 0,
        };
        let generation = store.begin_rebuild();
        store.begin_container_rebuild(
            LayerId::Root,
            None,
            Size::new(100.0, 100.0),
            1000,
            generation,
        );
        store.insert_container(LayerContainer::new(
            boundary_id,
            Some(boundary_owner),
            Size::new(10.0, 10.0),
            1000,
            generation,
        ));
        store.insert_chunk(test_chunk(chunk_owner, 0, generation));
        store.root_mut().children.push(LayerChild::Boundary {
            id: boundary_id,
            offset: Point::ZERO,
            clip: None,
        });
        store.root_mut().children.push(LayerChild::Chunk {
            id: chunk_id,
            offset: Point::ZERO,
            clip: None,
        });
        store.finish_container_rebuild(LayerId::Root);

        assert!(store.root_graph_valid_for(Size::new(100.0, 100.0), 1000));
        store.invalidate_layer(boundary_id, boundary_owner);
        assert!(!store.root_graph_valid_for(Size::new(100.0, 100.0), 1000));
        store.finish_container_rebuild(boundary_id);
        assert!(store.root_graph_valid_for(Size::new(100.0, 100.0), 1000));
        store.invalidate_layer(chunk_id, chunk_owner);
        assert!(!store.root_graph_valid_for(Size::new(100.0, 100.0), 1000));
        store.finish_chunk_rebuild(chunk_id);
        assert!(store.root_graph_valid_for(Size::new(100.0, 100.0), 1000));
    }
}
