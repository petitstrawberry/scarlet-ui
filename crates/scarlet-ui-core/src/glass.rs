//! Bounded screen-space glass optics shared by CPU and SGFX renderers.
//!
//! Technique references: kube.io/blog/liquid-glass-css-svg/ (convex bezel,
//! Snell displacement) and iyinchao/liquid-glass-studio (directional rim).
//! This is an independent approximation, not Apple's optical implementation.
use crate::geometry::{Point, Rect};
use core::sync::atomic::{AtomicU8, Ordering};

/// Application-wide quality ceiling for built-in glass materials.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum GlassQuality {
    /// Translucent tint only: no backdrop copy, blur, or refraction.
    TintOnly = 0,
    /// Bounded backdrop blur with a uniform tint.
    Frosted = 1,
    /// Bounded blur plus convex edge refraction and a directional rim.
    #[default]
    Refractive = 2,
}
static QUALITY: AtomicU8 = AtomicU8::new(GlassQuality::Refractive as u8);

/// Set the material quality ceiling. Request a scene redraw after changing it.
/// Existing resource-budget fallbacks still apply independently of this choice.
pub fn set_glass_quality(quality: GlassQuality) {
    QUALITY.store(quality as u8, Ordering::Relaxed);
}
pub fn glass_quality() -> GlassQuality {
    match QUALITY.load(Ordering::Relaxed) {
        0 => GlassQuality::TintOnly,
        1 => GlassQuality::Frosted,
        _ => GlassQuality::Refractive,
    }
}

/// Inward unit normal and distance to a rounded rectangle's boundary.
/// Coordinates must be finite; radius is clamped to the rectangle.
#[doc(hidden)]
pub fn glass_edge(rect: Rect, radius: f32, p: Point) -> (f32, f32, f32) {
    let hx = rect.size.width * 0.5;
    let hy = rect.size.height * 0.5;
    let r = radius.max(0.0).min(hx).min(hy);
    let x = p.x - rect.origin.x - hx;
    let y = p.y - rect.origin.y - hy;
    let qx = x.abs() - hx + r;
    let qy = y.abs() - hy + r;
    let sx = if x < 0.0 { -1.0 } else { 1.0 };
    let sy = if y < 0.0 { -1.0 } else { 1.0 };
    if qx > 0.0 && qy > 0.0 {
        let length = libm::sqrtf(qx * qx + qy * qy);
        (r - length, -sx * qx / length, -sy * qy / length)
    } else if qx > qy {
        (r - qx, -sx, 0.0)
    } else {
        (r - qy, 0.0, -sy)
    }
}

/// Refracted background position. Flat centers remain unchanged. The bounded
/// convex profile bends samples inward, never enlarging the capture region.
#[doc(hidden)]
pub fn glass_sample(rect: Rect, radius: f32, p: Point, strength: f32) -> Point {
    let strength = strength.clamp(0.0, 1.0);
    let bezel = radius
        .min(12.0)
        .min(rect.size.width * 0.25)
        .min(rect.size.height * 0.25)
        .max(0.0);
    if strength == 0.0 || bezel <= 0.0 {
        return p;
    }
    let (distance, nx, ny) = glass_edge(rect, radius, p);
    if distance >= bezel || distance < 0.0 {
        return p;
    }
    // Normal incidence at the flat center, grazing incidence at the rim.
    // Snell's law, n_air/n_glass = 1/1.5; no ray marching or dispersion.
    let t = (1.0 - distance / bezel).clamp(0.0, 0.98);
    let sin_i = t * t;
    let cos_i = libm::sqrtf(1.0 - sin_i * sin_i);
    let sin_t = sin_i / 1.5;
    let cos_t = libm::sqrtf(1.0 - sin_t * sin_t);
    let tangent = (sin_i * cos_t - cos_i * sin_t) / (cos_i * cos_t + sin_i * sin_t);
    let shift = (tangent * bezel * 1.65).min(bezel * 1.4) * strength;
    Point::new(p.x + nx * shift, p.y + ny * shift)
}

/// Signed reflection opacity: positive is white light, negative is a subtle
/// dark reflection. Both die away within 1.25 logical pixels of the edge.
#[doc(hidden)]
pub fn glass_reflection(distance: f32, inward_x: f32, inward_y: f32) -> f32 {
    let band = (1.0 - ((distance - 0.55) / 0.7).abs()).max(0.0);
    let light = (inward_x * 0.6 + inward_y * 0.8).clamp(-1.0, 1.0);
    band * if light >= 0.0 {
        0.30 * light
    } else {
        0.065 * light
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn convex_samples_stay_bounded_and_leave_center_undisturbed() {
        for (w, h) in [(100.0, 36.0), (180.0, 600.0), (4.0, 4.0)] {
            let r = Rect::from_xywh(13.0, 17.0, w, h);
            let center = Point::new(13.0 + w / 2.0, 17.0 + h / 2.0);
            assert_eq!(glass_sample(r, 12.0, center, 1.0), center);
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let p = Point::new(13.0 + x as f32 + 0.5, 17.0 + y as f32 + 0.5);
                    let s = glass_sample(r, 12.0, p, 1.0);
                    assert!(
                        s.x >= r.left() && s.x <= r.right() && s.y >= r.top() && s.y <= r.bottom()
                    );
                    assert_eq!(glass_sample(r, 12.0, p, 0.0), p);
                }
            }
        }
        let r = Rect::from_xywh(0.0, 0.0, 100.0, 36.0);
        assert!(glass_sample(r, 12.0, Point::new(50.0, 1.0), 1.0).y > 1.0);
    }
}
