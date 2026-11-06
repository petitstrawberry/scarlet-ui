//! Content and optional window chrome on separate layout planes.
use crate::{
    element::{Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult},
    geometry::{EdgeInsets, Point, Rect, Size},
    view::View,
};
use alloc::{boxed::Box, vec, vec::Vec};
use core::any::Any;

/// A content viewport behind optional header and sidebar slots.
///
/// Fixed content is laid out in the unobstructed rectangle. With
/// [`Self::scrolling`], the viewport extends slightly beneath the chrome,
/// while ScrollView (including ListView/GridView) consumes the corresponding
/// safe-area reservations as content insets. Chrome paints after content and
/// receives pointer hits first. Slots may be omitted independently.
#[derive(Clone)]
pub struct ContentShell<C: View> {
    content: C,
    header: Option<Box<dyn View>>,
    sidebar: Option<Box<dyn View>>,
    header_height: f32,
    sidebar_width: f32,
    underlap: f32,
    scrolling: bool,
}

impl<C: View> ContentShell<C> {
    /// Lay out fixed content inside the usable rectangle.
    pub fn new(content: C) -> Self {
        Self {
            content,
            header: None,
            sidebar: None,
            header_height: 0.,
            sidebar_width: 0.,
            underlap: 16.,
            scrolling: false,
        }
    }

    /// Give a scrollport a larger paint viewport and safe content insets.
    /// The content must consume safe areas, as ScrollView does by default.
    pub fn scrolling(content: C) -> Self {
        Self {
            scrolling: true,
            ..Self::new(content)
        }
    }

    /// Install a header beside the sidebar, above the selected content.
    pub fn header(mut self, header: impl View + 'static, height: f32) -> Self {
        self.header = Some(Box::new(header));
        self.header_height = extent(height);
        self
    }

    /// Install a full-height sidebar. Its visual margin belongs to the view.
    pub fn sidebar(mut self, sidebar: impl View + 'static, width: f32) -> Self {
        self.sidebar = Some(Box::new(sidebar));
        self.sidebar_width = extent(width);
        self
    }

    /// Set how far the scrolling viewport extends under each occupied slot.
    /// Zero preserves a partitioned viewport; omitted slots reserve no space.
    pub fn underlap(mut self, points: f32) -> Self {
        self.underlap = extent(points);
        self
    }
}

fn extent(value: f32) -> f32 {
    if value.is_finite() { value.max(0.) } else { 0. }
}

impl<C: View + Clone + 'static> View for ContentShell<C> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |v| ContentShellRenderObject::from_view(v),
            |r, v| {
                r.header_height = if v.header.is_some() {
                    v.header_height
                } else {
                    0.
                };
                r.sidebar_width = if v.sidebar.is_some() {
                    v.sidebar_width
                } else {
                    0.
                };
                r.has_header = v.header.is_some();
                r.underlap = v.underlap;
                r.scrolling = v.scrolling;
                UpdateResult::Updated
            },
            |v| {
                let mut children = vec![v.content.clone_view()];
                if let Some(header) = &v.header {
                    children.push(header.clone());
                }
                if let Some(sidebar) = &v.sidebar {
                    children.push(sidebar.clone());
                }
                children
            },
        ))
    }
    fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
        // Each child retains its own subscriptions.
        Vec::new()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct ContentShellRenderObject {
    header_height: f32,
    sidebar_width: f32,
    has_header: bool,
    underlap: f32,
    scrolling: bool,
    insets: EdgeInsets,
    size: Size,
}
impl ContentShellRenderObject {
    fn from_view<C: View>(v: &ContentShell<C>) -> Self {
        Self {
            header_height: if v.header.is_some() {
                v.header_height
            } else {
                0.
            },
            sidebar_width: if v.sidebar.is_some() {
                v.sidebar_width
            } else {
                0.
            },
            has_header: v.header.is_some(),
            underlap: v.underlap,
            scrolling: v.scrolling,
            insets: EdgeInsets::ZERO,
            size: Size::ZERO,
        }
    }
}
impl ElementRenderObject for ContentShellRenderObject {
    fn set_safe_area_insets(&mut self, insets: EdgeInsets) {
        self.insets = insets;
    }
    fn child_safe_area_insets(&self, _: EdgeInsets) -> EdgeInsets {
        EdgeInsets::ZERO
    }
    fn update_needs_layout(&self) -> bool {
        true
    }
    fn layout(&mut self, c: LayoutConstraints) -> Size {
        self.size = Size::new(
            if c.max_width.is_finite() {
                c.max_width
            } else {
                c.min_width
            },
            if c.max_height.is_finite() {
                c.max_height
            } else {
                c.min_height
            },
        );
        self.size
    }
    fn layout_with_children(
        &mut self,
        c: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        let size = self.layout(c);
        let safe = crate::geometry::safe_area_rect(size, self.insets);
        let sidebar = self.sidebar_width.min(safe.size.width);
        let header = self.header_height.min(safe.size.height);
        let usable = Rect::from_xywh(
            safe.left() + sidebar,
            safe.top() + header,
            (safe.size.width - sidebar).max(0.),
            (safe.size.height - header).max(0.),
        );
        if let Some(content) = children.first_mut() {
            if self.scrolling {
                let left = if sidebar > 0. {
                    usable.left() - self.underlap.min(sidebar)
                } else {
                    0.
                };
                let top = if header > 0. {
                    usable.top() - self.underlap.min(header)
                } else {
                    0.
                };
                content.set_safe_area_insets(EdgeInsets::new(
                    usable.left() - left,
                    usable.top() - top,
                    (size.width - safe.right()).max(0.),
                    (size.height - safe.bottom()).max(0.),
                ));
                content.layout(LayoutConstraints::tight(
                    (size.width - left).max(0.),
                    (size.height - top).max(0.),
                ));
                content.set_position(Point::new(left, top));
            } else {
                content.set_safe_area_insets(EdgeInsets::ZERO);
                content.layout(LayoutConstraints::tight(
                    usable.size.width,
                    usable.size.height,
                ));
                content.set_position(usable.origin);
            }
        }
        if self.has_header
            && let Some(child) = children.get_mut(1)
        {
            child.set_safe_area_insets(EdgeInsets::ZERO);
            child.layout(LayoutConstraints::tight(usable.size.width, header));
            child.set_position(Point::new(usable.left(), safe.top()));
        }
        let sidebar_index = if self.has_header { 2 } else { 1 };
        if let Some(child) = children.get_mut(sidebar_index) {
            child.set_safe_area_insets(EdgeInsets::ZERO);
            child.layout(LayoutConstraints::tight(sidebar, safe.size.height));
            child.set_position(safe.origin);
        }
        size
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
