//! Stateless candy-colored artwork; the parent supplies all animation values.

use scarlet_ui::{prelude::*, renderer::PaintContext};
use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    Orbit,
    Zigzag,
    Jelly,
    Party,
}

/// Paint within `size`. Phase loops, progress fills, and burst is idle at 1.
pub fn paint(
    ctx: &mut PaintContext<'_>,
    origin: Point,
    size: Size,
    scene: Scene,
    phase: f32,
    progress: f32,
    burst: f32,
    palette: usize,
) {
    if !size.width.is_finite()
        || !size.height.is_finite()
        || size.width <= 0.0
        || size.height <= 0.0
    {
        return;
    }
    // A centered 420 × 168 canvas fits both the short cards and taller stages.
    let scale = (size.width / 420.0).min(size.height / 168.0);
    let colors = match palette % 3 {
        0 => [0xf780b3, 0x73d7cf, 0xa997f0, 0xffd370], // Candy
        1 => [0x5ec8a7, 0x89d4e3, 0xa4bc87, 0xfadb91], // Mint
        _ => [0xf99088, 0xffbd80, 0xc196db, 0xffdb81], // Sunset
    }
    .map(|rgb| Color::rgb((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255));
    let mut canvas = Canvas {
        ctx,
        origin: Point::new(
            origin.x + (size.width - 420.0 * scale) * 0.5,
            origin.y + (size.height - 168.0 * scale) * 0.5,
        ),
        scale,
        colors,
    };
    let angle = phase.rem_euclid(1.0) * TAU;
    let progress = progress.clamp(0.0, 1.0);
    match scene {
        Scene::Orbit => canvas.orbit(angle, progress),
        Scene::Zigzag => canvas.zigzag(angle, progress),
        Scene::Jelly => canvas.jelly(angle, progress),
        Scene::Party => canvas.party(angle, burst.clamp(0.0, 1.0)),
    }
}

fn fade(color: Color, alpha: f32) -> Color {
    Color { a: alpha, ..color }
}

fn sample(steps: usize, f: impl Fn(f32) -> Point) -> Vec<Point> {
    (0..=steps).map(|i| f(i as f32 / steps as f32)).collect()
}

fn zigzag_point(t: f32) -> Point {
    let step = t * 6.0;
    let index = (step as usize).min(5);
    let along = step - index as f32;
    let y = if index % 2 == 0 { along } else { 1.0 - along };
    Point::new(38.0 + 344.0 * t, 110.0 - 56.0 * y)
}

struct Canvas<'a, 'b> {
    ctx: &'a mut PaintContext<'b>,
    origin: Point,
    scale: f32,
    colors: [Color; 4],
}

impl Canvas<'_, '_> {
    fn point(&self, p: Point) -> Point {
        Point::new(
            self.origin.x + p.x * self.scale,
            self.origin.y + p.y * self.scale,
        )
    }

    fn circle(&mut self, x: f32, y: f32, radius: f32, color: Color) {
        self.ctx
            .fill_circle(self.point(Point::new(x, y)), radius * self.scale, color);
    }

    fn path(&mut self, mut points: Vec<Point>, color: Color) {
        points.pop(); // Filled curves close implicitly; omit the duplicate endpoint.
        let points: Vec<_> = points.into_iter().map(|p| self.point(p)).collect();
        self.ctx.fill_path(points, color);
    }

    fn ellipse(&mut self, x: f32, y: f32, rx: f32, ry: f32, color: Color) {
        let points = sample(48, |t| {
            let a = TAU * t;
            Point::new(x + rx * a.cos(), y + ry * a.sin())
        });
        self.path(points, color);
    }

    fn stroke(&mut self, points: Vec<Point>, width: f32, color: Color) {
        // StrokePath closes paths with >2 points on some backends. Keep arcs open.
        for pair in points.windows(2) {
            self.ctx.draw_line(
                self.point(pair[0]),
                self.point(pair[1]),
                width * self.scale,
                color,
            );
        }
        if let (Some(first), Some(last)) = (points.first(), points.last()) {
            if points.len() == 2 || first.distance_to(*last) > 0.001 {
                self.circle(first.x, first.y, width * 0.5, color);
                self.circle(last.x, last.y, width * 0.5, color);
            }
        }
    }

    fn arc(&mut self, radius: f32, start: f32, sweep: f32, width: f32, color: Color) {
        if sweep <= 0.0 {
            return;
        }
        let count = (sweep * radius / 3.0).ceil().max(1.0) as usize;
        let points = sample(count, |t| {
            let a = start + sweep * t;
            Point::new(210.0 + radius * a.cos(), 84.0 + radius * a.sin())
        });
        self.stroke(points, width, color);
    }

    fn pill(&mut self, x: f32, y: f32, width: f32, height: f32, color: Color) {
        if width <= 0.0 {
            return;
        }
        let rect = Rect::new(
            self.point(Point::new(x, y)),
            Size::new(width * self.scale, height * self.scale),
        );
        self.ctx
            .fill_rounded_rect(rect, height.min(width) * self.scale * 0.5, color);
    }

    fn star(&mut self, x: f32, y: f32, radius: f32, rotation: f32, color: Color) {
        // A sampled radial curve gives the five tips and valleys soft corners.
        let points = sample(60, |t| {
            let a = TAU * t;
            let r = radius * (0.76 + 0.24 * (5.0 * a).cos());
            let direction = a + rotation - PI * 0.5;
            Point::new(x + r * direction.cos(), y + r * direction.sin())
        });
        self.path(points, color);
    }

    fn heart(&mut self, x: f32, y: f32, radius: f32, color: Color) {
        let points = sample(48, |t| {
            let a = TAU * t;
            let h =
                13.0 * a.cos() - 5.0 * (2.0 * a).cos() - 2.0 * (3.0 * a).cos() - (4.0 * a).cos();
            Point::new(x + radius * a.sin().powi(3), y - radius * h / 16.0)
        });
        self.path(points, color);
    }

    fn face(&mut self, x: f32, y: f32, scale: f32) {
        let ink = Color::rgb(66, 50, 78);
        let blush = Color::rgb(246, 143, 163);
        for side in [-1.0, 1.0] {
            let (cx, cy) = (x + side * 13.0 * scale, y + 4.0 * scale);
            self.ellipse(cx, cy, 4.5 * scale, 2.4 * scale, blush);
            self.circle(x + side * 6.5 * scale, y - 2.0 * scale, 2.2 * scale, ink);
        }
        let smile = sample(12, |t| {
            Point::new(
                x + (t * 10.0 - 5.0) * scale,
                y + (4.0 + 3.0 * (PI * t).sin()) * scale,
            )
        });
        self.stroke(smile, 1.8 * scale, ink);
    }

    fn orbit(&mut self, angle: f32, progress: f32) {
        let c = self.colors;
        self.circle(210.0, 84.0, 73.0, fade(c[2], 0.08));
        self.arc(62.0, 0.0, TAU, 2.0, fade(c[2], 0.16));
        self.arc(41.0, 0.0, TAU, 5.0, fade(c[0], 0.18));
        self.arc(41.0, -PI * 0.5, TAU * progress, 5.0, c[0]);
        self.circle(210.0, 84.0, 29.0, Color::rgb(255, 244, 217));
        self.ellipse(200.0, 70.0, 7.0, 3.5, fade(Color::WHITE, 0.8));
        self.face(210.0, 85.0, 1.0);
        for i in 0..40 {
            let t = i as f32 / 39.0;
            let a = angle - PI * 0.5 - 2.4 * (1.0 - t);
            self.circle(
                210.0 + 62.0 * a.cos(),
                84.0 + 62.0 * a.sin(),
                1.0 + 4.0 * t,
                fade(c[(i / 14) % 3], 0.15 + 0.85 * t),
            );
        }
        let head = angle - PI * 0.5;
        let (x, y) = (210.0 + 62.0 * head.cos(), 84.0 + 62.0 * head.sin());
        self.circle(x, y, 7.0, c[2]);
        self.circle(x - 2.0, y - 2.0, 2.0, Color::WHITE);
        for i in 0..3 {
            let a = -angle + TAU * i as f32 / 3.0;
            let (x, y) = (210.0 + 108.0 * a.cos(), 84.0 + 48.0 * a.sin());
            self.circle(x, y, 7.0, c[i]);
            self.circle(x - 2.0, y - 2.0, 2.0, fade(Color::WHITE, 0.85));
        }
        self.star(64.0, 106.0, 7.0 + angle.sin(), 0.2, c[3]);
        self.star(355.0, 57.0, 9.0 - angle.sin(), -0.2, c[1]);
    }

    fn zigzag(&mut self, angle: f32, progress: f32) {
        let c = self.colors;
        for i in 0..25 {
            let x = 38.0 + 344.0 * i as f32 / 24.0;
            let height = if i % 4 == 0 { 7.0 } else { 3.0 };
            self.pill(x - 1.0, 136.0, 2.0, height, fade(c[2], 0.3));
        }
        for i in 0..36 {
            let start = i as f32 / 36.0 + 0.011;
            let end = (i + 1) as f32 / 36.0 - 0.011;
            self.stroke(
                vec![zigzag_point(start), zigzag_point(end)],
                9.0,
                fade(c[2], 0.13),
            );
            if progress > start {
                self.stroke(
                    vec![zigzag_point(start), zigzag_point(progress.min(end))],
                    9.0,
                    c[(i / 6) % 4],
                );
            }
        }
        // Phase keeps the loaded path lively when the progress star is parked.
        if progress > 0.0 {
            for i in 0..3 {
                let travel = 0.5 - 0.5 * (angle + TAU * i as f32 / 3.0).cos();
                let bead = zigzag_point(progress * travel);
                self.circle(bead.x, bead.y, 3.5, c[(i + 1) % 4]);
                self.circle(bead.x - 1.0, bead.y - 1.0, 1.1, Color::WHITE);
            }
        }
        let marker = zigzag_point(progress);
        let y = marker.y - 7.0 - 2.0 * angle.sin();
        self.ellipse(marker.x, 148.0, 13.0, 3.0, fade(c[2], 0.12));
        self.circle(marker.x, y, 22.0, fade(c[3], 0.18));
        self.star(marker.x, y, 18.0, 0.12 * angle.sin(), c[3]);
        self.face(marker.x, y + 1.0, 0.55);
        self.star(47.0, 35.0, 5.0 + angle.cos(), 0.3, c[0]);
        self.star(363.0, 33.0, 5.0 - angle.cos(), -0.3, c[1]);
    }

    fn jelly(&mut self, angle: f32, progress: f32) {
        let c = self.colors;
        let hop = (1.0 - angle.cos()) * 0.5;
        let (rx, ry) = (47.0 - 10.0 * hop, 28.0 + 6.0 * hop);
        let (x, y) = (210.0 + 8.0 * angle.sin(), 128.0 - 22.0 * hop - ry);
        self.ellipse(
            210.0,
            135.0,
            45.0 - 10.0 * hop,
            5.0,
            fade(c[2], 0.16 - 0.06 * hop),
        );
        for side in [-1.0, 1.0] {
            self.ellipse(x + side * 18.0, y - ry + 5.0, 9.0, 17.0, c[0]);
            self.ellipse(
                x + side * 18.0,
                y - ry + 1.0,
                4.0,
                9.0,
                fade(Color::WHITE, 0.45),
            );
        }
        let blob = sample(64, |t| {
            let a = TAU * t;
            let wobble = 1.0 + 0.045 * (3.0 * a + angle).sin();
            Point::new(x + rx * a.cos() * wobble, y + ry * a.sin())
        });
        self.path(blob, c[0]);
        self.ellipse(x - 14.0, y - 13.0, 10.0, 4.0, fade(Color::WHITE, 0.55));
        self.face(x, y + 3.0, 1.15);
        for side in [-1.0, 1.0] {
            let (sx, sy) = (210.0 + side * 85.0, 85.0 - 17.0 * hop);
            self.star(sx, sy, 6.0 + 2.0 * hop, side * angle, c[1]);
            let (dx, dy) = (210.0 + side * 112.0, 105.0 + 4.0 * angle.sin());
            self.circle(dx, dy, 3.0, fade(c[2], 0.5));
        }
        self.star(139.0, 153.0, 7.0, 0.0, c[3]);
        self.pill(155.0, 148.0, 110.0, 10.0, fade(c[1], 0.18));
        self.pill(155.0, 148.0, 110.0 * progress, 10.0, c[1]);
    }

    fn party(&mut self, angle: f32, burst: f32) {
        let c = self.colors;
        self.ellipse(210.0, 84.0, 69.0, 60.0, fade(c[3], 0.1));
        for i in 0..8 {
            let a = TAU * i as f32 / 8.0;
            let (x, y) = (210.0 + 132.0 * a.cos(), 84.0 + 55.0 * a.sin());
            let radius = 4.0 + 1.5 * (angle + a).sin();
            self.star(x, y, radius, 0.2 * angle.sin(), c[i % 4]);
        }
        if burst < 1.0 {
            let spread = 1.0 - (1.0 - burst).powi(3);
            let alpha = (1.0 - burst) * (burst * 12.0).min(1.0);
            let ring_color = fade(c[0], alpha * 0.35);
            self.arc(28.0 + 43.0 * spread, 0.0, TAU, 2.0, ring_color);
            for i in 0..28 {
                let a = TAU * i as f32 / 28.0;
                let x = 210.0 + (125.0 + (i % 3) as f32 * 15.0) * spread * a.cos();
                let y = 84.0 + 54.0 * spread * a.sin() + 10.0 * burst * burst;
                let radius = (5.0 + (i % 3) as f32) * (1.0 - 0.35 * burst);
                let color = fade(c[i % 4], alpha);
                match i % 3 {
                    0 => self.star(x, y, radius, a + burst * TAU, color),
                    1 => self.heart(x, y, radius, color),
                    _ => self.pill(x - 2.0, y - radius * 0.5, 4.0, radius, color),
                }
            }
        }
        self.ellipse(210.0, 135.0, 33.0, 4.0, fade(c[2], 0.13));
        let radius = 37.0 + 1.5 * angle.sin();
        self.star(210.0, 84.0, radius, 0.06 * angle.sin(), c[3]);
        self.ellipse(200.0, 70.0, 7.0, 3.0, fade(Color::WHITE, 0.65));
        self.face(210.0, 86.0, 1.1);
    }
}
