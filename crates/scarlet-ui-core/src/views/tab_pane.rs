//! Shared framing for a tab strip and its selected content.

use alloc::boxed::Box;
use core::any::Any;

use super::{ElevationRole, Surface, SurfaceStyle, VStack};
use crate::{
    color::{Color, ColorPalette},
    element::{ComponentElement, Element},
    view::{View, ViewExt},
};

/// A tab strip and content viewport inside one continuous rounded frame.
///
/// Use this when an application owns tab selection, closing, dragging or split
/// groups. `TabPane` owns only their shared presentation: the bar keeps its
/// intrinsic height and content fills the remaining height. Both slots are
/// clipped to one outer silhouette; the surface background paints first and
/// the frame paints **after** both slots, including opaque content backgrounds.
/// No inset is needed to keep content from covering the frame.
///
/// For a self-contained selection model and built-in tab buttons, use `TabView`.
/// A `TabView` can also be the content of a pane (for example, settings sections).
///
/// ```no_run
/// use scarlet_ui_core::prelude::*;
/// let pane = TabPane::new(
///     Text::new("Document.md").padding(8.0),
///     Text::new("Selected document"),
/// ).border_color(Color::rgb(150, 150, 160));
/// ```
#[derive(Clone)]
pub struct TabPane<B: View, C: View> {
    bar: B,
    content: C,
    style: SurfaceStyle,
    border: Color,
    radius: f32,
}

impl<B: View, C: View> TabPane<B, C> {
    /// Combine a top tab strip and a selected-content view. This does not change
    /// the application's tab identities, selection, focus or drag/drop behavior.
    pub fn new(bar: B, content: C) -> Self {
        Self {
            bar,
            content,
            style: SurfaceStyle::bubble_glass().elevation(ElevationRole::Raised),
            border: ColorPalette::default().divider(),
            radius: 16.0,
        }
    }

    /// Set the background material and elevation. The foreground frame remains
    /// enabled regardless of the material, unlike a general-purpose `Surface`.
    pub fn surface_style(mut self, style: SurfaceStyle) -> Self {
        self.style = style;
        self
    }

    /// Set the frame color; the shared surface renderer paints it above content.
    pub fn border_color(mut self, color: Color) -> Self {
        self.border = color;
        self
    }

    /// Set the single radius used for content clipping and the concentric frame.
    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.radius = if radius.is_finite() {
            radius.max(0.0)
        } else {
            0.0
        };
        self
    }
}

impl<B: View + Clone, C: View + Clone> View for TabPane<B, C> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(
            self.clone(),
            Self::build,
        ))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl<B: View + Clone, C: View + Clone> TabPane<B, C> {
    fn build(view: &Self) -> Box<dyn View> {
        Box::new(
            Surface::section(
                VStack::new((
                    view.bar.clone().clip(),
                    view.content
                        .clone()
                        .frame(f32::INFINITY, f32::INFINITY)
                        .clip(),
                ))
                .spacing(0.0)
                .frame(f32::INFINITY, f32::INFINITY),
            )
            .surface_style(view.style)
            .corner_radius(view.radius)
            .clip_content(true)
            .border_color(view.border),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        geometry::Size,
        pipeline::RenderingPipeline,
        views::{Spacer, Window},
    };

    #[test]
    fn opaque_slots_cannot_cover_the_shared_frame_even_after_rebuild_and_resize() {
        let mut pipeline = RenderingPipeline::new();
        let mut previous_ids = None;
        for (width, height, border, content) in [
            (176, 116, Color::RED, Color::GREEN),
            (216, 156, Color::BLUE, Color::rgb(0, 200, 0)),
        ] {
            let pane = TabPane::new(
                Spacer::new()
                    .frame(f32::INFINITY, 24.0)
                    .background(Color::BLACK),
                Spacer::new().background(content),
            )
            .border_color(border)
            .padding(8.0);
            let window = Window::new("Tabs", pane)
                .decorated(false)
                .size(Size::new(width as f32, height as f32));
            if let Some(root) = pipeline.element_tree_mut().root_mut() {
                root.update(&window);
            } else {
                pipeline.set_root(window.create_element());
            }
            pipeline.resize(Size::new(width as f32, height as f32));
            pipeline.layout_initial();
            let mut ids = alloc::vec::Vec::new();
            fn collect(
                element: &dyn Element,
                ids: &mut alloc::vec::Vec<crate::element::ElementId>,
            ) {
                ids.push(element.id());
                for child in element.children() {
                    collect(child.as_ref(), ids);
                }
            }
            collect(pipeline.element_tree().root().unwrap(), &mut ids);
            if let Some(old) = &previous_ids {
                assert_eq!(&ids, old, "rebuild replaced slot elements");
            }
            previous_ids = Some(ids);
            let buffer = pipeline.render().unwrap();
            // The left/right frame crosses both the opaque bar and the body.
            for y in [28, 50, height - 28] {
                for x in [8, width - 9] {
                    let pixel = buffer.get_pixel(x, y).unwrap();
                    let under = if y < 32 { Color::BLACK } else { content };
                    assert_ne!(pixel, under.to_bgra(), "frame covered at ({x}, {y})");
                    let channel = if border == Color::RED {
                        (pixel >> 16) & 255
                    } else {
                        pixel & 255
                    };
                    assert!(channel >= 120, "frame missing at ({x}, {y}): {pixel:08x}");
                }
            }
            assert_ne!(
                buffer.get_pixel(width / 2, height - 9),
                Some(content.to_bgra())
            );
            assert_eq!(
                buffer.get_pixel(width / 2, 20),
                Some(Color::BLACK.to_bgra())
            );
            assert_eq!(buffer.get_pixel(width / 2, 50), Some(content.to_bgra()));
            // Content must not square off the lower corners of the pane.
            assert_ne!(buffer.get_pixel(8, height - 9), Some(content.to_bgra()));
        }
        pipeline.teardown();
    }
}
