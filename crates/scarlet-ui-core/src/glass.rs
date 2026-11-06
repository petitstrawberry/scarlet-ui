//! Bounded screen-space glass optics shared by CPU and SGFX renderers.
//!
//! Technique references: kube.io/blog/liquid-glass-css-svg/ (convex bezel,
//! Snell displacement) and iyinchao/liquid-glass-studio (directional rim).
//! This is an independent approximation, not Apple's optical implementation.
use crate::geometry::{Point, Rect};
use core::sync::atomic::{AtomicU8, Ordering};

// Contour review C: strengthen the separated shadow by 50 percent.
const CONTOUR_SHADOW_GAIN: f32 = 1.5;

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

/// Legacy narrow-boundary parameters, retained for source compatibility.
/// Built-in materials use only `glass_surface_reflection`, without this outline.
/// Both values vary around the
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

/// Width and shading opacity of the compact contour, in logical pixels.
/// Side-facing normals carry a wider dark edge; horizontal rims stay finer.
#[doc(hidden)]
pub fn glass_contour_edge(inward_x: f32, inward_y: f32) -> (f32, f32) {
    let side = inward_x * inward_x;
    // Reach the first interior sample even when a 1x edge falls on a pixel
    // center, while keeping both horizontal rims finer than the side contour.
    (
        1.25 + 0.50 * side,
        0.05 + 0.13 * side + 0.025 * (-inward_y).max(0.),
    )
}

/// Contour widths are rounded in device pixels before the normal interpolates
/// them around each corner. Both cardinal widths survive fractional scaling.
#[doc(hidden)]
pub fn glass_contour_width(inward_x: f32, scale: f32) -> f32 {
    let scale = scale.max(0.001);
    let top = libm::roundf(scale * 0.5).max(1.);
    let side = libm::roundf(scale).max(top + 1.);
    (top + (side - top) * inward_x * inward_x) / scale
}

/// A soft directional reflection spread across the curved shoulder.
/// The normal tilts toward the viewer across the same shoulder as refraction.
/// This moves and broadens the specular lobe across the surface instead of
/// painting a broad bright outline. Horizontal shoulders receive soft light
/// and a finer bright rim; side-facing contours carry compact shading.
/// Negative values shade toward black; positive values reflect toward white.
/// This screen-space approximation
/// follows the reference appearance without claiming its optical mechanism.
#[doc(hidden)]
pub fn glass_surface_reflection(distance: f32, inward_x: f32, inward_y: f32, bezel: f32) -> f32 {
    glass_surface_reflection_at_scale(distance, inward_x, inward_y, bezel, 1.)
}

/// Keep the shadow line distinct from the shoulder's blurred reflection.
#[doc(hidden)]
pub fn glass_surface_reflection_at_scale(
    distance: f32,
    inward_x: f32,
    inward_y: f32,
    bezel: f32,
    scale: f32,
) -> f32 {
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
        x8
    };
    let key = lobe(direction * normal_xy * 0.85 + normal_z * 0.5267827);
    let fill = lobe(-direction * normal_xy * 0.94 + normal_z * 0.34117445);
    let grazing = 1. - normal_z;
    let grazing2 = grazing * grazing;
    let fresnel = grazing2 * grazing2 * grazing;
    let horizontal = inward_y * inward_y;
    let surface = (0.012 * fresnel + (0.16 * key + 0.06 * fill) * normal_xy) * horizontal;
    let (width, opacity) = glass_contour_edge(inward_x, inward_y);
    let width = width.min(bezel * 0.10);
    let fade = (1. - distance / width).clamp(0., 1.);
    let light = (0.34 - 0.06 * (-inward_y).max(0.)) * horizontal;
    let shoulder = surface + light * fade * fade * (3. - 2. * fade);
    let scale = scale.max(0.001);
    let line_width = glass_contour_width(inward_x, scale).min(bezel);
    // Only the last physical pixel is antialiased. Do not diffuse the shadow
    // across the whole band or let shoulder light wash over the line.
    let line = ((line_width - distance) * scale + 0.5).clamp(0., 1.);
    shoulder * (1. - line) - opacity * CONTOUR_SHADOW_GAIN * line
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn light_reflects_across_the_curved_surface_not_only_the_outline() {
        let bezel = 20.;
        let reflection = |d, x, y| glass_surface_reflection(d, x, y, bezel);
        assert!(reflection(3., 0.6, 0.8) > reflection(3., -0.8, 0.6));
        assert!(reflection(3., -0.6, -0.8) > 0.01);
        assert_eq!(reflection(bezel, 0., 1.), 0.);
        assert_eq!(reflection(-1., 0., 1.), 0.);
        for i in 0..100 {
            let value = reflection(i as f32 * bezel / 100., 0., 1.);
            assert!((-0.35..=0.36).contains(&value));
        }
    }
    #[test]
    fn side_contours_are_firmer_while_horizontal_shoulders_carry_light() {
        let reflection = |d, x, y| glass_surface_reflection(d, x, y, 20.);
        assert!(reflection(0., 1., 0.) < reflection(0., 0., 1.) - 0.10);
        assert!(reflection(0., -1., 0.) < reflection(0., 0., -1.) - 0.08);
        assert_eq!(reflection(3., 1., 0.), 0.);
        assert!(reflection(2., 0., 1.) > 0.01);
        for angle in 0..72 {
            let a = angle as f32 * core::f32::consts::TAU / 72.;
            for step in 0..100 {
                let value = reflection(step as f32 * 0.2, libm::cosf(a), libm::sinf(a));
                assert!((-0.35..=0.36).contains(&value));
            }
        }
    }
    #[test]
    fn horizontal_rims_remain_visible_and_thinner_than_the_sides() {
        let side_width = glass_contour_width(1., 1.);
        for y in [-1., 1.] {
            let width = glass_contour_width(0., 1.);
            assert!(width < side_width);
            let reflection = |d| glass_surface_reflection(d, 0., y, 20.);
            assert!(reflection(0.) < reflection(2.) - 0.03);
            assert!(reflection(0.5) < reflection(2.) - 0.03);
            assert!(
                reflection(2.) > 0.,
                "the broad shoulder still carries light"
            );
        }
    }
    #[test]
    fn shadow_widths_remain_distinct_in_device_pixels() {
        for scale in [0.75, 1., 1.25, 1.5, 2., 3.] {
            let top = glass_contour_width(0., scale) * scale;
            let side = glass_contour_width(1., scale) * scale;
            assert_eq!(top, libm::roundf(top));
            assert_eq!(side, libm::roundf(side));
            assert!(top >= 1. && side >= top + 1.);
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
