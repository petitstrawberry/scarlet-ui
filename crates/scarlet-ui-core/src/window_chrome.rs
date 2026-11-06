//! Standard Window chrome for hosts whose content is rendered externally.
//!
//! The host retains its own event loop and image resources. This renderer only
//! owns the ScarletUI frame; input and dimensions use physical surface pixels.
use crate::{
    buffer::Buffer,
    color::Color,
    compositor::DamageRect,
    event::{Event, MouseEvent, WindowEvent},
    geometry::{Rect, Size},
    pipeline::RenderingPipeline,
    renderer::{BackendFrame, CpuPaintRenderer, PaintBackend, PaintContext},
    view::View,
    views::{Spacer, Window, WindowContentLayout},
};
use alloc::{boxed::Box, string::String, vec::Vec};

/// Physical-pixel rectangle, also used by foreign-language frame hosts.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChromeRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Physical-pixel insets around the external content.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChromeInsets {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}
impl ChromeInsets {
    pub fn content(self, width: u32, height: u32) -> Option<ChromeRect> {
        let width = width.checked_sub(self.left.checked_add(self.right)?)?;
        let height = height.checked_sub(self.top.checked_add(self.bottom)?)?;
        (width > 0 && height > 0).then_some(ChromeRect {
            x: self.left,
            y: self.top,
            width,
            height,
        })
    }
}

/// Geometry derives from the same Window layout that paints and handles input.
pub fn chrome_insets(scale_milli: u32, decorated: bool) -> ChromeInsets {
    let layout = WindowContentLayout::new(decorated);
    let scale = scale_milli as f32 / 1000.0;
    let offset = layout.offset();
    let extra = layout.decoration_size();
    ChromeInsets {
        left: libm::ceilf(offset.x * scale) as u32,
        top: libm::ceilf(offset.y * scale) as u32,
        right: libm::ceilf((extra.width - offset.x) * scale) as u32,
        bottom: libm::ceilf((extra.height - offset.y) * scale) as u32,
    }
}

/// One thread-affine Window pipeline. Call and destroy it on its creating UI thread.
pub struct WindowChrome {
    pipeline: RenderingPipeline,
    output: Buffer,
    width: u32,
    height: u32,
    scale_milli: u32,
    insets: ChromeInsets,
    clip: Vec<ChromeRect>,
    margin: u32,
}
impl WindowChrome {
    /// Create chrome at an outer physical-pixel extent, without rendering app pixels.
    pub fn new(
        title: impl Into<String>,
        width: u32,
        height: u32,
        scale_milli: u32,
        decorated: bool,
        resizable: bool,
    ) -> Result<Self, &'static str> {
        if !(250..=8000).contains(&scale_milli)
            || width > 16384
            || height > 16384
            || u64::from(width) * u64::from(height) > 16_000_000
        {
            return Err("Invalid frame extent or scale");
        }
        let insets = chrome_insets(scale_milli, decorated);
        if insets.content(width, height).is_none() {
            return Err("Frame has no content area");
        }
        let scale = scale_milli as f32 / 1000.0;
        let size = Size::new(width as f32 / scale, height as f32 / scale);
        let window = Window::new(title, Spacer::new())
            .size(size)
            .resizable(resizable)
            .decorated(decorated)
            .shadow(false)
            .background_color(Color::TRANSPARENT)
            .opaque(false);
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_scale_milli(scale_milli);
        pipeline.set_root(window.create_element());
        let info = pipeline.layout_initial();
        pipeline.set_paint_backend(Box::new(FrameBackend {
            renderer: CpuPaintRenderer::new(enclosing_size(size), scale_milli, Color::TRANSPARENT),
        }));
        let mut paint = PaintContext::new();
        paint.fill_rounded_rect(
            Rect::from_xywh(0., 0., size.width, size.height),
            info.effective_corner_radius(),
            Color::WHITE,
        );
        let mut mask = CpuPaintRenderer::new(enclosing_size(size), scale_milli, Color::TRANSPARENT);
        mask.execute(&paint);
        let clip = alpha_bands(mask.buffer().data(), mask.buffer().width(), width, height);
        let margin =
            libm::ceilf(info.effective_corner_radius() * scale) as u32 + libm::ceilf(scale) as u32;
        Ok(Self {
            pipeline,
            output: Buffer::from_dimensions(width, height),
            width,
            height,
            scale_milli,
            insets,
            clip,
            margin,
        })
    }
    pub fn insets(&self) -> ChromeInsets {
        self.insets
    }
    pub fn content(&self) -> ChromeRect {
        self.insets.content(self.width, self.height).unwrap()
    }
    pub fn clip(&self) -> &[ChromeRect] {
        &self.clip
    }
    pub fn margin(&self) -> u32 {
        self.margin
    }
    pub fn has_dirty(&self) -> bool {
        self.pipeline.has_dirty()
    }
    pub fn render(&mut self) -> Option<&Buffer> {
        let source = self.pipeline.render()?;
        if source.width() < self.width || source.height() < self.height {
            return None;
        }
        let stride = source.width() as usize * 4;
        let row = self.width as usize * 4;
        for (y, target) in self.output.data_mut().chunks_exact_mut(row).enumerate() {
            target.copy_from_slice(&source.data()[y * stride..y * stride + row]);
        }
        Some(&self.output)
    }
    /// Dispatch a physical-pixel mouse event to ScarletUI's standard controls.
    pub fn mouse(&mut self, event: MouseEvent) -> Vec<WindowEvent> {
        let logical = |v: i32| (i64::from(v) * 1000 / i64::from(self.scale_milli)) as i32;
        let event = match event {
            MouseEvent::Moved { x, y } => MouseEvent::Moved {
                x: logical(x),
                y: logical(y),
            },
            MouseEvent::ButtonPressed {
                button,
                x,
                y,
                click_count,
            } => MouseEvent::ButtonPressed {
                button,
                x: logical(x),
                y: logical(y),
                click_count,
            },
            MouseEvent::ButtonReleased {
                button,
                x,
                y,
                click_count,
            } => MouseEvent::ButtonReleased {
                button,
                x: logical(x),
                y: logical(y),
                click_count,
            },
            other => other,
        };
        self.pipeline.handle_event(&Event::Mouse(event));
        self.pipeline
            .take_emitted_events()
            .into_iter()
            .filter_map(|event| {
                if let Event::Window(action) = event {
                    Some(action)
                } else {
                    None
                }
            })
            .collect()
    }
}

// CPU buffers have integral logical dimensions. Keep a padded raster target,
// then crop its rows to the host's exact physical extent. Layout and input keep
// the fractional logical size; padding never enlarges the application surface.
fn enclosing_size(size: Size) -> Size {
    Size::new(libm::ceilf(size.width), libm::ceilf(size.height))
}
struct FrameBackend {
    renderer: CpuPaintRenderer,
}
impl PaintBackend for FrameBackend {
    fn resize(&mut self, size: Size, scale: u32) {
        self.renderer.resize(enclosing_size(size), scale);
    }
    fn render<'a>(
        &'a mut self,
        context: &PaintContext<'_>,
        background: Color,
        damage: Option<&[Rect]>,
        _physical_damage: Option<&[DamageRect]>,
    ) -> crate::error::Result<BackendFrame<'a>> {
        self.renderer.set_background_color(background);
        self.renderer.execute_with_damage(context, damage);
        Ok(BackendFrame::Cpu {
            buffer: self.renderer.buffer(),
        })
    }
}

fn alpha_bands(pixels: &[u8], stride: u32, width: u32, height: u32) -> Vec<ChromeRect> {
    let mut bands: Vec<ChromeRect> = Vec::new();
    for y in 0..height {
        let row = &pixels
            [y as usize * stride as usize * 4..(y as usize * stride as usize + width as usize) * 4];
        let left = (0..width).find(|x| row[*x as usize * 4 + 3] >= 128);
        let right = (0..width).rev().find(|x| row[*x as usize * 4 + 3] >= 128);
        if let (Some(left), Some(right)) = (left, right) {
            let width = right - left + 1;
            if let Some(last) = bands.last_mut()
                && last.x == left
                && last.width == width
                && last.y + last.height == y
            {
                last.height += 1;
            } else {
                bands.push(ChromeRect {
                    x: left,
                    y,
                    width,
                    height: 1,
                });
            }
        }
    }
    bands
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standard_chrome_keeps_external_content_transparent_at_each_scale() {
        for scale in [1000, 1250, 1500, 2000] {
            for (cw, ch) in [(320, 240), (321, 241)] {
                let inset = chrome_insets(scale, true);
                let (w, h) = (cw + inset.left + inset.right, ch + inset.top + inset.bottom);
                let mut chrome =
                    WindowChrome::new("External content", w, h, scale, true, true).unwrap();
                assert_eq!((chrome.content().width, chrome.content().height), (cw, ch));
                let content = chrome.content();
                let pixels = chrome.render().unwrap().data();
                assert_eq!(pixels.len(), (w * h * 4) as usize);
                assert_eq!(
                    pixels[(((content.y + 100) * w + content.x + 100) * 4 + 3) as usize],
                    0
                );
                assert!(
                    pixels[..(inset.top * w * 4) as usize]
                        .chunks_exact(4)
                        .any(|p| p[3] > 0)
                );
            }
        }
    }
    #[test]
    fn standard_controls_emit_actions_at_physical_coordinates() {
        use crate::event::MouseButton;
        for scale in [1000, 2000] {
            let inset = chrome_insets(scale, true);
            let width = 320 * scale / 1000 + inset.left + inset.right;
            let height = 240 * scale / 1000 + inset.top + inset.bottom;
            let mut chrome =
                WindowChrome::new("Controls", width, height, scale, true, true).unwrap();
            chrome.render().unwrap();
            let x = width as i32 - (10 * scale / 1000) as i32;
            let y = (inset.top / 2) as i32;
            let press = chrome.mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                x,
                y,
                click_count: 1,
            });
            let release = chrome.mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x,
                y,
                click_count: 1,
            });
            assert!(
                press
                    .iter()
                    .chain(&release)
                    .any(|a| matches!(a, WindowEvent::CloseRequested))
            );
        }
    }
    #[test]
    fn frameless_content_has_no_insets() {
        let mut chrome = WindowChrome::new("", 320, 240, 1000, false, true).unwrap();
        assert_eq!(
            chrome.content(),
            ChromeRect {
                x: 0,
                y: 0,
                width: 320,
                height: 240
            }
        );
        assert!(
            chrome
                .render()
                .unwrap()
                .data()
                .chunks_exact(4)
                .all(|p| p[3] == 0)
        );
    }
}
