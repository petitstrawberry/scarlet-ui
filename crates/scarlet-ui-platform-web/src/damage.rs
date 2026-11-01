//! Physical damage selection for the persistent WebGPU render target.

type DamageRect = (u32, u32, u32, u32);

/// Clamp explicit damage without widening it to a full-target repaint.
///
/// A retained paint list may omit unchanged widgets, so clearing anything
/// outside these rectangles would erase content that will not be repainted.
/// Only `None` requests the complete target. Empty explicit damage remains
/// empty, allowing the caller to reject it rather than silently clear content.
pub(crate) fn render_areas(
    target_width: u32,
    target_height: u32,
    physical_damage: Option<&[DamageRect]>,
) -> Vec<DamageRect> {
    match physical_damage {
        None => vec![(0, 0, target_width, target_height)],
        Some(damage) => damage
            .iter()
            .filter_map(|&(x, y, width, height)| {
                let right = x.saturating_add(width).min(target_width);
                let bottom = y.saturating_add(height).min(target_height);
                (right > x && bottom > y).then(|| (x, y, right - x, bottom - y))
            })
            .collect(),
    }
}
