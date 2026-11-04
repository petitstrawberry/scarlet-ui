//! Render objects used by `NavigationView`.

use crate::buffer::Buffer;
use crate::color::{Color, ColorPalette};
use crate::element::{
    Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult,
};
use crate::geometry::{EdgeInsets, Point, Rect, Size};
use crate::graphics;
use crate::icon::{Icon, IconStyle};
use crate::renderer::PaintContext;
use crate::state::State;
use crate::view::View;
use crate::views::navigation::view::{NavigationPresentation, NavigationSidebarStyle};
use crate::views::style;
use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;
use core::cell::Cell;

/// Layout render object for the complete navigation view.
///
/// Sidebar painting and pointer interaction belong to the first child render
/// object. Keeping that region as a real element lets the event dispatcher's
/// hover-path diff generate `Entered` and `Exited` at the sidebar boundary.
pub struct NavigationViewRenderObject {
    sidebar_width: f32,
    header_height: f32,
    presentation: NavigationPresentation,
    minimum_content_size: Size,
    size: Size,
}

const BOTTOM_BAR_EXTRA_HEIGHT: f32 = 12.0;
const BOTTOM_BAR_ICON_TOP_INSET: f32 = 8.0;
const BOTTOM_BAR_TEXT_BOTTOM_INSET: f32 = 5.0;
const TEXT_LINE_HEIGHT_FACTOR: f32 = 1.2;

fn bottom_bar_height(navigation_item_height: f32) -> f32 {
    navigation_item_height.max(0.0) + BOTTOM_BAR_EXTRA_HEIGHT
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct BottomBarContentGeometry {
    icon_origin: Option<Point>,
    text_origin: Point,
}

fn bottom_bar_content_geometry(
    item: Rect,
    shows_icon: bool,
    icon_size: f32,
    font_size: f32,
    text_width: f32,
) -> BottomBarContentGeometry {
    let text_x = item.origin.x + (item.size.width - text_width) * 0.5;
    let text_height = font_size * TEXT_LINE_HEIGHT_FACTOR;
    BottomBarContentGeometry {
        icon_origin: shows_icon.then(|| {
            Point::new(
                item.origin.x + (item.size.width - icon_size) * 0.5,
                item.origin.y + BOTTOM_BAR_ICON_TOP_INSET,
            )
        }),
        text_origin: Point::new(
            text_x,
            if shows_icon {
                item.origin.y + item.size.height - text_height - BOTTOM_BAR_TEXT_BOTTOM_INSET
            } else {
                item.origin.y + (item.size.height - text_height) * 0.5
            },
        ),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct NavigationGeometry {
    navigation: Rect,
    header: Option<Rect>,
    content: Rect,
}

fn navigation_geometry(
    size: Size,
    presentation: NavigationPresentation,
    sidebar_width: f32,
    header_height: f32,
    bottom_bar_height: f32,
) -> NavigationGeometry {
    let width = size.width.max(0.0);
    let height = size.height.max(0.0);
    let header_height = header_height.max(0.0).min(height);
    match presentation {
        NavigationPresentation::BottomBar => {
            let bottom_height = bottom_bar_height
                .max(0.0)
                .min((height - header_height).max(0.0));
            NavigationGeometry {
                navigation: Rect::from_xywh(0.0, height - bottom_height, width, bottom_height),
                header: (header_height > 0.0)
                    .then(|| Rect::from_xywh(0.0, 0.0, width, header_height)),
                content: Rect::from_xywh(
                    0.0,
                    header_height,
                    width,
                    (height - header_height - bottom_height).max(0.0),
                ),
            }
        }
        NavigationPresentation::Automatic | NavigationPresentation::Sidebar => {
            let sidebar_width = sidebar_width.max(0.0).min(width);
            let content_width = (width - sidebar_width).max(0.0);
            NavigationGeometry {
                navigation: Rect::from_xywh(0.0, 0.0, sidebar_width, height),
                header: (header_height > 0.0)
                    .then(|| Rect::from_xywh(sidebar_width, 0.0, content_width, header_height)),
                content: Rect::from_xywh(
                    sidebar_width,
                    header_height,
                    content_width,
                    (height - header_height).max(0.0),
                ),
            }
        }
    }
}

impl NavigationViewRenderObject {
    /// Create a navigation layout render object.
    ///
    /// # Arguments
    ///
    /// * `sidebar_width` - Fixed width reserved for the sidebar child.
    /// * `header_height` - Height reserved above the content child.
    ///
    /// # Returns
    ///
    /// A render object ready to lay out navigation children.
    pub fn new(sidebar_width: f32, header_height: f32) -> Self {
        Self::new_with_presentation_and_content_size(
            sidebar_width,
            header_height,
            NavigationPresentation::Sidebar,
            Size::new(320.0, 240.0),
        )
    }

    /// Create a navigation layout render object with a presentation policy.
    ///
    /// # Arguments
    ///
    /// * `sidebar_width` - Fixed width reserved by sidebar presentation.
    /// * `header_height` - Height reserved above the content child.
    /// * `presentation` - Adaptive or forced navigation presentation.
    ///
    /// # Returns
    ///
    /// A configured render object ready to lay out navigation children.
    pub fn new_with_presentation(
        sidebar_width: f32,
        header_height: f32,
        presentation: NavigationPresentation,
    ) -> Self {
        Self::new_with_presentation_and_content_size(
            sidebar_width,
            header_height,
            presentation,
            Size::new(320.0, 240.0),
        )
    }

    /// Create a navigation layout with explicit adaptive content requirements.
    ///
    /// # Arguments
    ///
    /// * `sidebar_width` - Fixed width reserved by sidebar presentation.
    /// * `header_height` - Height reserved above the content child.
    /// * `presentation` - Adaptive or forced navigation presentation.
    /// * `minimum_content_size` - Smallest useful selected-content area.
    ///
    /// # Returns
    ///
    /// A configured render object ready for per-window adaptive layout.
    pub fn new_with_presentation_and_content_size(
        sidebar_width: f32,
        header_height: f32,
        presentation: NavigationPresentation,
        minimum_content_size: Size,
    ) -> Self {
        Self {
            sidebar_width,
            header_height: header_height.max(0.0),
            presentation,
            minimum_content_size,
            size: Size::ZERO,
        }
    }

    /// Update navigation layout configuration.
    pub(crate) fn update_configuration(
        &mut self,
        sidebar_width: f32,
        header_height: f32,
        presentation: NavigationPresentation,
        minimum_content_size: Size,
    ) {
        self.sidebar_width = sidebar_width;
        self.header_height = header_height.max(0.0);
        self.presentation = presentation;
        self.minimum_content_size = minimum_content_size;
    }
}

impl ElementRenderObject for NavigationViewRenderObject {
    fn update_needs_layout(&self) -> bool {
        true
    }

    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.size = Size::new(
            constraints.min_width.min(constraints.max_width),
            constraints.min_height.min(constraints.max_height),
        );
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        let has_header = self.header_height > 0.0;
        let content_index = if has_header { 2 } else { 1 };
        if children.len() != content_index + 1 {
            crate::logln!(
                "[NavigationViewRenderObject::layout] WARNING: expected {} children, got {}",
                content_index + 1,
                children.len()
            );
        }

        let size = Size::new(
            constraints.max_width.max(0.0),
            constraints.max_height.max(0.0),
        );
        let resolved_bottom_bar_height = bottom_bar_height(style::metrics().navigation_item_height);
        let presentation = self.presentation.resolve_for_size(
            size,
            self.sidebar_width,
            self.minimum_content_size,
            resolved_bottom_bar_height,
        );
        let geometry = navigation_geometry(
            size,
            presentation,
            self.sidebar_width,
            self.header_height,
            resolved_bottom_bar_height,
        );
        if let Some(navigation) = children.get_mut(0) {
            if let Some(render_object) = navigation.render_object_mut()
                && let Some(navigation_render_object) = render_object
                    .as_any_mut()
                    .downcast_mut::<NavigationSidebarRenderObject>(
                )
            {
                navigation_render_object.set_presentation(presentation);
            }
            navigation.layout(LayoutConstraints::tight(
                geometry.navigation.size.width,
                geometry.navigation.size.height,
            ));
            navigation.set_position(geometry.navigation.origin);
        }

        if has_header && let Some(header) = children.get_mut(1) {
            let header_geometry = geometry
                .header
                .unwrap_or_else(|| Rect::from_xywh(0.0, 0.0, 0.0, 0.0));
            header.layout(LayoutConstraints::tight(
                header_geometry.size.width,
                header_geometry.size.height,
            ));
            header.set_position(header_geometry.origin);
        }

        if let Some(content) = children.get_mut(content_index) {
            content.layout(LayoutConstraints::new(
                geometry.content.size.width,
                geometry.content.size.width,
                0.0,
                geometry.content.size.height,
            ));
            content.set_position(geometry.content.origin);
        }

        self.size = size;
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn render(&mut self) {}

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Declarative child view for the interactive navigation sidebar.
#[derive(Clone)]
pub(crate) struct NavigationSidebar {
    labels: Vec<String>,
    icons: Vec<Option<Icon>>,
    selection_callbacks: Vec<Option<Rc<dyn Fn()>>>,
    selected_index: State<usize>,
    shows_icons: bool,
    icon_style: IconStyle,
    icon_color: Option<Color>,
    presentation: NavigationPresentation,
    sidebar_style: NavigationSidebarStyle,
}

impl NavigationSidebar {
    pub(crate) fn new(
        labels: Vec<String>,
        icons: Vec<Option<Icon>>,
        selection_callbacks: Vec<Option<Rc<dyn Fn()>>>,
        selected_index: State<usize>,
        shows_icons: bool,
        icon_style: IconStyle,
        icon_color: Option<Color>,
        presentation: NavigationPresentation,
    ) -> Self {
        Self {
            labels,
            icons,
            selection_callbacks,
            selected_index,
            shows_icons,
            icon_style,
            icon_color,
            presentation,
            sidebar_style: NavigationSidebarStyle::Simple,
        }
    }
}

impl NavigationSidebar {
    pub(crate) fn sidebar_style(mut self, style: NavigationSidebarStyle) -> Self {
        self.sidebar_style = style;
        self
    }
}

impl View for NavigationSidebar {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::new_with_builder(
            self.clone(),
            NavigationSidebarRenderObject::from_view,
        ))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Paint and interaction render object for the sidebar region.
pub(crate) struct NavigationSidebarRenderObject {
    labels: Vec<String>,
    icons: Vec<Option<Icon>>,
    selection_callbacks: Vec<Option<Rc<dyn Fn()>>>,
    selected_index: State<usize>,
    shows_icons: bool,
    icon_style: IconStyle,
    icon_color: Option<Color>,
    hovered_index: Option<usize>,
    last_painted_hovered_index: Cell<Option<usize>>,
    item_height: f32,
    size: Size,
    buffer: Option<Buffer>,
    font_size: f32,
    icon_size: u32,
    item_padding: f32,
    presentation: NavigationPresentation,
    sidebar_style: NavigationSidebarStyle,
}

impl NavigationSidebarRenderObject {
    fn from_view(view: &NavigationSidebar) -> Self {
        Self {
            labels: view.labels.clone(),
            icons: view.icons.clone(),
            selection_callbacks: view.selection_callbacks.clone(),
            selected_index: view.selected_index.clone(),
            shows_icons: view.shows_icons,
            icon_style: view.icon_style,
            icon_color: view.icon_color,
            hovered_index: None,
            last_painted_hovered_index: Cell::new(None),
            item_height: style::metrics().navigation_item_height,
            size: Size::ZERO,
            buffer: None,
            font_size: 14.0,
            icon_size: 16,
            item_padding: 8.0,
            presentation: view.presentation,
            sidebar_style: view.sidebar_style,
        }
    }

    pub(crate) fn hovered_index(&self) -> Option<usize> {
        self.hovered_index
    }

    pub(crate) fn set_hovered_index(&mut self, index: Option<usize>) {
        self.hovered_index = index;
    }

    pub(crate) fn set_presentation(&mut self, presentation: NavigationPresentation) {
        self.presentation = presentation;
    }

    pub(crate) fn index_at_point(&self, x: f32, y: f32) -> Option<usize> {
        if x < 0.0 || x >= self.size.width || y < 0.0 {
            return None;
        }
        if y >= self.size.height {
            return None;
        }
        if self.is_floating() {
            if x < 16.0 || x >= self.size.width - 16.0 || y < 16.0 {
                return None;
            }
            let index = ((y - 16.0) / 40.0) as usize;
            return (index < self.labels.len()
                && y < 16.0 + index as f32 * 40.0 + 36.0
                && y < self.size.height - 16.0)
                .then_some(index);
        }
        let index = match self.presentation {
            NavigationPresentation::BottomBar => {
                if self.labels.is_empty() {
                    return None;
                }
                let item_width = self.size.width / self.labels.len() as f32;
                (x / item_width) as usize
            }
            NavigationPresentation::Automatic | NavigationPresentation::Sidebar => {
                (y / self.item_height) as usize
            }
        };
        (index < self.labels.len()).then_some(index)
    }

    pub(crate) fn selected_index(&self) -> &State<usize> {
        &self.selected_index
    }

    pub(crate) fn selection_callback(&self, index: usize) -> Option<Rc<dyn Fn()>> {
        self.selection_callbacks.get(index).cloned().flatten()
    }

    fn is_floating(&self) -> bool {
        self.sidebar_style == NavigationSidebarStyle::Floating
            && self.presentation != NavigationPresentation::BottomBar
    }

    fn is_glass_bottom_bar(&self) -> bool {
        self.sidebar_style == NavigationSidebarStyle::Floating
            && self.presentation == NavigationPresentation::BottomBar
    }

    fn paint_glass_bottom_bar(&self, ctx: &mut PaintContext<'_>, origin: Point) {
        let palette = ColorPalette::default();
        let panel = Rect::new(origin, self.size);
        let radius = style::radius_for(panel, 8.0);
        let mut material = style::GlassMaterial::regular();
        material.tint = palette.text().with_opacity(0.045).blend_over(material.tint);
        let shadow = palette.shadow();
        style::elevation_shadow(
            ctx,
            panel,
            radius,
            shadow.with_opacity(shadow.a * 1.75),
            style::ElevationRole::Raised,
        );
        ctx.push_rounded_clip(panel, radius);
        style::paint_glass_material(ctx, panel, radius, material);
        let item_width = self.size.width / self.labels.len().max(1) as f32;
        for (index, label) in self.labels.iter().enumerate() {
            let item = Rect::from_xywh(
                origin.x + index as f32 * item_width,
                origin.y,
                item_width,
                self.size.height,
            );
            let selected = self.selected_index.get() == index;
            let face_radius = style::radius_for(item, 6.0);
            // The upper edge meets the active destination. Only the lower,
            // exposed corners curve, matching a bottom-placed TabView.
            let face = Rect::from_xywh(
                item.origin.x,
                item.origin.y - face_radius,
                item.size.width,
                item.size.height + face_radius,
            );
            ctx.push_clip(item);
            if selected {
                let mut selected_material = style::GlassMaterial::regular();
                if self.hovered_index == Some(index) {
                    selected_material.tint = selected_material
                        .tint
                        .with_opacity((selected_material.tint.a + 0.04).min(1.0));
                }
                style::paint_glass_material(ctx, face, face_radius, selected_material);
                ctx.fill_rect(
                    Rect::from_xywh(
                        item.origin.x,
                        item.origin.y,
                        item.size.width,
                        style::metrics().navigation_indicator_width,
                    ),
                    palette.primary(),
                );
            } else if self.hovered_index == Some(index) {
                ctx.fill_rounded_rect(face, face_radius, palette.menu_hover());
            }
            let color = if selected {
                palette.primary()
            } else {
                palette.text()
            };
            let (text_width, _) = graphics::measure_text_sized(label, self.font_size);
            let icon = self
                .icons
                .get(index)
                .copied()
                .flatten()
                .filter(|_| self.shows_icons);
            let geometry = bottom_bar_content_geometry(
                item,
                icon.is_some(),
                self.icon_size as f32,
                self.font_size,
                text_width as f32,
            );
            if let (Some(icon), Some(icon_origin)) = (icon, geometry.icon_origin) {
                crate::views::icon::paint_icon(
                    ctx,
                    icon_origin,
                    self.icon_size as f32,
                    icon,
                    self.icon_style,
                    self.icon_color.unwrap_or(color),
                );
            }
            ctx.draw_text(geometry.text_origin, label.clone(), color, self.font_size);
            ctx.pop_clip();
        }
        ctx.pop_clip();
    }

    fn floating_row(&self, origin: Point, index: usize) -> Rect {
        Rect::from_xywh(
            origin.x + 16.0,
            origin.y + 16.0 + index as f32 * 40.0,
            (self.size.width - 32.0).max(0.0),
            36.0,
        )
    }

    fn paint_floating(&self, ctx: &mut PaintContext<'_>, origin: Point) {
        let palette = ColorPalette::default();
        let panel = Rect::from_xywh(
            origin.x + 8.0,
            origin.y + 8.0,
            (self.size.width - 16.0).max(0.0),
            (self.size.height - 16.0).max(0.0),
        );
        // Do not cover the backdrop with an opaque rail before sampling it.
        // Floating navigation uses the same real material as controls/surfaces.
        let mut material = style::GlassMaterial::regular();
        // A neutral structural tint remains visible over a plain white canvas,
        // while retaining real backdrop response and the existing geometry.
        material.tint = palette.text().with_opacity(0.045).blend_over(material.tint);
        style::paint_glass_material(ctx, panel, 16.0, material);
        // Clip destinations to the inset panel on short windows.
        let content = Rect::from_xywh(
            origin.x + 16.0,
            origin.y + 16.0,
            (self.size.width - 32.0).max(0.0),
            (self.size.height - 32.0).max(0.0),
        );
        ctx.push_clip(content);
        for index in 0..self.labels.len() {
            let row = self.floating_row(origin, index);
            let selected = self.selected_index.get() == index;
            if selected {
                ctx.fill_rounded_rect(row, 12.0, palette.primary().with_opacity(0.10));
            } else if self.hovered_index == Some(index) {
                ctx.fill_rounded_rect(row, 12.0, palette.menu_hover());
            }
            let color = if selected {
                palette.primary()
            } else {
                palette.text()
            };
            let mut text_x = row.origin.x + 12.0;
            if self.shows_icons {
                if let Some(icon) = self.icons.get(index).copied().flatten() {
                    crate::views::icon::paint_icon(
                        ctx,
                        Point::new(text_x, row.origin.y + 10.0),
                        16.0,
                        icon,
                        self.icon_style,
                        self.icon_color.unwrap_or(color),
                    );
                }
                // Reserve the same column even for destinations without an icon.
                text_x += 26.0;
            }
            let (_, text_height) =
                graphics::measure_text_sized(&self.labels[index], self.font_size);
            ctx.draw_text(
                Point::new(text_x, row.origin.y + (36.0 - text_height as f32) * 0.5),
                self.labels[index].to_owned(),
                color,
                self.font_size,
            );
        }
        ctx.pop_clip();
    }

    fn paint_commands(&self, ctx: &mut PaintContext<'_>, origin: Point) {
        if self.is_glass_bottom_bar() {
            self.paint_glass_bottom_bar(ctx, origin);
            return;
        }
        if self.is_floating() {
            self.paint_floating(ctx, origin);
            return;
        }
        let palette = ColorPalette::default();
        let metrics = style::metrics();
        let width = self.size.width.max(0.0);
        let height = self.size.height.max(0.0);
        ctx.fill_rect(
            Rect::from_xywh(origin.x, origin.y, width, height),
            style::surface_color(&palette, style::SurfaceRole::Structural),
        );

        let selected = self.selected_index.get();
        for index in 0..self.labels.len() {
            let is_bottom_bar = self.presentation == NavigationPresentation::BottomBar;
            let item_width = if is_bottom_bar && !self.labels.is_empty() {
                width / self.labels.len() as f32
            } else {
                width
            };
            let x = if is_bottom_bar {
                index as f32 * item_width
            } else {
                0.0
            };
            let y = if is_bottom_bar {
                0.0
            } else {
                index as f32 * self.item_height
            };
            let item_height = if is_bottom_bar {
                height
            } else {
                self.item_height
            };
            let is_selected = selected == index;
            if self.hovered_index == Some(index) {
                ctx.fill_rect(
                    Rect::from_xywh(origin.x + x, origin.y + y, item_width, item_height),
                    palette.menu_hover(),
                );
            }
            if is_selected {
                let indicator = if is_bottom_bar {
                    Rect::from_xywh(
                        origin.x + x,
                        origin.y,
                        item_width,
                        metrics.navigation_indicator_width,
                    )
                } else {
                    Rect::from_xywh(
                        origin.x,
                        origin.y + y,
                        metrics.navigation_indicator_width,
                        item_height,
                    )
                };
                ctx.fill_rect(indicator, palette.primary());
            }

            let text_color = if is_selected {
                palette.primary()
            } else {
                palette.text()
            };
            let (text_width, text_height) =
                graphics::measure_text_sized(&self.labels[index], self.font_size);
            let bottom_content = is_bottom_bar.then(|| {
                bottom_bar_content_geometry(
                    Rect::from_xywh(origin.x + x, origin.y + y, item_width, item_height),
                    self.shows_icons && self.icons.get(index).is_some_and(Option::is_some),
                    self.icon_size as f32,
                    self.font_size,
                    text_width as f32,
                )
            });
            let text_x = if let Some(content) = bottom_content {
                content.text_origin.x
            } else if self.shows_icons {
                if let Some(icon) = self.icons.get(index).copied().flatten() {
                    let icon_size = self.icon_size as f32;
                    crate::views::icon::paint_icon(
                        ctx,
                        Point::new(
                            origin.x + self.item_padding + 4.0,
                            origin.y + y + (self.item_height - icon_size) * 0.5,
                        ),
                        icon_size,
                        icon,
                        self.icon_style,
                        self.icon_color.unwrap_or(text_color),
                    );
                    origin.x + self.item_padding + icon_size + 10.0
                } else {
                    origin.x + self.item_padding + 8.0
                }
            } else {
                origin.x + self.item_padding + 8.0
            };
            if is_bottom_bar
                && self.shows_icons
                && let Some(icon) = self.icons.get(index).copied().flatten()
            {
                let icon_size = self.icon_size as f32;
                let icon_origin = bottom_content
                    .and_then(|content| content.icon_origin)
                    .unwrap_or(Point::new(origin.x + x, origin.y + y));
                crate::views::icon::paint_icon(
                    ctx,
                    icon_origin,
                    icon_size,
                    icon,
                    self.icon_style,
                    self.icon_color.unwrap_or(text_color),
                );
            }
            let text_y = if let Some(content) = bottom_content {
                content.text_origin.y
            } else {
                origin.y + y + (self.item_height - text_height as f32) * 0.5
            };
            ctx.draw_text(
                Point::new(text_x, text_y),
                self.labels[index].to_owned(),
                text_color,
                self.font_size,
            );
        }

        if width > 0.0 && self.presentation == NavigationPresentation::BottomBar {
            ctx.fill_rect(
                Rect::from_xywh(origin.x, origin.y, width, 1.0),
                palette.divider(),
            );
        } else if width > 0.0 {
            ctx.fill_rect(
                Rect::from_xywh(origin.x + width - 1.0, origin.y, 1.0, height),
                palette.divider(),
            );
        }
    }
}

impl ElementRenderObject for NavigationSidebarRenderObject {
    fn reads_backdrop(&self) -> bool {
        self.is_floating() || self.is_glass_bottom_bar()
    }

    fn paint_outsets(&self) -> EdgeInsets {
        if self.is_glass_bottom_bar() {
            style::elevation_outsets(style::ElevationRole::Raised)
        } else {
            EdgeInsets::ZERO
        }
    }

    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.size = Size::new(constraints.max_width, constraints.max_height);
        let width = libm::ceilf(self.size.width.max(0.0)) as u32;
        let height = libm::ceilf(self.size.height.max(0.0)) as u32;
        let needs_resize = self.buffer.as_ref().is_none_or(|buffer| {
            buffer.logical_width() != width || buffer.logical_height() != height
        });
        if needs_resize {
            self.buffer = Some(Buffer::from_logical_dimensions(width, height));
        }
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn self_paint_bounds(&self, origin: Point) -> Option<Rect> {
        if self.reads_backdrop() {
            return None;
        }
        let previous = self.last_painted_hovered_index.get();
        let current = self.hovered_index;
        let (first, last) = match (previous, current) {
            (Some(previous), Some(current)) => (previous.min(current), previous.max(current)),
            (Some(index), None) | (None, Some(index)) => (index, index),
            (None, None) => return Some(Rect::new(origin, self.size)),
        };
        if self.presentation == NavigationPresentation::BottomBar && !self.labels.is_empty() {
            let item_width = self.size.width.max(0.0) / self.labels.len() as f32;
            Some(Rect::from_xywh(
                origin.x + first as f32 * item_width,
                origin.y,
                (last - first + 1) as f32 * item_width,
                self.size.height.max(0.0),
            ))
        } else {
            Some(Rect::from_xywh(
                origin.x,
                origin.y + first as f32 * self.item_height,
                self.size.width.max(0.0),
                (last - first + 1) as f32 * self.item_height,
            ))
        }
    }

    fn update(&mut self, new_view: &dyn View) -> UpdateResult {
        let Some(view) = new_view.as_any().downcast_ref::<NavigationSidebar>() else {
            return UpdateResult::Replaced;
        };
        self.labels = view.labels.clone();
        self.icons = view.icons.clone();
        self.selection_callbacks = view.selection_callbacks.clone();
        self.selected_index = view.selected_index.clone();
        self.shows_icons = view.shows_icons;
        self.icon_style = view.icon_style;
        self.icon_color = view.icon_color;
        self.presentation = view.presentation;
        self.sidebar_style = view.sidebar_style;
        self.item_height = style::metrics().navigation_item_height;
        self.hovered_index = self
            .hovered_index
            .filter(|index| *index < self.labels.len());
        UpdateResult::Updated
    }

    fn render(&mut self) {
        if self.reads_backdrop() {
            if let Some(mut buffer) = self.buffer.take() {
                let mut ctx = PaintContext::new();
                self.paint_commands(&mut ctx, Point::ZERO);
                #[allow(deprecated)]
                crate::renderer::CpuPaintRenderer::execute_into_buffer(
                    &mut buffer,
                    Color::CLEAR,
                    &ctx,
                    None,
                );
                self.buffer = Some(buffer);
            }
            self.last_painted_hovered_index.set(self.hovered_index);
            return;
        }
        if let Some(buffer) = self.buffer.as_mut() {
            let palette = ColorPalette::default();
            let metrics = style::metrics();
            let width = buffer.logical_width();
            let height = buffer.logical_height();
            let selected = self.selected_index.get();
            {
                let mut canvas = graphics::Canvas::for_buffer(buffer);
                canvas.fill_rect(
                    0,
                    0,
                    width,
                    height,
                    style::surface_color(&palette, style::SurfaceRole::Structural),
                );
                for index in 0..self.labels.len() {
                    let is_bottom_bar = self.presentation == NavigationPresentation::BottomBar;
                    let item_width = if is_bottom_bar && !self.labels.is_empty() {
                        width as f32 / self.labels.len() as f32
                    } else {
                        width as f32
                    };
                    let item_x = if is_bottom_bar {
                        index as f32 * item_width
                    } else {
                        0.0
                    };
                    let item_y = if is_bottom_bar {
                        0.0
                    } else {
                        index as f32 * self.item_height
                    };
                    let item_height = if is_bottom_bar {
                        height as f32
                    } else {
                        self.item_height
                    };
                    let item_left = libm::floorf(item_x) as i32;
                    let item_top = libm::floorf(item_y) as i32;
                    let item_right = libm::ceilf(item_x + item_width) as i32;
                    let item_bottom = libm::ceilf(item_y + item_height) as i32;
                    let item_pixel_width = (item_right - item_left).max(0) as u32;
                    let item_pixel_height = (item_bottom - item_top).max(0) as u32;
                    if self.hovered_index == Some(index) {
                        canvas.fill_rect(
                            item_left,
                            item_top,
                            item_pixel_width,
                            item_pixel_height,
                            palette.menu_hover(),
                        );
                    }
                    if selected == index {
                        if is_bottom_bar {
                            canvas.fill_rect(
                                item_left,
                                0,
                                item_pixel_width,
                                metrics.navigation_indicator_width as u32,
                                palette.primary(),
                            );
                        } else {
                            canvas.fill_rect(
                                0,
                                item_top,
                                metrics.navigation_indicator_width as u32,
                                item_pixel_height,
                                palette.primary(),
                            );
                        }
                    }
                    let color = if selected == index {
                        palette.primary()
                    } else {
                        palette.text()
                    };
                    let (text_width, text_height) =
                        graphics::measure_text_sized(&self.labels[index], self.font_size);
                    let bottom_content = is_bottom_bar.then(|| {
                        bottom_bar_content_geometry(
                            Rect::from_xywh(item_x, item_y, item_width, item_height),
                            self.shows_icons && self.icons.get(index).copied().flatten().is_some(),
                            self.icon_size as f32,
                            self.font_size,
                            text_width as f32,
                        )
                    });
                    let text_origin = bottom_content.map_or_else(
                        || {
                            Point::new(
                                self.item_padding + 8.0,
                                item_y + (self.item_height - text_height as f32) * 0.5,
                            )
                        },
                        |content| content.text_origin,
                    );
                    canvas.draw_text_sized(
                        libm::roundf(text_origin.x) as i32,
                        libm::roundf(text_origin.y) as i32,
                        self.labels.get(index).map(String::as_str).unwrap_or("Item"),
                        color,
                        self.font_size,
                    );
                }
                if width > 0 && self.presentation == NavigationPresentation::BottomBar {
                    canvas.draw_line(0, 0, width as i32, 0, palette.divider());
                } else if width > 0 {
                    canvas.draw_line(
                        width as i32 - 1,
                        0,
                        width as i32 - 1,
                        height as i32,
                        palette.divider(),
                    );
                }
            }

            if self.presentation == NavigationPresentation::BottomBar
                && self.shows_icons
                && !self.labels.is_empty()
            {
                let scale_milli = buffer.scale_milli();
                let item_width = width as f32 / self.labels.len() as f32;
                let pixel_icon_size =
                    libm::ceilf(self.icon_size as f32 * scale_milli as f32 / 1_000.0) as u16;
                for index in 0..self.labels.len() {
                    let Some(icon) = self.icons.get(index).copied().flatten() else {
                        continue;
                    };
                    let (text_width, _) =
                        graphics::measure_text_sized(&self.labels[index], self.font_size);
                    let content = bottom_bar_content_geometry(
                        Rect::from_xywh(index as f32 * item_width, 0.0, item_width, height as f32),
                        true,
                        self.icon_size as f32,
                        self.font_size,
                        text_width as f32,
                    );
                    let Some(origin) = content.icon_origin else {
                        continue;
                    };
                    let raster =
                        crate::icon::rasterize_icon(icon, pixel_icon_size, self.icon_style);
                    let color = self.icon_color.unwrap_or_else(|| {
                        if selected == index {
                            palette.primary()
                        } else {
                            palette.text()
                        }
                    });
                    buffer.composite_alpha_mask(
                        &raster.mask,
                        raster.width,
                        raster.height,
                        libm::roundf(origin.x * scale_milli as f32 / 1_000.0) as i32,
                        libm::roundf(origin.y * scale_milli as f32 / 1_000.0) as i32,
                        color,
                        None,
                    );
                }
            }
        }
        self.last_painted_hovered_index.set(self.hovered_index);
    }

    fn paint(&self, ctx: &mut PaintContext<'_>, origin: Point) -> bool {
        self.paint_commands(ctx, origin);
        self.last_painted_hovered_index.set(self.hovered_index);
        true
    }

    fn get_buffer(&self) -> Option<&Buffer> {
        self.buffer.as_ref()
    }

    fn clear_buffer(&mut self) {
        self.buffer = None;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NavigationGeometry, NavigationPresentation, NavigationSidebar,
        NavigationSidebarRenderObject, NavigationViewRenderObject, bottom_bar_content_geometry,
        bottom_bar_height, navigation_geometry,
    };
    use crate::color::Color;
    use crate::element::{ElementRenderObject, LayoutConstraints};
    use crate::geometry::Point;
    use crate::geometry::{EdgeInsets, Rect, Size};
    use crate::icon::{Icon, IconStyle};
    use crate::renderer::PaintContext;
    use crate::state::State;
    use alloc::string::String;
    use alloc::vec;

    #[test]
    fn bottom_bar_geometry_reserves_header_content_and_bottom_regions() {
        let bottom_height =
            bottom_bar_height(crate::views::style::metrics().navigation_item_height);
        assert_eq!(
            navigation_geometry(
                Size::new(800.0, 600.0),
                NavigationPresentation::BottomBar,
                200.0,
                48.0,
                bottom_height,
            ),
            NavigationGeometry {
                navigation: Rect::from_xywh(0.0, 600.0 - bottom_height, 800.0, bottom_height),
                header: Some(Rect::from_xywh(0.0, 0.0, 800.0, 48.0)),
                content: Rect::from_xywh(0.0, 48.0, 800.0, 600.0 - 48.0 - bottom_height),
            }
        );
    }

    #[test]
    fn bottom_bar_height_stays_stable_across_posture_changes() {
        let pointer_height = {
            let _guard = crate::input_environment::install_test_input_environment(
                crate::InputEnvironment::desktop(),
            );
            bottom_bar_height(crate::views::style::metrics().navigation_item_height)
        };
        let touch_height = {
            let _guard = crate::input_environment::install_test_input_environment(
                crate::InputEnvironment::new(1, Some(true), None, true, false, false, false),
            );
            bottom_bar_height(crate::views::style::metrics().navigation_item_height)
        };

        assert_eq!(pointer_height, 52.0);
        assert_eq!(touch_height, pointer_height);
    }

    #[test]
    fn bottom_bar_content_geometry_centralizes_icon_and_text_alignment() {
        let geometry = bottom_bar_content_geometry(
            Rect::from_xywh(100.0, 20.0, 80.0, 64.0),
            true,
            16.0,
            14.0,
            28.0,
        );
        assert_eq!(
            geometry.icon_origin,
            Some(crate::geometry::Point::new(132.0, 28.0))
        );
        assert_eq!(geometry.text_origin.x, 126.0);
        assert!((geometry.text_origin.y - 62.2).abs() < 0.001);
    }

    #[test]
    fn legacy_constructor_preserves_fixed_sidebar_presentation() {
        let render_object = NavigationViewRenderObject::new(200.0, 0.0);
        assert_eq!(render_object.presentation, NavigationPresentation::Sidebar);
    }

    #[test]
    fn sidebar_geometry_keeps_header_and_content_after_navigation_rail() {
        assert_eq!(
            navigation_geometry(
                Size::new(800.0, 600.0),
                NavigationPresentation::Sidebar,
                200.0,
                48.0,
                bottom_bar_height(crate::views::style::metrics().navigation_item_height),
            ),
            NavigationGeometry {
                navigation: Rect::from_xywh(0.0, 0.0, 200.0, 600.0),
                header: Some(Rect::from_xywh(200.0, 0.0, 600.0, 48.0)),
                content: Rect::from_xywh(200.0, 48.0, 600.0, 552.0),
            }
        );
    }

    #[test]
    fn bottom_bar_hit_testing_uses_horizontal_destination_cells() {
        let view = NavigationSidebar::new(
            vec![
                String::from("One"),
                String::from("Two"),
                String::from("Three"),
            ],
            vec![None, None, None],
            vec![None, None, None],
            State::new(crate::state::generate_state_id(), 0),
            false,
            IconStyle::default(),
            None,
            NavigationPresentation::BottomBar,
        );
        let mut render_object = NavigationSidebarRenderObject::from_view(&view);
        let bottom_height =
            bottom_bar_height(crate::views::style::metrics().navigation_item_height);
        render_object.layout(LayoutConstraints::tight(300.0, bottom_height));

        assert_eq!(render_object.index_at_point(10.0, 30.0), Some(0));
        assert_eq!(render_object.index_at_point(150.0, 30.0), Some(1));
        assert_eq!(render_object.index_at_point(299.0, 30.0), Some(2));
        assert_eq!(render_object.index_at_point(300.0, 30.0), None);
        assert_eq!(render_object.index_at_point(10.0, bottom_height), None);
    }

    #[test]
    #[allow(deprecated)]
    fn legacy_bottom_bar_buffer_renders_configured_icon() {
        let build = |shows_icons| {
            NavigationSidebar::new(
                vec![String::new()],
                vec![Some(Icon::Home)],
                vec![None],
                State::new(crate::state::generate_state_id(), 99),
                shows_icons,
                IconStyle::default(),
                Some(Color::rgb(1.0, 0.0, 0.0)),
                NavigationPresentation::BottomBar,
            )
        };
        let bottom_height =
            bottom_bar_height(crate::views::style::metrics().navigation_item_height);
        let mut without_icon = NavigationSidebarRenderObject::from_view(&build(false));
        without_icon.layout(LayoutConstraints::tight(100.0, bottom_height));
        without_icon.render();
        let mut with_icon = NavigationSidebarRenderObject::from_view(&build(true));
        with_icon.layout(LayoutConstraints::tight(100.0, bottom_height));
        with_icon.render();

        assert_ne!(
            without_icon.get_buffer().expect("legacy buffer").data(),
            with_icon
                .get_buffer()
                .expect("legacy buffer with icon")
                .data()
        );
    }

    fn floating_sidebar() -> NavigationSidebar {
        NavigationSidebar::new(
            vec![String::from("One"), String::from("Two")],
            vec![Some(Icon::Home), None],
            vec![None, None],
            State::new(crate::state::generate_state_id(), 0),
            true,
            IconStyle::default(),
            None,
            NavigationPresentation::Sidebar,
        )
        .sidebar_style(super::NavigationSidebarStyle::Floating)
    }

    #[test]
    fn floating_sidebar_uses_real_backdrop_before_crisp_selection_and_labels() {
        let mut render = NavigationSidebarRenderObject::from_view(&floating_sidebar());
        render.layout(LayoutConstraints::tight(200.0, 200.0));
        assert!(render.reads_backdrop());
        let mut ctx = PaintContext::new();
        render.paint(&mut ctx, Point::ZERO);
        assert!(
            matches!(ctx.commands().first(), Some(crate::renderer::PaintCommand::BackdropFilterRoundedRect {
            rect, corner_radius, blur_radius, ..
        }) if *rect == Rect::from_xywh(8.0, 8.0, 184.0, 184.0)
            && *corner_radius == 16.0 && *blur_radius == 18.0)
        );
        assert!(
            !ctx.commands().iter().any(|command| matches!(
                command,
                crate::renderer::PaintCommand::FillPath { .. }
                    | crate::renderer::PaintCommand::StrokePath { .. }
                    | crate::renderer::PaintCommand::StrokeRoundedRect { .. }
                    | crate::renderer::PaintCommand::FillVerticalGradientRoundedRect { .. }
            )),
            "no opaque underlay, bevel or fixed decorative border"
        );
        render.set_presentation(NavigationPresentation::BottomBar);
        assert!(
            render.reads_backdrop(),
            "floating navigation keeps its glass material in portrait"
        );
        render.set_presentation(NavigationPresentation::Sidebar);
        let plain = floating_sidebar().sidebar_style(super::NavigationSidebarStyle::Simple);
        render.update(&plain);
        assert!(!render.reads_backdrop(), "Simple sidebar remains unchanged");
    }

    #[test]
    fn floating_sidebar_keeps_a_structural_surface_on_white() {
        let mut render = NavigationSidebarRenderObject::from_view(&floating_sidebar());
        render.layout(LayoutConstraints::tight(200.0, 200.0));
        let mut ctx = PaintContext::new();
        render.paint(&mut ctx, Point::ZERO);
        let mut renderer =
            crate::renderer::CpuPaintRenderer::new(Size::new(200.0, 200.0), 1000, Color::WHITE);
        renderer.execute(&ctx);
        let outside = renderer.buffer().get_pixel(0, 160).unwrap();
        let inside = renderer.buffer().get_pixel(100, 160).unwrap();
        assert_ne!(inside, outside, "panel must not disappear on a white page");
        let red = (inside >> 16) & 255;
        assert!(
            (238..=249).contains(&red),
            "structural tone remains restrained"
        );
    }

    #[test]
    fn floating_sidebar_samples_changes_underneath_the_panel() {
        let mut render = NavigationSidebarRenderObject::from_view(&floating_sidebar());
        render.layout(LayoutConstraints::tight(200.0, 200.0));
        let sample = |color| {
            let mut ctx = PaintContext::new();
            ctx.fill_rect(Rect::from_xywh(0.0, 0.0, 200.0, 200.0), color);
            render.paint(&mut ctx, Point::ZERO);
            let mut renderer =
                crate::renderer::CpuPaintRenderer::new(Size::new(200.0, 200.0), 1000, Color::WHITE);
            renderer.execute(&ctx);
            renderer.buffer().get_pixel(100, 160).unwrap()
        };
        assert_ne!(
            sample(Color::rgb(80, 130, 180)),
            sample(Color::rgb(180, 100, 80)),
            "floating material must show the current backdrop, not an opaque cached rail"
        );
    }

    #[test]
    fn floating_bottom_bar_shares_rounded_glass_and_preserves_destination_geometry() {
        use crate::renderer::PaintCommand;
        let mut render = NavigationSidebarRenderObject::from_view(&floating_sidebar());
        render.set_presentation(NavigationPresentation::BottomBar);
        render.layout(LayoutConstraints::tight(390.0, 52.0));
        assert!(render.reads_backdrop());
        assert_eq!(render.index_at_point(1.0, 1.0), Some(0));
        assert_eq!(render.index_at_point(389.0, 51.0), Some(1));
        let mut ctx = PaintContext::new();
        render.paint(&mut ctx, Point::ZERO);
        let glass: Vec<_> = ctx
            .commands()
            .iter()
            .filter_map(|command| match command {
                PaintCommand::BackdropFilterRoundedRect {
                    rect,
                    corner_radius,
                    blur_radius,
                    tint,
                } => Some((*rect, *corner_radius, *blur_radius, *tint)),
                _ => None,
            })
            .collect();
        assert_eq!(glass.len(), 2);
        assert_eq!(glass[0].0, Rect::from_xywh(0.0, 0.0, 390.0, 52.0));
        assert_eq!(glass[0].1, 8.0);
        assert_eq!(glass[1].0, Rect::from_xywh(0.0, -6.0, 195.0, 58.0));
        assert_eq!(glass[1].1, 6.0);
        assert!(glass.iter().all(|entry| entry.2 == 18.0 && entry.3.a < 1.0));
        assert!(matches!(
            ctx.commands().first(),
            Some(PaintCommand::DrawRoundedRectShadow { .. })
        ));
        assert_ne!(render.paint_outsets(), EdgeInsets::ZERO);
        assert!(!ctx.commands().iter().any(|command| matches!(
            command,
            PaintCommand::FillVerticalGradientRoundedRect { .. }
                | PaintCommand::StrokeRoundedRect { .. }
        )));
        let sample = |background| {
            let mut ctx = PaintContext::new();
            ctx.fill_rect(Rect::from_xywh(0.0, 0.0, 390.0, 52.0), background);
            render.paint(&mut ctx, Point::ZERO);
            let mut cpu =
                crate::renderer::CpuPaintRenderer::new(Size::new(390.0, 52.0), 1000, Color::WHITE);
            cpu.execute(&ctx);
            cpu.buffer().get_pixel(300, 42).unwrap()
        };
        assert_ne!(
            sample(Color::rgb(80, 130, 180)),
            sample(Color::rgb(180, 100, 80))
        );
        let simple = floating_sidebar().sidebar_style(super::NavigationSidebarStyle::Simple);
        render.update(&simple);
        render.set_presentation(NavigationPresentation::BottomBar);
        assert!(
            !render.reads_backdrop(),
            "Simple navigation remains unchanged"
        );
    }

    #[test]
    fn floating_sidebar_hit_regions_match_inset_rows_and_exclude_gaps() {
        let mut render = NavigationSidebarRenderObject::from_view(&floating_sidebar());
        render.layout(LayoutConstraints::tight(200.0, 500.0));
        assert_eq!(render.index_at_point(28.0, 20.0), Some(0));
        assert_eq!(render.index_at_point(28.0, 56.0), Some(1));
        assert_eq!(render.index_at_point(8.0, 20.0), None);
        assert_eq!(render.index_at_point(28.0, 8.0), None);
        assert_eq!(render.index_at_point(28.0, 52.0), None);
        assert_eq!(render.index_at_point(185.0, 20.0), None);
        render.set_presentation(NavigationPresentation::BottomBar);
        render.layout(LayoutConstraints::tight(200.0, 52.0));
        assert!(!render.is_floating());
        assert_eq!(render.index_at_point(8.0, 8.0), Some(0));
        assert_eq!(render.index_at_point(150.0, 8.0), Some(1));
    }

    #[test]
    #[allow(deprecated)]
    fn floating_sidebar_buffer_matches_retained_paint_and_accepts_style_updates() {
        let view = floating_sidebar();
        let mut render = NavigationSidebarRenderObject::from_view(&view);
        render.layout(LayoutConstraints::tight(200.0, 200.0));
        // Other tests change the process-wide UI scale. Fix both targets to
        // the same explicit scale instead of sampling that global twice.
        render.buffer = Some(crate::buffer::Buffer::from_logical_dimensions_with_scale(
            200, 200, 1000,
        ));
        render.render();
        let mut expected =
            crate::buffer::Buffer::from_logical_dimensions_with_scale(200, 200, 1000);
        let mut ctx = crate::renderer::PaintContext::new();
        render.paint(&mut ctx, crate::geometry::Point::ZERO);
        crate::renderer::CpuPaintRenderer::execute_into_buffer(
            &mut expected,
            Color::CLEAR,
            &ctx,
            None,
        );
        let actual = render.get_buffer().unwrap();
        assert_eq!(
            (actual.width(), actual.height()),
            (expected.width(), expected.height())
        );
        assert!(
            actual.data() == expected.data(),
            "floating sidebar CPU and retained pixels differ"
        );
        let plain = view.sidebar_style(super::NavigationSidebarStyle::Simple);
        render.update(&plain);
        assert!(!render.is_floating());
        assert_eq!(render.index_at_point(1.0, 1.0), Some(0));
    }

    #[test]
    fn sidebar_hit_testing_uses_vertical_destination_rows() {
        let view = NavigationSidebar::new(
            vec![String::from("One"), String::from("Two")],
            vec![None, None],
            vec![None, None],
            State::new(crate::state::generate_state_id(), 0),
            false,
            IconStyle::default(),
            None,
            NavigationPresentation::Sidebar,
        );
        let mut render_object = NavigationSidebarRenderObject::from_view(&view);
        render_object.layout(LayoutConstraints::tight(200.0, 500.0));

        assert_eq!(render_object.index_at_point(20.0, 10.0), Some(0));
        assert_eq!(
            render_object.index_at_point(20.0, render_object.item_height + 1.0),
            Some(1)
        );
        assert_eq!(
            render_object.index_at_point(20.0, render_object.item_height * 2.0),
            None
        );
        assert_eq!(render_object.index_at_point(-1.0, 10.0), None);
    }
}
