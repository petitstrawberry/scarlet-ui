# Retained Layer Tree — Allocation-Free Paint Path

## Goal
Refactor the ScarletUI paint path (`crates/scarlet-ui-core/src/`) from a flat per-element repaint-boundary bitmap cache into a **retained layer/composite pipeline** that preserves nested repaint boundaries and reaches **zero heap allocation on warm steady-state scrolling**. The rollout must remain behaviorally safe: every phase compiles, passes tests, and never increases the measured warm-scroll allocation count.

## Origin / Context
- Current good commit on `main`: `a2b31ea` (Add retained repaint boundary caching).
- WIP experiment `wip/repaint-layer-tree-container` (`ab6510d`) is **reference only**, not to be merged.
- **Root cause of jank (verified, on `main` today):** `cache_nested_boundaries(false)` + `has_descendant_repaint_boundary` skip at `pipeline/rendering.rs:462-474` discards the parent boundary cache whenever a nested boundary exists. Widget Factory Display page (parent `ScrollView` ⊃ inner `ScrollView`) therefore re-rasters the parent viewport every scroll frame. The test `scroll_view_auto_boundary_skips_nested_boundaries` (`rendering.rs:1258-1277`) encodes this bad behavior as intended and MUST be replaced.

## Hard Constraints (non-negotiable, gates every phase)
1. **Zero per-frame heap allocation in steady-state scrolling** is the final acceptance criterion (Phase 7).
2. Every phase compiles.
3. Every phase passes existing tests + that phase's new tests.
4. Every phase measures warm-scroll heap allocations and is **strictly ≤ previous** accepted phase.
5. Composite of retained cached subtrees **must bypass `PaintContext`** (direct blit).
6. `PaintContext<'a>` remains **rebuild-only**; never retained by pipeline-owned structures.
7. Pipeline-owned retained records **must not store** `PaintBuffer::Borrowed(&'a Buffer)`.
8. Scroll offset alone **must not** invalidate child retained boundary content.
9. `cache_nested_boundaries(false)` **must not** discard a parent cache/layer (redefine semantics).
10. Composite must **never emit `PaintCommand`s** for retained layer composition.

## Verified Current Architecture (file:line anchors)
- Frame entry: `RenderingPipeline::render_paint_path` — `pipeline/rendering.rs:320`.
- Per-frame `PaintContext::new()` — `pipeline/rendering.rs:352` (`Vec<PaintCommand>` + `Vec<PaintBuffer>`).
- Recursive walk: `walk_and_paint` — `pipeline/rendering.rs:387-443`. Order: `paint_self → push_clip → children-or-cached-boundary → paint_overlay`.
- Nested skip (jank source): `pipeline/rendering.rs:462-474`.
- Flat cache: `paint_caches: BTreeMap<ElementId, PaintCache>` — `pipeline/rendering.rs:24-30,46`.
- Boundary rebuild: `build_repaint_boundary_context` — `pipeline/rendering.rs:637-657` → `CpuPaintRenderer::execute_into_buffer`.
- Invalidation (ancestor-wide): `invalidate_repaint_boundary_caches` — `pipeline/rendering.rs:766-807` via `find_path_ids`.
- Per-frame dirty: `paint_dirty_rects` — `pipeline/rendering.rs:827-863` builds `BTreeSet<ElementId>` + `Vec<Rect>`.
- `PaintBuffer<'a>`: `renderer.rs:138-156` — lifetime `'a` tied to paint pass.
- `execute_into_buffer` creates fresh renderer each rebuild: `renderer.rs:485-503`.
- Per-command `scaled: Vec<Point>` — `renderer.rs:688,789`; `crossings: Vec<f32>` per scanline — `renderer.rs:1022`; path helpers allocate `Vec<Point>` — `renderer.rs:26-79`.
- ScrollView wraps child: `views/scroll.rs:437-448`; offset via `set_position(-offset)` — `scroll.rs:783,931-940`; scrollbar in `paint_overlay` — `scroll.rs:910-928` (**dynamic**); clip — `scroll.rs:856-858`.

---

## Phased Work Breakdown

> TDD loop per phase: write/update failing tests → smallest production change → targeted test gate → `cargo test -p scarlet-ui-core --lib` → warm-scroll allocation gate → commit.

### Phase 1: baseline-gate
**Scope:** Establish a repeatable allocation + behavior baseline before refactoring. Test-only instrumentation; no production behavior change.

**Files:**
- `crates/scarlet-ui-core/src/lib.rs` — `#[cfg(test)]` test-support wiring.
- `crates/scarlet-ui-core/src/testing/mod.rs` — **new** test-only module.
- `crates/scarlet-ui-core/src/testing/alloc_counter.rs` — **new** counting allocator + measurement helpers.
- `crates/scarlet-ui-core/src/pipeline/rendering.rs` — test-only counters around `PaintContext::new` (L352), `walk_and_paint` (L387-443), `paint_repaint_boundary` (L445-620), rebuild branches (L517-597).

**New types/functions:**
```rust
pub(crate) struct AllocationSnapshot { allocations: usize, deallocations: usize, allocated_bytes: usize }
pub(crate) fn reset_allocation_counts();
pub(crate) fn allocation_snapshot() -> AllocationSnapshot;
pub(crate) fn measure_allocations<F: FnOnce()>(f: F) -> AllocationSnapshot;

#[cfg(test)] struct PaintTestCounters {
    paint_context_news: usize, walk_and_paint_calls: usize,
    boundary_rebuilds: usize, retained_composites: usize,
}
```

**Removed/deprecated:** none.

**Parallelizable within phase:** Yes — allocation harness and behavior-baseline tests independently, then integrate in `rendering.rs`.

**Test gate:**
- All existing rendering tests stay green (notably `scroll_view_auto_boundary_skips_nested_boundaries`, `repaint_boundary_cache_*`, `present_damage_*`, `scroll_view_updates_lazy_vstack_viewport_hint`).
- Add `warm_scroll_allocation_baseline_is_measured`.
- Add `warm_scroll_path_counter_baseline_is_measured` (proves warm scroll currently enters `PaintContext::new` + `walk_and_paint`, so later fast-path tests can detect bypass).
- Add `allocation_counter_measurement_does_not_allocate_while_reading_counts`.

**Allocation gate:** Record `PHASE_1_WARM_SCROLL_ALLOCS` = max budget for Phase 2. Phase 7 target = 0.

**Risk:** Counting-allocator noise from setup/formatting/lazy-init inside the measured window → tight measurement loop, reset right before the measured frame.

---

### Phase 2: retained-scratch
**Scope:** Move raster scratch into the long-lived `CpuPaintRenderer`; stop creating a fresh renderer per offscreen boundary rebuild.

**Files:**
- `crates/scarlet-ui-core/src/renderer.rs` — add `RasterScratch` fields to `CpuPaintRenderer` (near L441-449); replace per-command `scaled: Vec<Point>` (L688-690, L789-790); replace per-scanline `crossings: Vec<f32>` (L1022); replace `EffectiveClipRects::Rects(Vec<Rect>)` alloc sites (L462-466, L638-650) with scratch-backed iteration; convert static `execute_into_buffer` (L485-503) → instance method reusing scratch.
- `crates/scarlet-ui-core/src/pipeline/rendering.rs` — callsites L557-562, L581-586 use `self.paint_renderer.as_mut().unwrap()`; `paint_renderer` already retained at L40.

**New types/functions:**
```rust
pub(crate) struct RasterScratch { scaled_points, crossings, clip_rects, path_points: Vec<_> }
impl CpuPaintRenderer {
    pub fn execute_into_external_buffer(&mut self, buffer: &mut Buffer, bg: Color, ctx: &PaintContext<'_>, damage: Option<&[Rect]>);
    pub fn reserve_scratch_for_path(&mut self, points: usize);
}
```

**Removed/deprecated:**
- Deprecate static `CpuPaintRenderer::execute_into_buffer`.
- Remove fresh `CpuPaintRenderer { layer_stack: Vec::new(), ... }` creation in boundary rebuild.
- Remove per-scanline `crossings: Vec<f32>`.

**Parallelizable within phase:** Yes — renderer scratch migration + pipeline callsite migration split, merge before tests.

**Test gate:**
- Existing renderer + repaint-boundary tests pass.
- Add `cpu_paint_renderer_reuses_scaled_point_scratch` (2 runs, no capacity growth).
- Add `cpu_paint_renderer_reuses_scanline_crossings_scratch`.
- Add `boundary_rebuild_uses_pipeline_renderer_scratch`.
- Run `cargo test -p scarlet-ui-core --lib renderer pipeline::rendering`.

**Allocation gate:** `≤ PHASE_1_WARM_SCROLL_ALLOCS`. Reduces rebuild allocations; warm scroll may still allocate via `PaintContext::new`, `dirty_ids.to_vec()`, `BTreeSet`, command construction.

**Risk:** Borrowing the renderer's window buffer while rendering into an external boundary buffer → structure as non-reentrant buffer swap; add `paint_renderer_busy: bool` debug guard.

---

### Phase 3: LayerStore-intro
**Scope:** Introduce retained layer data model in **shadow mode** (buildable/inspectable/prunable, but main output still from existing paint path).

**Files:**
- `crates/scarlet-ui-core/src/pipeline/layers.rs` — **new** module: `LayerId`, `LayerClip`, `LayerChild`, `LayerContainer`, `PictureChunk`, `LayerStore` (generation/prune/clear).
- `crates/scarlet-ui-core/src/pipeline/mod.rs` — declare private `layers` module.
- `crates/scarlet-ui-core/src/pipeline/rendering.rs` — add `layer_store: LayerStore` field; init in `new`; clear in `teardown` (L84-94), `set_scale_milli` (L97-120), `set_root` (L128-134), `resize` (L254-280). Keep `paint_caches` temporarily.

**New types/functions:**
```rust
enum LayerId { Root, Boundary(ElementId), Chunk { owner: ElementId, ordinal: u16 } }
struct LayerClip { rect: Rect, corner_radius: f32 }
enum LayerChild { Boundary { id, offset, clip }, Chunk { id, offset, clip } }
struct LayerContainer { id, owner: Option<ElementId>, logical_size, scale_milli, children: Vec<LayerChild>, generation, valid, invalidated_by }
struct PictureChunk { id, owner, ordinal, logical_bounds, buffer: Buffer, generation, valid }
struct LayerStore { generation, root, containers: BTreeMap<LayerId, LayerContainer>, chunks: BTreeMap<LayerId, PictureChunk> }
impl LayerStore { new(); clear(); begin_rebuild() -> u64; mark_container/mark_chunk; prune_unmarked(gen); container/chunk accessors }
```

**Removed/deprecated:** no new use of `PaintCache`; do not remove `paint_caches` yet.

**Parallelizable within phase:** Yes — `LayerStore` unit tests independent of pipeline field wiring.

**Test gate:**
- Existing tests unchanged.
- Add `layer_id_distinguishes_multiple_chunks_for_same_owner`.
- Add `layer_container_preserves_child_order`.
- Add `layer_store_prunes_unmarked_chunks_and_containers`.
- Add `layer_store_clear_drops_root_replace_state`.
- Run `cargo test -p scarlet-ui-core --lib layer_store pipeline::rendering`.

**Allocation gate:** `≤ Phase 2`. Shadow population must not run on the measured warm-scroll path (or be allocation-stable when warm).

**Risk:** Keying picture chunks by `ElementId` only collapses multiple chunks for one owner → MUST use `LayerId::Chunk { owner, ordinal }`.

---

### Phase 4: nested-rebuild
**Scope:** Replace flat cache model with retained boundary containers + ordered picture chunks. Remove nested-skip; preserve parent + child retained layers.

**Files:**
- `crates/scarlet-ui-core/src/pipeline/rendering.rs` — replace nested-skip (L462-474); replace `paint_repaint_boundary` (L445-620); replace `build_repaint_boundary_context` (L637-657) with retained chunk-flush rebuild; keep `walk_and_paint` as rebuild-only traversal.
- `crates/scarlet-ui-core/src/pipeline/layers.rs` — chunk ordinal allocation, container child reset/append, validity tracking.
- `crates/scarlet-ui-core/src/renderer.rs` — use `execute_into_external_buffer` (Phase 2) for chunk raster.
- `crates/scarlet-ui-core/src/element/render.rs` — redefine doc for `repaint_boundary_cache_nested_boundaries` (L93-97): `false` no longer means "discard parent cache".
- `crates/scarlet-ui-core/src/views/scroll.rs` — keep `.cache_nested_boundaries(false)` (L443-445) but it must not trigger parent-cache removal.

**New types/functions:**
```rust
impl LayerStore { begin_container_rebuild(...); next_chunk_id(owner) -> LayerId; append_child(container, child); invalidate_layer(id, by); is_valid_for(id, size, scale) }
impl RenderingPipeline { rebuild_boundary_layer(...); rebuild_picture_chunk(owner, ordinal, ctx, bounds) -> LayerId; append_boundary_reference(...) }
```

**Removed/deprecated:**
- Remove parent-cache-removal-on-nested behavior.
- Deprecate `PaintCache` as primary retained store.
- Deprecate `has_descendant_repaint_boundary` as a skip predicate (diagnostic only, if kept).

**Parallelizable within phase:** Partially — `LayerStore` ordering tests + doc/API changes independent; the `rendering.rs` rebuild rewrite is coupled (one implementer).

**Test gate:**
- Update `repaint_boundary_cache_survives_scroll_offset_repaint` → assert retained boundary/chunk identity, not `paint_caches` pointer.
- Update `repaint_boundary_cache_is_invalidated_by_descendant_dirty` → descendant dirty rebuilds appropriate retained layer without destroying unrelated layers.
- Add `nested_repaint_boundary_rebuild_preserves_parent_and_child_layers`.
- Add `nested_repaint_boundary_child_is_composited_through_parent_order` (chunk-before-child, chunk-after-child).
- **Replace** `scroll_view_auto_boundary_skips_nested_boundaries` → `scroll_view_auto_boundary_retains_nested_boundaries` (both outer + inner retained).
- Keep `repaint_boundary_cache_respects_ancestor_clip`.
- Run `cargo test -p scarlet-ui-core --lib repaint_boundary scroll_view_auto_boundary present_damage`.

**Allocation gate:** `≤ Phase 3`. Cold/miss frames may allocate; warm scroll after retained layers built must not exceed Phase 3.

**Risk:** Paint-ordering regression around `paint_self`/clips/child-boundary/overlay (current order `paint_self → push_clip → children → paint_overlay`).

---

### Phase 5: nearest-invalidation
**Scope:** Replace ancestor-wide invalidation + per-frame dirty scratch allocation with **nearest-layer** invalidation + retained `DirtyScratch`.

**Files:**
- `crates/scarlet-ui-core/src/element/tree.rs` — add non-allocating path-fill API next to `find_path_ids` (L104-113).
- `crates/scarlet-ui-core/src/pipeline/rendering.rs` — replace `dirty_ids.to_vec()` (L333); replace `invalidate_repaint_boundary_caches` (L766-807); replace `paint_dirty_rects` (L827-863, drop `BTreeSet`); retained damage scratch for `present_damage_rects` callers (L340-349, L42).
- `crates/scarlet-ui-core/src/pipeline/layers.rs` — nearest-invalidation helpers, layer lookup by owner.
- `crates/scarlet-ui-core/src/pipeline/owner.rs` — no change; keep reading `last_paint_ids()` (L526-529) as borrowed slice.

**New types/functions:**
```rust
impl ElementTree { find_path_ids_into(target, out: &mut Vec<ElementId>) -> bool; find_nearest_repaint_boundary_in_path(path) -> Option<ElementId>; find_nearest_repaint_boundary_ancestor_into(target, path) -> Option<ElementId> }
struct DirtyScratch { ids, path, rects, damage: Vec<_> }  // clear_for_frame()
impl RenderingPipeline { invalidate_nearest_retained_layers(dirty_ids); dirty_rects_into(dirty_ids); present_damage_rects_into(rects, size, scale, out) }
```

**Removed/deprecated:**
- Deprecate allocation-returning `ElementTree::find_path_ids` for paint-invalidation use.
- Remove `BTreeSet<ElementId>` from paint dirty-rect path.
- Remove ancestor-wide invalidation as default policy.

**Parallelizable within phase:** Yes — `ElementTree` non-allocating API + `DirtyScratch`/damage plumbing independent, joined in `RenderingPipeline`.

**Test gate:**
- Keep `present_damage_keeps_large_partial_region`, `present_damage_uses_full_for_whole_window`.
- Update `repaint_boundary_cache_is_invalidated_by_descendant_dirty` → only nearest ancestor invalidated.
- Add `nested_repaint_boundary_dirty_descendant_invalidates_inner_only`.
- Add `dirty_root_invalidates_root_layer_only`.
- Add `find_path_ids_into_reuses_capacity`.
- Add `paint_dirty_rects_does_not_allocate_after_capacity_warmup`.
- Run `cargo test -p scarlet-ui-core --lib dirty_rects repaint_boundary present_damage`.

**Allocation gate:** `≤ Phase 4`. Eliminates `dirty_ids.to_vec()`, `BTreeSet`, dirty-rect `Vec` creation after warm-up.

**Risk:** Nearest-layer invalidation can leave stale pixels if a dirty descendant affects ancestor visuals outside its nearest boundary (ancestor overlays, selection overlays, clips).

---

### Phase 6: composite-fast-path
**Scope:** Add retained composite fast path. Warm scroll frames with valid retained layers and no rebuild-required dirty layer: skip `PaintContext::new`, skip `walk_and_paint`, skip boundary rebuild; directly blit layer tree into window buffer.

**Files:**
- `crates/scarlet-ui-core/src/pipeline/rendering.rs` — fast-path branch in `render_paint_path` before `PaintContext::new` (L352); eligibility checks; retained composite traversal over `LayerStore`.
- `crates/scarlet-ui-core/src/pipeline/layers.rs` — immutable traversal of root/container children, fast-path validity.
- `crates/scarlet-ui-core/src/renderer.rs` — direct composite methods (blit `Buffer`/subrects with offset+clip, no `PaintCommand`).
- `crates/scarlet-ui-core/src/buffer.rs` — reuse existing `composite`/`composite_clipped`/`composite_rect`/`composite_rect_clipped`; add small non-allocating helpers only if needed.
- `crates/scarlet-ui-core/src/views/scroll.rs` — for this phase's zero-alloc test, use a scrollbar-disabled (`ScrollbarVisibility::Never`) warm scroll scenario; default dynamic scrollbar finalized in Phase 7.

**New types/functions:**
```rust
enum RetainedCompositeResult { Composited, NeedsRebuild, BlockedByDynamicOverlay, Empty }
struct CompositeState { offset: Point, clip: Option<LayerClip>, opacity: f32 }
impl RenderingPipeline { try_composite_retained_frame(bg, dirty_ids) -> RetainedCompositeResult; retained_fast_path_allowed(dirty_ids) -> bool; composite_layer_tree(id, state); composite_layer_child(child, state) }
impl CpuPaintRenderer { begin_retained_composite(bg, damage); composite_buffer_rect_with_clip(src, src_rect, dst, clip, opacity) }
```

**Removed/deprecated:**
- Deprecate `PaintCommand::DrawBufferRect` solely to compose retained cached subtrees.
- Deprecate `PaintContext` for retained layer composition.

**Parallelizable within phase:** Partially — renderer direct-composite APIs + fast-path predicate tests independent; `render_paint_path` branch + `LayerStore` traversal tightly coupled.

**Test gate:**
- Existing repaint-boundary/scroll/damage tests pass.
- Add `warm_scroll_fast_path_skips_paint_context_new`.
- Add `warm_scroll_fast_path_skips_walk_and_paint`.
- Add `warm_scroll_fast_path_skips_boundary_rebuild`.
- Add `retained_composite_blits_boundary_with_accumulated_scroll_offset` (visible pixels move correctly).
- Add `retained_composite_respects_ancestor_clip`.
- Add `retained_composite_preserves_present_damage`.
- Add `warm_scroll_without_dynamic_overlay_allocates_zero_after_warmup` (exact zero).
- Run `cargo test -p scarlet-ui-core --lib retained_composite warm_scroll repaint_boundary present_damage`.

**Allocation gate:** `≤ Phase 5`. Scrollbar-disabled retained-composite scenario = **exactly zero** after warm-up. Default dynamic-scrollbar scenario may stay blocked until Phase 7 but must not exceed Phase 5.

**Risk:** Fast-path predicate too broad → skips necessary repaint (dynamic overlays, selection overlays, background/scale/root changes, dirty layers needing rebuild).

---

### Phase 7: overlay-handling
**Scope:** Resolve dynamic overlay/scrollbar as the final composite-only blocker. Implement flush protocol `[chunk-A] child-layer [chunk-B] child-layer [chunk-C + overlay]`; make **default** warm `ScrollView` scrolling zero-allocation.

**Files:**
- `crates/scarlet-ui-core/src/pipeline/rendering.rs` — split owner paint into ordered chunks around child boundary refs + overlay emission; overlay chunks rebuild allocation-free OR represented as retained primitives.
- `crates/scarlet-ui-core/src/pipeline/layers.rs` — dynamic chunk metadata / retained primitive metadata for overlays.
- `crates/scarlet-ui-core/src/views/scroll.rs` — convert `paint_overlay` (L910-928) into allocation-free overlay chunk rebuild OR retained scrollbar primitive data.
- `crates/scarlet-ui-core/src/element/render.rs` — optional render-object overlay classification API.
- `crates/scarlet-ui-core/src/renderer.rs` — non-allocating direct primitive draw/composite for retained scrollbar rects; rounded-rect overlay drawing must use Phase 2 scratch, NOT `path_rounded_rect` allocation.

**New types/functions:**
```rust
enum LayerChunkKind { StaticPicture, DynamicOverlay }
enum RetainedPrimitive { RoundedRect { rect, radius, color } }
struct OverlayScratch { primitives: Vec<RetainedPrimitive> }
impl LayerStore { mark_dynamic_overlay_chunk(id); dynamic_overlay_chunk_for_owner(owner); set_retained_primitives(id, primitives) }
trait RenderObject { fn retained_overlay_primitives(&self, origin, out: &mut Vec<RetainedPrimitive>) -> bool }
impl RenderingPipeline { rebuild_dynamic_overlay_allocation_free(owner, origin); composite_retained_primitive(primitive, state) }
impl CpuPaintRenderer { fill_rounded_rect_direct(rect, radius, color, clip) }
```

**Removed/deprecated:**
- Deprecate dynamic scrollbar rendering through per-frame `PaintContext` on warm scroll.
- Remove default fast-path blocking for `ScrollView` scrollbar once overlay retained/allocation-free.
- Permanently replace any remaining `scroll_view_auto_boundary_skips_nested_boundaries`.

**Parallelizable within phase:** Partially — retained scrollbar primitive extraction + overlay damage tests split; final ordering/composite integration serial.

**Test gate:**
- Keep `scroll_view_updates_lazy_vstack_viewport_hint`, `present_damage_*`, updated `repaint_boundary_cache_*`, `nested_repaint_boundary_dirty_descendant_invalidates_inner_only`, `repaint_boundary_cache_respects_ancestor_clip`.
- Add `scrollbar_overlay_retained_composite_updates_position_on_scroll` (thumb moves on warm scroll without `walk_and_paint`).
- Add `scrollbar_overlay_damage_includes_old_and_new_thumb`.
- Add `overlay_chunk_order_is_after_child_layers`.
- Add `default_scroll_view_warm_scroll_allocates_zero_after_warmup` (**exact zero**).
- Add `default_scroll_view_warm_scroll_skips_paint_context_and_walk` (exact zero counts).
- Run `cargo test -p scarlet-ui-core --lib warm_scroll overlay retained_composite repaint_boundary present_damage`.

**Allocation gate (FINAL HARD GATE):** Default warm steady-state scrolling = **exactly zero** heap allocations after warm-up; `≤ Phase 6`.

**Risk:** Rebuilding overlay chunks via `PaintContext::fill_rounded_rect` silently reintroduces `path_rounded_rect` allocation → must use retained primitives OR scratch-backed non-allocating overlay recorder.

---

## Cross-cutting Concerns

- **`PaintBuffer<'a>` lifetime migration:** `Borrowed(&'a Buffer)` stays valid only inside transient `PaintContext<'a>` during rebuild/cold paint. `LayerStore`, retained chunks, retained primitives, composite traversal must **never** store `Borrowed`. Retained picture chunks own `Buffer` directly (or `Arc<Buffer>` only if concrete sharing need). `PaintContext<'a>` is dropped before retained composite frames.
- **Long-lived `CpuPaintRenderer`:** continues at `RenderingPipeline.paint_renderer: Option<CpuPaintRenderer>`. Phase 2 external-buffer method reuses this renderer's scratch for offscreen chunk rebuilds, then restores window buffer. No static `execute_into_buffer` creating fresh renderer after Phase 2.
- **Reentrancy guard:** all rebuild/composite runs synchronously inside `render_paint_path`; renderer must not call back into pipeline render. Add `paint_renderer_busy: bool` debug/test guard to reject nested `execute_into_external_buffer`.
- **ElementTree non-allocating path-fill:** add `find_path_ids_into` + nearest-boundary helpers in `element/tree.rs`. Paint invalidation must use caller-owned scratch, not allocation-returning `find_path_ids` on the frame path.
- **Retained damage scratch:** `DirtyScratch` on `RenderingPipeline` for dirty IDs / path IDs / logical rects / physical damage. `.clear()` allowed; new `Vec`s on measured frame not.
- **Clip on `LayerChild`:** every `LayerChild` carries the clip that applied when flushed. Composite fast path accumulates parent clip ∩ child clip without emitting `PushClip`/`PopClip` commands.
- **Dynamic overlays must not block final fast path:** scrollbars in `views/scroll.rs` must become retained primitive data OR allocation-free dynamic overlay chunk rebuild.
- **`cache_nested_boundaries(false)` semantic change:** redefine as "do not flatten descendant boundaries into this boundary's picture chunk" OR no-op. Never "discard parent retained layer".
- **Atomic commit strategy:** one commit per phase when gate passes. Suggested sequence: `test: add warm scroll allocation baseline` → `refactor: reuse paint renderer scratch` → `feat: introduce retained layer store` → `feat: retain nested repaint boundaries` → `refactor: invalidate nearest retained layer` → `feat: composite retained warm scroll path` → `feat: retain scroll overlay without allocations`.

## Parallelization Map

**Strictly sequential (hard dependencies):**
1 → 2 → (3) → 4 → 5 → 6 → 7
(Phase 3 can be interleaved with 2's tail, but 3 before 4; 2 before 4; 4 before 5; 5 before 6; 6 before 7.)

**Parallelizable WITHIN phases:**
- P1: allocation harness ‖ behavior-baseline tests ‖ paint-path counters
- P2: renderer scratch migration ‖ pipeline external-buffer callsite migration
- P3: `LayerStore` data-structure tests ‖ lifecycle clearing in `RenderingPipeline`
- P4: `cache_nested_boundaries(false)` docs/test replacement ‖ retained chunk ordering tests (the `rendering.rs` rebuild path stays one implementer)
- P5: `ElementTree::find_path_ids_into` ‖ `DirtyScratch`/damage plumbing
- P6: direct renderer composite methods ‖ fast-path predicate tests (integration serial)
- P7: retained scrollbar primitive extraction ‖ overlay damage tests (final ordering/composite integration serial)

## Open Questions for the Implementer
1. Allocation counter scope: default `std` tests only, or also `legacy-scarlet-std`? **First target: default `std` unit tests.**
2. Scrollbar: retained primitives vs allocation-free dynamic overlay chunks? Retained primitives likely simpler for zero-alloc; spike must confirm rounded clipping + damage preserved.
3. Syncing scroll offset transforms into `LayerStore` without walking element tree on fast path: non-allocating post-flush sync, OR record scroll transform metadata when `apply_scroll_offset` changes child position?
4. Remove `PaintCache` entirely in Phase 4, or keep behind tests until Phase 6? Preferred: remove once `LayerStore` owns boundary buffers; keeping temporarily reduces rollout risk.
5. Do `paint_select_overlays` (dropdown/select overlays) need the same retained-overlay treatment before declaring the fast path generally enabled? Final zero-alloc gate must at least cover steady-state `ScrollView` scrolling; select-overlay generalization can be a follow-up.

## Verification Commands (per phase)
```bash
cargo test -p scarlet-ui-core --lib
cargo test -p scarlet-ui --lib --tests
SCARLET_UI_REPAINT_LOG=1 cargo test -p scarlet-ui-core scroll_view_auto_boundary_retains_nested_boundaries -- --nocapture
```
