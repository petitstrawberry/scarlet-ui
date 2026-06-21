//! RenderingPipeline - Integration of PipelineOwner, ElementTree, and Compositor
//!
//! RenderingPipeline is the main entry point for the rendering system.
//! It orchestrates all phases of the rendering pipeline.

#![allow(deprecated)]

use crate::buffer::Buffer;
use crate::compositor::DamageRect;
use crate::element::{Element, ElementTree, LayoutConstraints};
use crate::event::EventDispatcher;
use crate::geometry::{Point, Size};
use crate::pipeline::{PipelineId, PipelineOwner};
use crate::renderer::{CpuPaintRenderer, CpuRenderer, FrameSize, PaintContext};
use crate::views::WindowInfo;
use alloc::boxed::Box;
use alloc::vec::Vec;

/// RenderingPipeline integrates all components of the rendering system
pub struct RenderingPipeline {
    element_tree: ElementTree,
    pipeline_owner: PipelineOwner,
    renderer: Option<Box<dyn crate::renderer::Renderer>>,
    window_size: Size,
    scale_milli: u32,
    event_dispatcher: EventDispatcher,
    paint_renderer: Option<CpuPaintRenderer>,
    paint_enabled: bool,
}

impl RenderingPipeline {
    /// Create a new RenderingPipeline
    pub fn new() -> Self {
        Self::with_pipeline_id(PipelineId::generate())
    }

    /// Create a new RenderingPipeline with a stable owner ID.
    pub fn with_pipeline_id(pipeline_id: PipelineId) -> Self {
        Self {
            element_tree: ElementTree::with_pipeline_id(pipeline_id),
            pipeline_owner: PipelineOwner::with_pipeline_id(pipeline_id),
            renderer: None,
            window_size: Size::new(800.0, 600.0),
            scale_milli: 1000,
            event_dispatcher: EventDispatcher::new(),
            paint_renderer: None,
            paint_enabled: false,
        }
    }

    pub fn set_paint_enabled(&mut self, enabled: bool) {
        self.paint_enabled = enabled;
    }

    /// Return this pipeline's owner ID.
    pub const fn pipeline_id(&self) -> PipelineId {
        self.element_tree.pipeline_id()
    }

    /// Unmount the element tree and discard pending global dirty work.
    pub fn teardown(&mut self) {
        self.element_tree.clear_root();
        crate::pipeline::clear_global_dirty(self.pipeline_id());
        self.renderer = None;
        self.paint_renderer = None;
    }

    /// Set the output scale in milli-units.
    pub fn set_scale_milli(&mut self, scale_milli: u32) {
        self.scale_milli = scale_milli.max(1);
        crate::graphics::set_current_scale_milli(self.scale_milli);
        if let Some(ref mut renderer) = self.renderer {
            renderer.resize(FrameSize {
                width: self.window_size.width,
                height: self.window_size.height,
                scale_milli: self.scale_milli,
            });
        }
        if let Some(ref mut paint_renderer) = self.paint_renderer {
            paint_renderer.resize(self.window_size, self.scale_milli);
        }
        if let Some(root) = self.element_tree.root_mut() {
            root.clear_buffers();
        }
        if let Some(root) = self.element_tree.root() {
            self.pipeline_owner.mark_needs_layout(root.id());
        }
    }

    /// Return the current output scale in milli-units.
    pub fn scale_milli(&self) -> u32 {
        self.scale_milli
    }

    /// Set the root Element
    pub fn set_root(&mut self, root_element: Box<dyn Element>) {
        self.element_tree.set_root(root_element);
        if let Some(root) = self.element_tree.root() {
            self.event_dispatcher.set_root(root.id());
        }
    }

    /// Get the ElementTree
    pub fn element_tree(&self) -> &ElementTree {
        &self.element_tree
    }

    /// Get mutable reference to the ElementTree
    pub fn element_tree_mut(&mut self) -> &mut ElementTree {
        &mut self.element_tree
    }

    /// Get the PipelineOwner
    pub fn pipeline_owner(&self) -> &PipelineOwner {
        &self.pipeline_owner
    }

    /// Get mutable reference to the PipelineOwner
    pub fn pipeline_owner_mut(&mut self) -> &mut PipelineOwner {
        &mut self.pipeline_owner
    }

    /// Get the StateRegistry
    pub fn state_registry(&self) -> &crate::pipeline::StateRegistry {
        self.pipeline_owner.state_registry()
    }

    /// Get mutable reference to the StateRegistry
    pub fn state_registry_mut(&mut self) -> &mut crate::pipeline::StateRegistry {
        self.pipeline_owner.state_registry_mut()
    }

    /// Has any dirty elements?
    pub fn has_dirty(&self) -> bool {
        self.pipeline_owner.has_dirty()
    }

    /// Extract window information from the element tree
    ///
    /// This searches the element tree for a Window View and extracts
    /// the app_id, title, size, window type, background, and policies from it.
    ///
    /// Returns window information or defaults if no Window is found.
    fn extract_window_info(&self) -> WindowInfo {
        // Default values
        let default_info = WindowInfo::new(
            alloc::string::String::from("com.example.scarletui"),
            alloc::string::String::from("ScarletUI Application"),
            Size::new(800.0, 600.0),
            0,
            None,
            true,
            true,
            crate::color::ColorPalette::light().window_background(),
            true,
        );

        // Try to find a Window View in the element tree
        if let Some(root) = self.element_tree.root() {
            if let Some(window_info) = self.find_window_view(root) {
                return window_info;
            }
        }

        default_info
    }

    /// Recursively search for a Window View in the element tree
    fn find_window_view(&self, element: &dyn Element) -> Option<WindowInfo> {
        // Check if this element provides window info
        if let Some(info) = element.get_window_info() {
            return Some(info);
        }

        // Check children recursively
        for child in element.children() {
            if let Some(info) = self.find_window_view(child.as_ref()) {
                return Some(info);
            }
        }

        None
    }

    /// Perform initial layout
    ///
    /// This should be called once after setting the root element
    /// to determine the window size and create the compositor.
    ///
    /// Returns window information extracted from the Window View.
    pub fn layout_initial(&mut self) -> WindowInfo {
        // Extract window info first
        let window_info = self.extract_window_info();

        // Use the preferred size from Window as the actual window size
        let window_size = window_info.size;

        // Perform initial layout with tight constraints matching the window size
        let constraints = LayoutConstraints::tight(window_size.width, window_size.height);
        let _layout_size = self.element_tree.layout(constraints);

        // Create renderer with the window size
        crate::graphics::set_current_scale_milli(self.scale_milli);
        self.renderer = Some(Box::new(CpuRenderer::new(
            window_size,
            self.scale_milli,
            window_info.background_color,
        )));
        self.window_size = window_size;

        // Mark root as dirty for initial paint
        if let Some(root) = self.element_tree.root() {
            self.pipeline_owner.mark_needs_paint(root.id());
        }

        window_info
    }

    /// Set window size and resize compositor
    pub fn resize(&mut self, new_size: Size) {
        self.window_size = new_size;
        crate::graphics::set_current_scale_milli(self.scale_milli);
        if let Some(ref mut renderer) = self.renderer {
            renderer.resize(FrameSize {
                width: new_size.width,
                height: new_size.height,
                scale_milli: self.scale_milli,
            });
        }
        if let Some(ref mut paint_renderer) = self.paint_renderer {
            paint_renderer.resize(new_size, self.scale_milli);
        }

        if let Some(root) = self.element_tree.root_mut() {
            root.clear_buffers();
        }

        // Mark entire tree for relayout
        // Note: In a full implementation, we would mark specific elements
        if let Some(root) = self.element_tree.root() {
            self.pipeline_owner.mark_needs_layout(root.id());
        }
    }

    /// Handle a render frame
    ///
    /// This flushes all dirty phases and renders to the window buffer.
    pub fn render(&mut self) -> Option<&Buffer> {
        if crate::debug::is_enabled() {
            crate::logln!("[RenderingPipeline] render() starting...");
        }
        // Flush all dirty phases (build, layout, paint)
        crate::graphics::set_current_scale_milli(self.scale_milli);
        self.pipeline_owner
            .flush(&mut self.element_tree, self.window_size);
        if crate::debug::is_enabled() {
            crate::logln!("[RenderingPipeline] flush() completed");
        }

        let background_color = self.extract_window_info().background_color;

        if self.paint_enabled {
            return self.render_paint_path(background_color);
        }

        if let Some(ref mut renderer) = self.renderer {
            renderer.set_background_color(background_color);
            if let Some(root) = self.element_tree.root() {
                let dirty_ids = self.pipeline_owner.last_paint_ids();
                renderer.composite(root, dirty_ids);
            }

            Some(renderer.buffer())
        } else {
            None
        }
    }

    fn render_paint_path(&mut self, background_color: crate::color::Color) -> Option<&Buffer> {
        let size = self.window_size;
        let scale = self.scale_milli;

        if self.paint_renderer.is_none() {
            self.paint_renderer = Some(CpuPaintRenderer::new(size, scale, background_color));
        }

        let pr = self.paint_renderer.as_mut().unwrap();
        pr.set_background_color(background_color);

        let mut ctx = PaintContext::new();
        let any_painted = if let Some(root) = self.element_tree.root() {
            let base_painted = Self::walk_and_paint(&mut ctx, root, Point::ZERO);
            let overlay_painted = Self::paint_select_overlays(&mut ctx, root, Point::ZERO);
            base_painted || overlay_painted
        } else {
            false
        };

        if any_painted {
            pr.execute(&ctx);
        }

        Some(pr.buffer())
    }

    fn walk_and_paint(ctx: &mut PaintContext, element: &dyn Element, origin: Point) -> bool {
        let abs = Point::new(
            origin.x + element.position().x,
            origin.y + element.position().y,
        );
        let mut painted = Self::paint_element_self(ctx, element, abs);

        for child in element.children() {
            if Self::walk_and_paint(ctx, child.as_ref(), abs) {
                painted = true;
            }
        }

        painted
    }

    fn paint_select_overlays(ctx: &mut PaintContext, element: &dyn Element, origin: Point) -> bool {
        let abs = Point::new(
            origin.x + element.position().x,
            origin.y + element.position().y,
        );
        let mut painted = false;

        for child in element.children() {
            if Self::paint_select_overlays(ctx, child.as_ref(), abs) {
                painted = true;
            }
        }

        if Self::is_expanded_select(element) && Self::paint_element_self(ctx, element, abs) {
            painted = true;
        }

        painted
    }

    fn is_expanded_select(element: &dyn Element) -> bool {
        element
            .render_object()
            .and_then(|render_object| {
                render_object
                    .as_any()
                    .downcast_ref::<crate::views::SelectRenderObject>()
            })
            .is_some_and(|select| select.is_expanded())
    }

    fn paint_element_self(ctx: &mut PaintContext, element: &dyn Element, abs: Point) -> bool {
        let Some(ro) = element.render_object() else {
            return false;
        };

        let before = ctx.commands().len();
        if ro.paint(ctx, abs) || ctx.commands().len() > before {
            return true;
        }

        if let Some(buf) = element.get_buffer() {
            let rect = crate::geometry::Rect::new(
                abs,
                Size::new(buf.logical_width() as f32, buf.logical_height() as f32),
            );
            ctx.draw_buffer(rect, buf.clone());
            return true;
        }

        false
    }

    /// Handle a render frame and return the buffer with physical damage rectangles.
    ///
    /// The damage is `None` when the whole window should be presented.
    pub fn render_with_damage(&mut self) -> Option<(&Buffer, Option<&[DamageRect]>)> {
        if self.paint_enabled {
            crate::graphics::set_current_scale_milli(self.scale_milli);
            self.pipeline_owner
                .flush(&mut self.element_tree, self.window_size);
            self.render_paint_path(self.extract_window_info().background_color)?;
            let pr = self.paint_renderer.as_ref().unwrap();
            return Some((pr.buffer(), None));
        }
        self.render()?;
        let renderer = self.renderer.as_ref()?;
        Some((renderer.buffer(), renderer.damage()))
    }

    pub fn window_buffer(&self) -> Option<&Buffer> {
        self.renderer.as_ref().map(|r| r.buffer())
    }

    pub fn window_buffer_mut(&mut self) -> Option<&mut Buffer> {
        self.renderer.as_mut().map(|r| r.buffer_mut())
    }

    /// Get the current window size
    pub fn window_size(&self) -> Size {
        self.window_size
    }

    /// Handle an event
    ///
    /// In a full implementation, this would route events through the
    /// EventDispatcher to the target elements.
    pub fn handle_event(&mut self, _event: &crate::event::Event) -> bool {
        self.event_dispatcher
            .dispatch(&mut self.element_tree, _event)
    }

    /// Take emitted events from the event dispatcher
    pub fn take_emitted_events(&mut self) -> Vec<crate::event::Event> {
        self.event_dispatcher.take_emitted_events()
    }

    pub fn focused_text_input_state(&self) -> Option<crate::element::TextInputElementState> {
        self.element_tree.focused_text_input_state()
    }
}

impl Drop for RenderingPipeline {
    fn drop(&mut self) {
        self.teardown();
    }
}

impl Default for RenderingPipeline {
    fn default() -> Self {
        Self::new()
    }
}
