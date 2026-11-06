//! Backend-independent paint footprints, revision differences and reconstruction.
//! Notifications, immutable recording identities and raster tiles are separate.
//! Read dependencies refer only to pixels produced earlier in paint order.
use super::*;

/// A conservative region in logical coordinates. Unknown is deliberately not empty.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PaintRegion {
    Empty,
    Rect(Rect),
    Unknown,
}

/// Output coverage and the preceding scene pixels an operation samples.
/// Ordinary source-over paint has no nonlocal reads: unchanged translucent
/// content can be retained, but must still be recomposited over dirty pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintDependencies {
    pub writes: PaintRegion,
    pub reads: PaintRegion,
}
impl PaintDependencies {
    pub const UNKNOWN: Self = Self {
        writes: PaintRegion::Unknown,
        reads: PaintRegion::Unknown,
    };
    pub const fn new(writes: PaintRegion, reads: PaintRegion) -> Self {
        Self { writes, reads }
    }
    pub(crate) fn translated(self, offset: Point) -> Self {
        let shift = |r| match r {
            PaintRegion::Rect(r) => PaintRegion::Rect(Rect::new(
                Point::new(r.left() + offset.x, r.top() + offset.y),
                r.size,
            )),
            other => other,
        };
        Self::new(shift(self.writes), shift(self.reads))
    }
}
fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let (x, y) = (a.left().max(b.left()), a.top().max(b.top()));
    let (r, b) = (a.right().min(b.right()), a.bottom().min(b.bottom()));
    (r > x && b > y).then(|| Rect::from_xywh(x, y, r - x, b - y))
}
fn union(a: Rect, b: Rect) -> Rect {
    let (x, y) = (a.left().min(b.left()), a.top().min(b.top()));
    Rect::from_xywh(
        x,
        y,
        a.right().max(b.right()) - x,
        a.bottom().max(b.bottom()) - y,
    )
}
fn expand(r: Rect, pad: f32) -> Rect {
    Rect::from_xywh(
        r.left() - pad,
        r.top() - pad,
        r.size.width + pad * 2.,
        r.size.height + pad * 2.,
    )
}
fn valid(r: Rect) -> bool {
    [r.left(), r.top(), r.right(), r.bottom()]
        .iter()
        .all(|v| v.is_finite())
        && r.size.width >= 0.
        && r.size.height >= 0.
}
fn add(regions: &mut Vec<Rect>, rect: Rect, viewport: Rect) {
    let Some(mut rect) = intersect(rect, viewport) else {
        return;
    };
    let mut i = 0;
    while i < regions.len() {
        if intersect(regions[i], rect).is_some() {
            rect = union(rect, regions.remove(i));
            i = 0;
        } else {
            i += 1;
        }
    }
    if rect.size.width > 0. && rect.size.height > 0. {
        regions.push(rect);
    }
}
fn hits(region: PaintRegion, dirty: &[Rect]) -> bool {
    match region {
        PaintRegion::Unknown => !dirty.is_empty(),
        PaintRegion::Empty => false,
        PaintRegion::Rect(r) => dirty.iter().any(|d| intersect(*d, r).is_some()),
    }
}
fn add_region(dirty: &mut Vec<Rect>, region: PaintRegion, viewport: Rect) {
    match region {
        PaintRegion::Empty => {}
        PaintRegion::Rect(r) if valid(r) => add(dirty, r, viewport),
        _ => {
            dirty.clear();
            dirty.push(viewport);
        }
    }
}
fn command_dependencies(c: &PaintCommand, ctx: &PaintContext<'_>, scale: u32) -> PaintDependencies {
    match c {
        PaintCommand::BackdropFilterRoundedRect {
            rect, blur_radius, ..
        } => {
            let factor = scale.max(1) as f32 / 1000.;
            if !valid(*rect) || !blur_radius.is_finite() {
                return PaintDependencies::UNKNOWN;
            }
            // Conservative union of CPU/GPU kernel support, reconstruction taps
            // and physical-pixel rounding. Refraction samples inward.
            let radius = (blur_radius.max(0.) * factor).min(64.);
            let halo = (libm::ceilf(radius + (radius / 4.).max(1.)) + 2.) / factor;
            PaintDependencies::new(
                PaintRegion::Rect(expand(*rect, 1. / factor)),
                PaintRegion::Rect(expand(*rect, halo)),
            )
        }
        PaintCommand::Extension { dependencies, .. } => *dependencies,
        PaintCommand::PushClip { .. } | PaintCommand::PopClip | PaintCommand::SetOpacity { .. } => {
            PaintDependencies::new(PaintRegion::Empty, PaintRegion::Empty)
        }
        _ => PaintDependencies::new(
            command_paint_bounds(c, ctx, scale)
                .map_or(PaintRegion::Empty, |r| PaintRegion::Rect(expand(r, 2.))),
            PaintRegion::Empty,
        ),
    }
}

fn visit(
    ctx: &PaintContext<'_>,
    origin: Point,
    clips: &mut Vec<ClipRegion>,
    scale: u32,
    out: &mut Vec<PaintDependencies>,
) {
    for c in ctx.commands() {
        match c {
            PaintCommand::DrawDisplayList { origin: p, list } => {
                let p = Point::new(origin.x + p.x, origin.y + p.y);
                if list.self_contained {
                    for dep in list.cached_dependencies(scale).iter() {
                        push_clipped(dep.translated(p), clips, out);
                    }
                } else {
                    visit(list.context(), p, clips, scale, out);
                }
            }
            PaintCommand::PushClip {
                rect,
                corner_radius,
            } => clips.push(ClipRegion {
                rect: Rect::new(
                    Point::new(origin.x + rect.left(), origin.y + rect.top()),
                    rect.size,
                ),
                corner_radius: *corner_radius,
            }),
            PaintCommand::PopClip => {
                clips.pop();
            }
            _ => {
                push_clipped(
                    command_dependencies(c, ctx, scale).translated(origin),
                    clips,
                    out,
                );
            }
        }
    }
}

fn push_clipped(
    mut dep: PaintDependencies,
    clips: &[ClipRegion],
    out: &mut Vec<PaintDependencies>,
) {
    for clip in clips {
        if let PaintRegion::Rect(bounds) = dep.writes {
            dep.writes = intersect(bounds, clip.rect).map_or(PaintRegion::Empty, PaintRegion::Rect);
        }
    }
    if dep.writes != PaintRegion::Empty {
        out.push(dep);
    }
}

impl DisplayList {
    // Immutable records can reuse glyph/geometry footprints across revisions of
    // their parent. Bound scale variants so moving a window cannot grow the cache.
    fn cached_dependencies(&self, scale: u32) -> Arc<[PaintDependencies]> {
        if let Some((_, deps)) = self.footprints.borrow().iter().find(|(s, _)| *s == scale) {
            return deps.clone();
        }
        let deps: Arc<[PaintDependencies]> = self.context.paint_dependencies(scale).into();
        let mut cache = self.footprints.borrow_mut();
        if cache.len() == 2 {
            cache.remove(0);
        }
        cache.push((scale, deps.clone()));
        deps
    }
}

impl PaintContext<'_> {
    /// Ordered conservative footprints. Backends may use these for invalidation,
    /// cache eligibility and clean-prefix reconstruction without knowing widgets.
    pub fn paint_dependencies(&self, scale_milli: u32) -> Vec<PaintDependencies> {
        let mut out = Vec::new();
        visit(self, Point::ZERO, &mut Vec::new(), scale_milli, &mut out);
        out
    }
    /// Propagate changed pixels to subsequent operations that sample them.
    /// This is output damage; reconstruction may require additional clean input.
    pub fn dependent_damage(&self, damage: &[Rect], viewport: Rect, scale: u32) -> Vec<Rect> {
        let mut out = Vec::new();
        for r in damage {
            add_region(&mut out, PaintRegion::Rect(*r), viewport);
        }
        for dep in self.paint_dependencies(scale) {
            if hits(dep.reads, &out) {
                add_region(&mut out, dep.writes, viewport);
            }
        }
        out
    }
    /// Regions that can be cleared and replayed without sampling last frame's
    /// foreground. Close over both input and output of every intersected reader.
    /// Keep disconnected regions separate; never bridge them with one global box.
    pub fn reconstruction_regions(&self, damage: &[Rect], viewport: Rect, scale: u32) -> Vec<Rect> {
        let factor = scale.max(1) as f32 / 1000.;
        let aligned = |region| match region {
            PaintRegion::Rect(r) if valid(r) => {
                let x = libm::floorf(r.left() * factor) / factor;
                let y = libm::floorf(r.top() * factor) / factor;
                PaintRegion::Rect(Rect::from_xywh(
                    x,
                    y,
                    libm::ceilf(r.right() * factor) / factor - x,
                    libm::ceilf(r.bottom() * factor) / factor - y,
                ))
            }
            PaintRegion::Rect(_) => PaintRegion::Unknown,
            other => other,
        };
        // Round before closure: outward pixel rounding can reach another reader.
        let deps = self
            .paint_dependencies(scale)
            .into_iter()
            .map(|d| PaintDependencies::new(aligned(d.writes), aligned(d.reads)))
            .collect::<Vec<_>>();
        let mut out = Vec::new();
        for r in damage {
            add_region(&mut out, aligned(PaintRegion::Rect(*r)), viewport);
        }
        let mut included = alloc::vec![false;deps.len()];
        loop {
            let mut changed = false;
            for (i, dep) in deps.iter().enumerate() {
                if included[i] || dep.reads == PaintRegion::Empty {
                    continue;
                }
                if hits(dep.writes, &out) || hits(dep.reads, &out) {
                    match (dep.reads, dep.writes) {
                        (PaintRegion::Rect(input), PaintRegion::Rect(output)) => {
                            add_region(&mut out, PaintRegion::Rect(union(input, output)), viewport)
                        }
                        _ => {
                            add_region(&mut out, dep.reads, viewport);
                            add_region(&mut out, dep.writes, viewport);
                        }
                    }
                    included[i] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        out
    }
}

#[derive(PartialEq)]
enum AtomKey {
    List(u64),
    Command(PaintCommand),
    Buffer(PaintCommand, u64, u64),
}
struct Atom {
    key: AtomKey,
    origin: Point,
    clips: Vec<ClipRegion>,
    writes: PaintRegion,
}
fn atoms(
    ctx: &PaintContext<'_>,
    origin: Point,
    clips: &mut Vec<ClipRegion>,
    scale: u32,
    out: &mut Vec<Atom>,
) {
    for c in ctx.commands() {
        match c {
            PaintCommand::PushClip {
                rect,
                corner_radius,
            } => clips.push(ClipRegion {
                rect: Rect::new(
                    Point::new(origin.x + rect.left(), origin.y + rect.top()),
                    rect.size,
                ),
                corner_radius: *corner_radius,
            }),
            PaintCommand::PopClip => {
                clips.pop();
            }
            PaintCommand::DrawDisplayList { origin: p, list } => {
                let p = Point::new(origin.x + p.x, origin.y + p.y);
                if list
                    .context()
                    .commands()
                    .iter()
                    .any(|c| matches!(c, PaintCommand::DrawDisplayList { .. }))
                {
                    atoms(list.context(), p, clips, scale, out);
                } else {
                    let mut deps = Vec::new();
                    if list.self_contained {
                        for dep in list.cached_dependencies(scale).iter() {
                            push_clipped(dep.translated(p), clips, &mut deps);
                        }
                    } else {
                        visit(list.context(), p, &mut clips.clone(), scale, &mut deps);
                    }
                    let mut writes = PaintRegion::Empty;
                    for dep in deps {
                        writes = match (writes, dep.writes) {
                            (PaintRegion::Unknown, _) | (_, PaintRegion::Unknown) => {
                                PaintRegion::Unknown
                            }
                            (PaintRegion::Rect(a), PaintRegion::Rect(b)) => {
                                PaintRegion::Rect(union(a, b))
                            }
                            (PaintRegion::Empty, b) => b,
                            (a, PaintRegion::Empty) => a,
                        };
                    }
                    out.push(Atom {
                        key: AtomKey::List(list.identity()),
                        origin: p,
                        clips: clips.clone(),
                        writes,
                    });
                }
            }
            PaintCommand::SetOpacity { .. } => out.push(Atom {
                key: AtomKey::Command(c.clone()),
                origin,
                clips: clips.clone(),
                writes: PaintRegion::Unknown,
            }),
            _ => {
                // Keep the original buffer table when computing legacy image bounds.
                let mut dep = command_dependencies(c, ctx, scale).translated(origin);
                for clip in clips.iter() {
                    if let PaintRegion::Rect(r) = dep.writes {
                        dep.writes =
                            intersect(r, clip.rect).map_or(PaintRegion::Empty, PaintRegion::Rect);
                    }
                }
                out.push(Atom {
                    key: match c {
                        PaintCommand::DrawBuffer { buffer_idx, .. }
                        | PaintCommand::DrawBufferRect { buffer_idx, .. } => {
                            match ctx.buffer(BufferHandle(*buffer_idx)) {
                                Some(buffer) => {
                                    AtomKey::Buffer(c.clone(), buffer.identity(), buffer.revision())
                                }
                                None => AtomKey::Command(c.clone()),
                            }
                        }
                        _ => AtomKey::Command(c.clone()),
                    },
                    origin,
                    clips: clips.clone(),
                    writes: dep.writes,
                });
            }
        }
    }
}
impl DisplayList {
    /// Local changes between immutable revisions, before dependency propagation.
    /// Unknown coverage returns None: callers must repaint the complete picture.
    pub fn damage_since(&self, previous: &DisplayList, scale: u32) -> Option<Vec<Rect>> {
        if self.identity() == previous.identity() {
            return Some(Vec::new());
        }
        // Cross-record opacity/clip state cannot be compared as independent
        // atoms. Keep the existing complete replay fallback for those streams.
        if !self.is_self_contained() || !previous.is_self_contained() {
            return None;
        }
        let mut old = Vec::new();
        let mut new = Vec::new();
        atoms(
            previous.context(),
            Point::ZERO,
            &mut Vec::new(),
            scale,
            &mut old,
        );
        atoms(
            self.context(),
            Point::ZERO,
            &mut Vec::new(),
            scale,
            &mut new,
        );
        let mut damage = Vec::new();
        for i in 0..old.len().max(new.len()) {
            if let (Some(a), Some(b)) = (old.get(i), new.get(i)) {
                if a.key == b.key
                    && a.origin == b.origin
                    && a.clips == b.clips
                    && !matches!(a.key, AtomKey::Command(PaintCommand::Extension { .. }))
                {
                    continue;
                }
            }
            for a in [old.get(i), new.get(i)].into_iter().flatten() {
                match a.writes {
                    PaintRegion::Unknown => return None,
                    PaintRegion::Rect(r) => damage.push(r),
                    PaintRegion::Empty => {}
                }
            }
        }
        Some(damage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_xywh(x, y, w, h)
    }
    fn effect(ctx: &mut PaintContext<'_>, reads: Rect, writes: Rect) {
        ctx.draw_extension_with_dependencies(
            writes,
            Arc::new(1u32),
            PaintDependencies::new(PaintRegion::Rect(writes), PaintRegion::Rect(reads)),
        );
    }
    #[test]
    fn buffer_resource_changes_are_not_hidden_by_equal_command_indices() {
        let record = |color| {
            let mut buffer = Buffer::new(Size::new(8., 8.));
            buffer.clear(color);
            let mut ctx = PaintContext::new();
            ctx.draw_buffer(r(10., 10., 8., 8.), buffer);
            ctx.into_display_list()
        };
        let a = record(Color::BLACK);
        let b = record(Color::WHITE);
        assert!(!b.damage_since(&a, 1000).unwrap().is_empty());
    }

    #[test]
    fn physical_rounding_connects_adjacent_reader_before_replay() {
        let mut ctx = PaintContext::new();
        effect(&mut ctx, r(10.2, 10., 5., 5.), r(30., 10., 5., 5.));
        let dirty =
            ctx.reconstruction_regions(&[r(9.7, 10., 0.4, 1.)], r(0., 0., 100., 100.), 1000);
        assert!(
            dirty
                .iter()
                .any(|d| intersect(*d, r(30., 10., 5., 5.)).is_some())
        );
    }

    #[test]
    fn generic_read_edges_propagate_in_order_and_keep_independent_regions_separate() {
        let viewport = r(0., 0., 500., 500.);
        let mut ctx = PaintContext::new();
        effect(&mut ctx, r(10., 10., 20., 20.), r(80., 10., 20., 20.));
        effect(&mut ctx, r(80., 10., 20., 20.), r(150., 10., 20., 20.));
        effect(&mut ctx, r(400., 400., 20., 20.), r(450., 400., 20., 20.));
        let dirty = ctx.dependent_damage(&[r(12., 12., 2., 2.)], viewport, 2000);
        assert!(hits(PaintRegion::Rect(r(150., 10., 20., 20.)), &dirty));
        assert!(!hits(PaintRegion::Rect(r(450., 400., 20., 20.)), &dirty));
        let repair = ctx.reconstruction_regions(&dirty, viewport, 2000);
        assert_eq!(
            repair.len(),
            1,
            "one connected input/output dependency chain is reconstructed together"
        );
        assert!(!hits(PaintRegion::Rect(r(40., 40., 10., 10.)), &repair));
    }
    #[test]
    fn unknown_extensions_fall_back_but_ordinary_alpha_does_not_expand_damage() {
        let viewport = r(0., 0., 200., 200.);
        let dirty = r(4., 4., 2., 2.);
        let mut ctx = PaintContext::new();
        ctx.fill_rect(viewport, Color::WHITE.with_opacity(0.2));
        assert_eq!(
            ctx.reconstruction_regions(&[dirty], viewport, 1000),
            [dirty]
        );
        ctx.draw_extension(r(80., 80., 20., 20.), Arc::new(0u32));
        assert_eq!(
            ctx.reconstruction_regions(&[dirty], viewport, 1000),
            [viewport]
        );
    }
    #[test]
    fn blur_halo_and_overlapping_readers_expand_without_reaching_a_distant_effect() {
        let viewport = r(0., 0., 500., 200.);
        let mut ctx = PaintContext::new();
        ctx.backdrop_filter_rounded_rect(r(40., 40., 50., 50.), 10., 10., Color::WHITE);
        ctx.backdrop_filter_rounded_rect(r(90., 40., 50., 50.), 10., 10., Color::WHITE);
        ctx.backdrop_filter_rounded_rect(r(400., 40., 50., 50.), 10., 10., Color::WHITE);
        for scale in [1000, 2000, 3000] {
            let damage = ctx.dependent_damage(&[r(32., 50., 2., 2.)], viewport, scale);
            assert!(hits(PaintRegion::Rect(r(100., 50., 10., 10.)), &damage));
            let repair = ctx.reconstruction_regions(&damage, viewport, scale);
            assert!(repair.iter().all(|r| r.right() < 200.));
            assert!(repair.iter().any(|r| r.left() < 30. && r.right() > 150.));
        }
    }
    #[test]
    fn retained_revision_diff_preserves_static_children_and_reports_removal() {
        let mut static_paint = PaintContext::new();
        static_paint.fill_rect(r(10., 100., 20., 20.), Color::WHITE);
        let static_list = static_paint.into_display_list();
        let make = |color, previous: Option<&DisplayList>| {
            let mut changing = PaintContext::new();
            changing.fill_rect(r(10., 10., 20., 20.), color);
            let mut ctx = PaintContext::new();
            ctx.draw_display_list(Point::ZERO, changing.into_display_list());
            ctx.draw_display_list(Point::ZERO, static_list.clone());
            if let Some(old) = previous {
                ctx.into_cached_layer_revision(r(0., 0., 200., 200.), old)
            } else {
                ctx.into_cached_layer(r(0., 0., 200., 200.))
            }
        };
        let first = make(Color::WHITE, None);
        let next = make(Color::BLACK, Some(&first));
        assert_eq!(first.cache_identity(), next.cache_identity());
        assert_ne!(first.identity(), next.identity());
        assert!(
            next.damage_since(&first, 2000)
                .unwrap()
                .iter()
                .all(|r| r.bottom() < 40.)
        );
        let removed = PaintContext::new().into_cached_layer_revision(r(0., 0., 200., 200.), &next);
        assert!(
            removed
                .damage_since(&next, 2000)
                .unwrap()
                .iter()
                .any(|r| r.bottom() > 100.)
        );
    }
    #[test]
    fn extension_footprints_follow_translation_and_clipping() {
        let mut ctx = PaintContext::new();
        ctx.push_clip(r(10., 10., 20., 20.));
        effect(&mut ctx, r(-10., -10., 40., 40.), r(0., 0., 50., 50.));
        ctx.pop_clip();
        let mut moved = PaintContext::new();
        moved.append_retained_at(&ctx, Point::new(100., 100.));
        let deps = moved.paint_dependencies(1000);
        assert_eq!(deps[0].writes, PaintRegion::Rect(r(110., 110., 20., 20.)));
        assert_eq!(deps[0].reads, PaintRegion::Rect(r(90., 90., 40., 40.)));
    }
}
