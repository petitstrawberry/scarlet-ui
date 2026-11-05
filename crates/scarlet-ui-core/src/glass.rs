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

/// Width of the curved shoulder, independent of optical thickness.
#[doc(hidden)]
pub fn glass_bezel(rect: Rect, radius: f32) -> f32 {
    (radius * 1.5)
        .clamp(16.0, 24.0)
        .min(rect.size.width * 0.35)
        .min(rect.size.height * 0.35)
        .max(0.0)
}

/// Sample the broader curved edge while keeping optical thickness bounded.
#[doc(hidden)]
pub fn glass_sample(rect: Rect, radius: f32, p: Point, strength: f32) -> Point {
    let strength = strength.clamp(0.0, 1.0);
    let bezel = glass_bezel(rect, radius);
    if strength == 0.0 || bezel <= 0.0 {
        return p;
    }
    let (distance, nx, ny) = glass_edge(rect, radius, p);
    // Rounded-contour vertices can land a few ulps outside the boundary.
    // Treat these as grazing incidence rather than discontinuously losing the lens.
    if distance >= bezel || distance < -0.001 {
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
    // Widen the transition, not the maximum displacement or capture halo.
    let thickness = bezel.min(12.0);
    let shift = (tangent * thickness * 1.65).min(thickness * 1.4) * strength;
    Point::new(p.x + nx * shift, p.y + ny * shift)
}

/// Compatibility helper for the default curved shoulder.
#[doc(hidden)]
pub fn glass_reflection(distance: f32, inward_x: f32, inward_y: f32) -> f32 {
    glass_surface_reflection(distance, inward_x, inward_y, 16.)
}

/// Width and strength of the narrow boundary reflection. Both vary around the
/// contour: the key-lit corner is broader/brighter, the opposing return weaker,
/// and the sides between them nearly disappear. Width is in logical pixels.
#[doc(hidden)]
pub fn glass_highlight_edge(inward_x: f32, inward_y: f32) -> (f32, f32) {
    let facing = (inward_x * 0.6 + inward_y * 0.8).clamp(-1., 1.);
    let key2 = facing.max(0.) * facing.max(0.);
    let fill2 = (-facing).max(0.) * (-facing).max(0.);
    let key = key2 * key2;
    let fill = fill2 * fill2;
    (
        0.35 + 0.80 * key + 0.35 * fill,
        0.04 + 0.74 * key + 0.40 * fill,
    )
}

/// A narrow directional boundary reflection and a separate curved highlight.
/// The normal tilts toward the viewer across the same shoulder as refraction.
/// This moves and broadens the specular lobe across the surface instead of
/// painting a uniformly bright outline. A weaker opposing light and grazing
/// Fresnel reflection retain the return highlight seen on transparent glass.
#[doc(hidden)]
pub fn glass_surface_reflection(distance: f32, inward_x: f32, inward_y: f32, bezel: f32) -> f32 {
    if distance < 0. || bezel <= 0. || distance >= bezel {
        return 0.;
    }
    let t = (1. - distance / bezel).clamp(0., 1.);
    let normal_xy = t * t;
    let normal_z = libm::sqrtf((1. - normal_xy * normal_xy).max(0.));
    let direction = (inward_x * 0.6 + inward_y * 0.8).clamp(-1., 1.);
    let lobe = |cosine: f32| {
        let x = cosine.clamp(0., 1.);
        let x2 = x * x;
        let x4 = x2 * x2;
        let x8 = x4 * x4;
        x8 * x8
    };
    let key = lobe(direction * normal_xy * 0.85 + normal_z * 0.5267827);
    let fill = lobe(-direction * normal_xy * 0.94 + normal_z * 0.34117445);
    let grazing = 1. - normal_z;
    let grazing2 = grazing * grazing;
    let fresnel = grazing2 * grazing2 * grazing;
    let surface = 0.06 * fresnel + (0.46 * key + 0.22 * fill) * normal_xy;
    // The silhouette is its own narrow white edge, not the blurred tail of
    // the specular lobe. Let raster coverage antialias its outside boundary.
    let (rim_width, rim_strength) = glass_highlight_edge(inward_x, inward_y);
    let rim_width = rim_width.min(bezel * 0.25);
    let rim = rim_strength * ((rim_width - distance) / (rim_width * 0.6)).clamp(0., 1.);
    rim + (1. - rim) * surface
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn light_reflects_across_the_curved_surface_not_only_the_outline() {
        let bezel = 20.;
        let reflection = |d, x, y| glass_surface_reflection(d, x, y, bezel);
        let (key_width, key_strength) = glass_highlight_edge(0.6, 0.8);
        let (side_width, side_strength) = glass_highlight_edge(-0.8, 0.6);
        assert!(key_width > side_width * 2. && key_width < 1.25);
        assert!(key_strength > side_strength * 5.);
        assert!(reflection(0.2, 0.6, 0.8) > 0.50);
        assert!(reflection(1.5, 0.6, 0.8) > reflection(1.5, -0.8, 0.6));
        assert!(reflection(1., -0.6, -0.8) > 0.10);
        assert_eq!(reflection(bezel, 0., 1.), 0.);
        assert_eq!(reflection(-1., 0., 1.), 0.);
        for i in 0..100 {
            let value = reflection(i as f32 * bezel / 100., 0., 1.);
            assert!((0. ..=1.).contains(&value));
        }
    }
    #[test]
    fn curvature_extends_past_small_corners_without_increasing_displacement_limit() {
        let rect = Rect::from_xywh(0., 0., 240., 100.);
        let inner_edge = Point::new(120., 14.);
        assert!(glass_sample(rect, 8., inner_edge, 1.).y > inner_edge.y);
        for radius in [0., 8., 28.] {
            for y in 0..50 {
                let p = Point::new(120., y as f32 + 0.5);
                let sample = glass_sample(rect, radius, p, 1.);
                assert!((sample.y - p.y).abs() <= 12. * 1.4);
                assert_eq!(sample.x, p.x);
            }
        }
    }

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
