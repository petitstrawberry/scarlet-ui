//! State-backed grid primitives.
//!
//! `GridView` provides the layout and virtualization needed by icon-oriented
//! applications while leaving the item model and cell appearance to callers.
//! It is suitable for file managers, launchers, galleries, and media views.

use crate::element::{
    ComponentElement, Element, ElementId, ElementRenderObject, LayoutConstraints, RenderElement,
    UpdateResult,
};
use crate::geometry::{Point, Rect, Size};
use crate::pipeline::MountContext;
use crate::state::{Listenable, State};
use crate::view::View;
use crate::views::{ScrollView, ScrollbarVisibility, Spacer};
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::Any;

type CellBuilder<T> = Rc<dyn Fn(usize, T, Option<usize>) -> Box<dyn View>>;

/// A state-backed, vertically scrollable grid.
///
/// The grid creates fixed-size rows and delegates each cell's content to the
/// caller. The selected index is optional so the same component can be used as
/// a selectable file view or as a plain icon gallery. With
/// [`GridView::minimum_cell_width`], the configured column count is treated as
/// a maximum and the grid wraps items when the available width is smaller.
#[derive(Clone)]
pub struct GridView<T: Clone + 'static> {
    items: State<Vec<T>>,
    selected: State<Option<usize>>,
    columns: usize,
    row_height: f32,
    spacing: f32,
    row_spacing: f32,
    minimum_cell_width: Option<f32>,
    cell_builder: CellBuilder<T>,
    content_bottom_inset: f32,
    item_key: Option<Rc<dyn Fn(&T) -> crate::view::ViewKey>>,
}

impl<T: Clone + 'static> GridView<T> {
    /// Create a grid view.
    ///
    /// # Arguments
    ///
    /// * `items` - Reactive collection displayed by the grid.
    /// * `selected` - Reactive optional selected item index.
    /// * `columns` - Maximum number of cells in each row.
    /// * `row_height` - Logical height allocated to each row.
    /// * `cell_builder` - Closure producing a cell view.
    ///
    /// # Returns
    ///
    /// A virtualized grid view.
    pub fn new<V>(
        items: State<Vec<T>>,
        selected: State<Option<usize>>,
        columns: usize,
        row_height: f32,
        cell_builder: impl Fn(usize, T, Option<usize>) -> V + 'static,
    ) -> Self
    where
        V: View + 'static,
    {
        Self {
            items,
            selected,
            columns: columns.max(1),
            row_height: row_height.max(1.0),
            spacing: 12.0,
            row_spacing: 0.0,
            minimum_cell_width: None,
            cell_builder: Rc::new(move |index, item, selected| {
                Box::new(cell_builder(index, item, selected))
            }),
            content_bottom_inset: 0.0,
            item_key: None,
        }
    }

    /// Stable model identities for scroll anchoring through collection changes.
    pub fn item_key(mut self, key: impl Fn(&T) -> crate::view::ViewKey + 'static) -> Self {
        self.item_key = Some(Rc::new(key));
        self
    }

    /// Add scrollable space below the final item for floating controls.
    /// Unlike outer padding, this keeps the viewport behind the overlay.
    pub fn content_bottom_inset(mut self, inset: f32) -> Self {
        self.content_bottom_inset = if inset.is_finite() {
            inset.max(0.0)
        } else {
            0.0
        };
        self
    }

    /// Set the horizontal spacing between cells.
    ///
    /// # Arguments
    ///
    /// * `spacing` - Logical spacing between neighboring cells.
    ///
    /// # Returns
    ///
    /// The updated grid view.
    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing.max(0.0);
        self
    }

    /// Set the vertical spacing between rows.
    ///
    /// # Arguments
    ///
    /// * `spacing` - Logical spacing between neighboring rows.
    ///
    /// # Returns
    ///
    /// The updated grid view.
    pub fn row_spacing(mut self, spacing: f32) -> Self {
        self.row_spacing = spacing.max(0.0);
        self
    }

    /// Set the minimum logical width of a grid cell.
    ///
    /// When configured, the grid keeps this width and reduces the number of
    /// columns as the available width shrinks. Items that no longer fit are
    /// placed in the next row instead of being squeezed into narrower cells.
    ///
    /// # Arguments
    ///
    /// * `width` - Minimum logical width of each cell.
    ///
    /// # Returns
    ///
    /// The updated grid view.
    pub fn minimum_cell_width(mut self, width: f32) -> Self {
        self.minimum_cell_width = width.is_finite().then(|| width.max(1.0));
        self
    }

    /// Return the item state displayed by the grid.
    pub fn items(&self) -> &State<Vec<T>> {
        &self.items
    }

    /// Return the optional selected-item state.
    pub fn selected(&self) -> &State<Option<usize>> {
        &self.selected
    }

    /// Return the number of columns in each row.
    pub fn columns(&self) -> usize {
        self.columns
    }

    /// Return the row height in logical pixels.
    pub fn row_height(&self) -> f32 {
        self.row_height
    }
}

#[derive(Clone)]
struct AnyView {
    view: Rc<Box<dyn View>>,
}

impl AnyView {
    fn new(view: Box<dyn View>) -> Self {
        Self {
            view: Rc::new(view),
        }
    }
}

impl View for AnyView {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(
            self.clone(),
            build_any_view,
        ))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        self.view.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn build_any_view(view: &AnyView) -> Box<dyn View> {
    view.view.as_ref().as_ref().clone_view()
}

#[derive(Clone)]
struct GridRow {
    cells: Vec<AnyView>,
    row_height: f32,
    spacing: f32,
    minimum_cell_width: Option<f32>,
}

impl GridRow {
    fn new(
        cells: Vec<AnyView>,
        row_height: f32,
        spacing: f32,
        minimum_cell_width: Option<f32>,
    ) -> Self {
        Self {
            cells,
            row_height,
            spacing,
            minimum_cell_width,
        }
    }
}

impl View for GridRow {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children(
            self.clone(),
            |view| {
                GridRowRenderObject::new(
                    view.cells.len(),
                    view.row_height,
                    view.spacing,
                    view.minimum_cell_width,
                )
            },
            |view| {
                view.cells
                    .iter()
                    .map(|cell| Box::new(cell.clone()) as Box<dyn View>)
                    .collect()
            },
        ))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        let mut listenables = Vec::new();
        for cell in &self.cells {
            listenables.extend(cell.listenables());
        }
        listenables
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct GridRowRenderObject {
    cell_count: usize,
    row_height: f32,
    spacing: f32,
    minimum_cell_width: Option<f32>,
    size: Size,
}

impl GridRowRenderObject {
    fn new(
        cell_count: usize,
        row_height: f32,
        spacing: f32,
        minimum_cell_width: Option<f32>,
    ) -> Self {
        Self {
            cell_count,
            row_height,
            spacing,
            minimum_cell_width,
            size: Size::ZERO,
        }
    }
}

impl ElementRenderObject for GridRowRenderObject {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        let width = if constraints.max_width.is_finite() {
            constraints.max_width.max(constraints.min_width)
        } else {
            constraints.min_width
        };
        let height = self
            .row_height
            .clamp(constraints.min_height, constraints.max_height);
        self.size = Size::new(width, height);
        self.size
    }

    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        let size = self.layout(constraints);
        let count = self.cell_count.max(children.len()).max(1);
        let total_spacing = self.spacing * count.saturating_sub(1) as f32;
        let available_width = (size.width - total_spacing).max(0.0);
        let cell_width = match self.minimum_cell_width {
            Some(minimum) => (available_width / count as f32).max(minimum),
            None => (available_width / count as f32).max(0.0),
        };

        for (index, child) in children.iter_mut().enumerate() {
            child.layout(LayoutConstraints::tight(cell_width, size.height));
            child.set_position(Point::new(index as f32 * (cell_width + self.spacing), 0.0));
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

const GRID_CACHE_EXTENT: f32 = 512.0;

#[derive(Clone)]
struct GridContentView<T: Clone + 'static> {
    grid: GridView<T>,
}

impl<T: Clone + 'static> View for GridContentView<T> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(GridContentElement::new(self.grid.clone()))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        self.grid.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct GridContentElement<T: Clone + 'static> {
    id: ElementId,
    view: GridView<T>,
    items: Vec<T>,
    children: Vec<Box<dyn Element>>,
    child_indices: Vec<usize>,
    position: Point,
    size: Size,
    current_columns: usize,
    materialized_columns: usize,
    viewport_hint: Option<Rect>,
    last_constraints: Option<LayoutConstraints>,
    mount_context: Option<MountContext>,
    children_need_update: bool,
    children_needing_layout: Vec<usize>,
}

impl<T: Clone + 'static> GridContentElement<T> {
    fn new(view: GridView<T>) -> Self {
        let items = view.items.get();
        Self {
            id: ElementId::generate(),
            view,
            items,
            children: Vec::new(),
            child_indices: Vec::new(),
            position: Point::ZERO,
            size: Size::ZERO,
            current_columns: 0,
            materialized_columns: 0,
            viewport_hint: None,
            last_constraints: None,
            mount_context: None,
            children_need_update: false,
            children_needing_layout: Vec::new(),
        }
    }

    fn width_from_constraints(constraints: LayoutConstraints) -> f32 {
        if constraints.max_width.is_finite() {
            constraints.max_width.max(constraints.min_width)
        } else {
            constraints.min_width.max(0.0)
        }
    }

    fn columns_for_width(&self, width: f32) -> usize {
        let maximum = self.view.columns.max(1);
        let Some(minimum) = self.view.minimum_cell_width else {
            return maximum;
        };
        if !width.is_finite() {
            return maximum;
        }

        let stride = minimum + self.view.spacing;
        if !stride.is_finite() || stride <= 0.0 {
            return 1;
        }

        (libm::floorf((width + self.view.spacing) / stride) as usize).clamp(1, maximum)
    }

    fn row_count(&self, columns: usize) -> usize {
        self.items.len().div_ceil(columns.max(1))
    }

    fn row_stride(&self) -> f32 {
        self.view.row_height + self.view.row_spacing
    }

    fn content_height(&self, row_count: usize) -> f32 {
        if row_count == 0 {
            0.0
        } else {
            row_count as f32 * self.view.row_height
                + row_count.saturating_sub(1) as f32 * self.view.row_spacing
        }
    }

    fn build_row(&self, row_index: usize, columns: usize) -> Box<dyn View> {
        let items = &self.items;
        let selected = self.view.selected.get();
        let mut cells = Vec::with_capacity(columns);
        for column in 0..columns {
            let index = row_index * columns + column;
            if let Some(item) = items.get(index).cloned() {
                cells.push(AnyView::new((self.view.cell_builder)(
                    index, item, selected,
                )));
            } else {
                cells.push(AnyView::new(Box::new(Spacer::new())));
            }
        }
        Box::new(GridRow::new(
            cells,
            self.view.row_height,
            self.view.spacing,
            self.view.minimum_cell_width,
        ))
    }

    fn visible_range(&self, row_count: usize) -> core::ops::Range<usize> {
        if row_count == 0 {
            return 0..0;
        }

        let viewport = self.viewport_hint.unwrap_or_else(|| {
            Rect::from_xywh(
                0.0,
                0.0,
                self.size.width,
                GRID_CACHE_EXTENT.max(self.row_stride()),
            )
        });
        let row_stride = self.row_stride().max(1.0);
        let start_y = (viewport.top() - GRID_CACHE_EXTENT).max(0.0);
        let end_y = (viewport.bottom() + GRID_CACHE_EXTENT).max(start_y);
        let start = (libm::floorf(start_y / row_stride) as usize).min(row_count);
        let end = (libm::ceilf(end_y / row_stride) as usize + 1).min(row_count);
        start..end.max(start)
    }

    fn materialize_visible_children(&mut self) -> bool {
        let columns = self.current_columns.max(1);
        let range = self.visible_range(self.row_count(columns));
        let columns_changed = self.materialized_columns != columns;
        if self.child_indices.len() == range.len()
            && self.child_indices.first().copied() == range.clone().next()
            && !columns_changed
            && !self.children_need_update
        {
            return false;
        }
        let update_existing = columns_changed || self.children_need_update;
        let old_children = core::mem::take(&mut self.children);
        let old_indices = core::mem::take(&mut self.child_indices);
        let mut old = old_indices.into_iter().zip(old_children).peekable();
        self.children_needing_layout.clear();

        let mut new_children = Vec::with_capacity(range.len());
        let mut new_indices = Vec::with_capacity(range.len());
        let mut changed = columns_changed;
        // Both index streams are sorted. Merge them without repeatedly shifting
        // or searching every retained row when the viewport advances.
        for index in range {
            while old.peek().is_some_and(|(old_index, _)| *old_index < index) {
                let (_, mut child) = old.next().unwrap();
                child.unmount();
                changed = true;
            }
            if old.peek().is_some_and(|(old_index, _)| *old_index == index) {
                let (_, old_child) = old.next().unwrap();
                if update_existing {
                    self.children_needing_layout.push(index);
                    let row_view = self.build_row(index, columns);
                    let mut child = Some(old_child);
                    let result = crate::element::update_child(
                        &mut child,
                        Some(row_view.as_ref()),
                        self.mount_context,
                    );
                    if !matches!(result, UpdateResult::NoChange) {
                        changed = true;
                    }
                    if let Some(child) = child {
                        new_children.push(child);
                    }
                } else {
                    new_children.push(old_child);
                }
            } else {
                self.children_needing_layout.push(index);
                let row_view = self.build_row(index, columns);
                let mut child = row_view.create_element();
                if let Some(context) = self.mount_context {
                    child.mount(&context);
                }
                new_children.push(child);
                changed = true;
            }
            new_indices.push(index);
        }

        for (_, mut child) in old {
            changed = true;
            child.unmount();
        }
        self.children = new_children;
        self.child_indices = new_indices;
        self.materialized_columns = columns;
        self.children_need_update = false;
        changed
    }

    fn layout_visible_children(&mut self, only_new: bool) {
        let row_stride = self.row_stride();
        for (child, row_index) in self
            .children
            .iter_mut()
            .zip(self.child_indices.iter().copied())
        {
            if !only_new
                || self
                    .children_needing_layout
                    .binary_search(&row_index)
                    .is_ok()
            {
                child.layout(LayoutConstraints::tight(
                    self.size.width,
                    self.view.row_height,
                ));
            }
            child.set_position(Point::new(0.0, row_index as f32 * row_stride));
        }
    }
}

impl<T: Clone + 'static> Element for GridContentElement<T> {
    fn id(&self) -> ElementId {
        self.id
    }

    fn type_name(&self) -> &str {
        "GridContentElement"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn retains_virtual_items(&self) -> bool {
        true
    }

    fn scroll_anchor(&self, viewport: Rect) -> Option<crate::element::ScrollAnchor> {
        let key = self.view.item_key.as_ref()?;
        let columns = self.current_columns.max(1);
        let row = libm::floorf(viewport.origin.y / self.row_stride()).max(0.) as usize;
        let index = (row * columns).min(self.items.len().checked_sub(1)?);
        Some(crate::element::ScrollAnchor {
            key: key(&self.items[index]),
            position: Point::new(0., (index / columns) as f32 * self.row_stride()),
        })
    }
    fn resolve_scroll_anchor(&self, key: &crate::view::ViewKey) -> Option<Point> {
        let key_for = self.view.item_key.as_ref()?;
        let index = self.items.iter().position(|item| key_for(item) == *key)?;
        Some(Point::new(
            0.,
            (index / self.current_columns.max(1)) as f32 * self.row_stride(),
        ))
    }

    fn children(&self) -> &[Box<dyn Element>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Element>] {
        &mut self.children
    }

    fn update(&mut self, new_view: &dyn View) -> UpdateResult {
        let Some(new_view) = new_view.as_any().downcast_ref::<GridContentView<T>>() else {
            return UpdateResult::Replaced;
        };
        self.view = new_view.grid.clone();
        self.items = self.view.items.get();
        self.children_need_update = true;
        if let Some(constraints) = self.last_constraints {
            self.layout(constraints);
        }
        UpdateResult::Updated
    }

    fn rebuild(&mut self) -> UpdateResult {
        UpdateResult::NoChange
    }

    fn mount(&mut self, context: &MountContext) {
        self.mount_context = Some(*context);
        for child in &mut self.children {
            child.mount(context);
        }
    }

    fn unmount(&mut self) {
        for child in &mut self.children {
            child.unmount();
        }
        self.mount_context = None;
    }

    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.last_constraints = Some(constraints);
        let width = Self::width_from_constraints(constraints);
        self.current_columns = self.columns_for_width(width);
        let row_count = self.row_count(self.current_columns);
        self.size = Size::new(width, self.content_height(row_count));
        self.materialize_visible_children();
        self.layout_visible_children(false);
        self.size
    }

    fn last_layout_constraints(&self) -> Option<LayoutConstraints> {
        self.last_constraints
    }

    fn set_last_layout_constraints(&mut self, constraints: LayoutConstraints) {
        self.last_constraints = Some(constraints);
    }

    fn position(&self) -> Point {
        self.position
    }

    fn set_position(&mut self, position: Point) {
        self.position = position;
    }

    fn set_viewport_hint(&mut self, viewport: Rect) -> bool {
        if self.viewport_hint == Some(viewport) {
            return false;
        }
        self.viewport_hint = Some(viewport);
        let changed = self.materialize_visible_children();
        if changed {
            // Scrolling changes position, not the constraints of retained rows.
            self.layout_visible_children(true);
        }
        changed
    }

    fn bounds(&self) -> Rect {
        Rect::new(self.position, self.size)
    }

    fn hit_test(&self, point: Point) -> bool {
        self.bounds().contains(point)
    }

    fn clear_buffers(&mut self) {
        for child in &mut self.children {
            child.clear_buffers();
        }
    }
}

fn build_grid_child<T: Clone + 'static>(view: &GridView<T>) -> Box<dyn View> {
    Box::new(
        ScrollView::new(GridContentView { grid: view.clone() })
            .content_bottom_inset(view.content_bottom_inset)
            .scrollbar_visibility(ScrollbarVisibility::Automatic),
    )
}

impl<T: Clone + 'static> View for GridView<T> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(
            self.clone(),
            build_grid_child::<T>,
        ))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        alloc::vec![&self.items, &self.selected]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::GridView;
    use crate::color::Color;
    use crate::event::{Event, MouseButton, MouseEvent};
    use crate::pipeline::RenderingPipeline;
    use crate::state::{State, StateId};
    use crate::view::{View, ViewExt};

    #[test]
    fn loading_items_after_mount_exposes_every_grid_row() {
        use super::GridContentElement;
        fn content(element: &dyn crate::element::Element) -> Option<&GridContentElement<usize>> {
            element.as_any().downcast_ref().or_else(|| {
                element
                    .children()
                    .iter()
                    .find_map(|child| content(child.as_ref()))
            })
        }
        let items = State::new(crate::state::generate_state_id(), Vec::<usize>::new());
        let grid = GridView::new(
            items.clone(),
            State::new(crate::state::generate_state_id(), None),
            4,
            100.,
            |_, item, _| {
                crate::views::Rectangle::new()
                    .fill(Color::rgb(item as u8, 20, 50))
                    .frame(f32::INFINITY, 100.)
            },
        )
        .row_spacing(10.)
        .frame(400., 200.);
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(
            crate::views::Window::new("Albums", crate::views::RepaintBoundary::new(grid))
                .decorated(false)
                .size(crate::geometry::Size::new(400., 200.))
                .create_element(),
        );
        pipeline.layout_initial();
        pipeline.render_with_damage();
        items.set((0..100).collect());
        for _ in 0..4 {
            pipeline.render_with_damage();
        }
        let grid = content(pipeline.element_tree().root().unwrap()).unwrap();
        assert_eq!(grid.items.len(), 100);
        assert_eq!(grid.size.height, 2740.);
        pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
            delta_x: 0,
            delta_y: -100_000,
            x: 20,
            y: 20,
            phase: crate::event::WheelPhase::Moved,
            source: crate::event::ScrollSource::Trackpad,
        }));
        let mut last_pixel = None;
        for _ in 0..4 {
            if let Some((frame, _)) = pipeline.render_with_damage() {
                last_pixel = frame.get_pixel(20, 120);
            }
        }
        assert_eq!(last_pixel, Some(Color::rgb(96, 20, 50).to_bgra()));
        let grid = content(pipeline.element_tree().root().unwrap()).unwrap();
        assert!(
            grid.child_indices.contains(&24),
            "Final album row is unreachable: {:?}",
            grid.child_indices
        );
        pipeline.request_redraw();
        let expected = pipeline.render_with_damage().unwrap().0.get_pixel(20, 120);
        assert_eq!(expected, Some(Color::rgb(96, 20, 50).to_bgra()));
    }

    #[test]
    fn floating_clearance_exposes_the_last_grid_row_and_clamps_after_shrink() {
        use super::GridContentView;
        use crate::views::scroll::ScrollViewRenderObject;
        fn offset(element: &dyn crate::element::Element) -> Option<f32> {
            element
                .render_object()
                .and_then(|render| {
                    render
                        .as_any()
                        .downcast_ref::<ScrollViewRenderObject<GridContentView<usize>>>()
                })
                .map(|render| render.offset().1)
                .or_else(|| {
                    element
                        .children()
                        .iter()
                        .find_map(|child| offset(child.as_ref()))
                })
        }
        let items = State::new(
            crate::state::generate_state_id(),
            (0usize..100).collect::<Vec<_>>(),
        );
        let grid = GridView::new(
            items.clone(),
            State::new(crate::state::generate_state_id(), None),
            3,
            40.,
            |_, item, _| {
                crate::views::Rectangle::new()
                    .fill(Color::rgb(item as u8, 20, 50))
                    .frame(f32::INFINITY, 40.)
            },
        )
        .row_spacing(4.)
        .content_bottom_inset(60.)
        .frame(240., 180.);
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(
            crate::views::Window::new("Inset grid", grid)
                .decorated(false)
                .size(crate::geometry::Size::new(240., 180.))
                .create_element(),
        );
        pipeline.layout_initial();
        pipeline.render_with_damage();
        pipeline.handle_event(&Event::Mouse(MouseEvent::Wheel {
            delta_x: 0,
            delta_y: -100_000,
            x: 20,
            y: 20,
            phase: crate::event::WheelPhase::Moved,
            source: crate::event::ScrollSource::Trackpad,
        }));
        let pixel = pipeline.render_with_damage().unwrap().0.get_pixel(20, 100);
        assert_eq!(pixel, Some(Color::rgb(99, 20, 50).to_bgra()));
        assert_eq!(offset(pipeline.element_tree().root().unwrap()), Some(1372.));
        items.set((0..4).collect());
        for _ in 0..4 {
            pipeline.render_with_damage();
        }
        assert_eq!(offset(pipeline.element_tree().root().unwrap()), Some(0.));
        pipeline.teardown();
    }

    #[test]
    fn paints_all_columns_on_initial_layout() {
        let items = State::new(StateId::new(20_001), vec![0usize, 1, 2, 3, 4]);
        let selected = State::new(StateId::new(20_002), None);
        let colors = [
            Color::rgb(220, 40, 40),
            Color::rgb(40, 180, 80),
            Color::rgb(40, 80, 220),
            Color::rgb(220, 180, 40),
            Color::rgb(180, 40, 220),
        ];
        let grid = GridView::new(items, selected, 5, 120.0, move |index, _, _| {
            crate::views::Rectangle::new()
                .fill(colors[index])
                .frame(f32::INFINITY, 100.0)
        })
        .spacing(10.0);

        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(grid.create_element());
        pipeline.layout_initial();

        let pixel = pipeline
            .render_with_damage()
            .and_then(|(buffer, _)| buffer.get_pixel(170, 50));

        assert_eq!(pixel, Some(colors[1].to_bgra()));
    }

    #[test]
    fn selected_state_rebuilds_visible_cells_inside_scroll_view() {
        let items = State::new(crate::state::generate_state_id(), vec![0usize]);
        let selected = State::new(crate::state::generate_state_id(), None);
        let off = Color::rgb(180, 40, 40);
        let on = Color::rgb(40, 180, 80);
        let grid = GridView::new(items, selected.clone(), 1, 80.0, move |_, _, selected| {
            crate::views::Rectangle::new()
                .fill(if selected == Some(0) { on } else { off })
                .frame(f32::INFINITY, 80.0)
        });

        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(grid.create_element());
        pipeline.layout_initial();
        let before = pipeline
            .render_with_damage()
            .and_then(|(buffer, _)| buffer.get_pixel(20, 20));
        assert_eq!(before, Some(off.to_bgra()));

        selected.set(Some(0));
        let after = pipeline
            .render_with_damage()
            .and_then(|(buffer, _)| buffer.get_pixel(20, 20));
        assert_eq!(after, Some(on.to_bgra()));
    }

    #[test]
    fn wraps_items_into_multiple_rows() {
        let items = State::new(StateId::new(20_015), (0usize..10).collect::<Vec<_>>());
        let selected = State::new(StateId::new(20_016), None);
        let colors = [
            Color::rgb(220, 40, 40),
            Color::rgb(40, 180, 80),
            Color::rgb(40, 80, 220),
            Color::rgb(220, 180, 40),
            Color::rgb(180, 40, 220),
            Color::rgb(220, 100, 40),
            Color::rgb(40, 180, 180),
            Color::rgb(100, 40, 220),
            Color::rgb(180, 180, 40),
            Color::rgb(40, 120, 180),
        ];
        let grid = GridView::new(items, selected, 5, 120.0, move |index, _, _| {
            crate::views::Rectangle::new()
                .fill(colors[index])
                .frame(f32::INFINITY, 100.0)
        })
        .spacing(10.0);

        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(grid.create_element());
        pipeline.layout_initial();
        let buffer = pipeline
            .render_with_damage()
            .map(|(buffer, _)| buffer)
            .expect("multi-row grid should render");

        for (index, color) in colors.into_iter().enumerate() {
            let row = (index / 5) as u32;
            let column = (index % 5) as u32;
            let cell_width = (800u32 - 4 * 10) / 5;
            let left = column * (cell_width + 10);
            let right = left + cell_width;
            let top = row * 120u32;
            let bottom = top + 100;
            assert!(
                (left..right).any(|x| {
                    (top..bottom).any(|y| buffer.get_pixel(x, y) == Some(color.to_bgra()))
                }),
                "grid item {index} should be painted in row {row}, column {column}"
            );
        }
    }

    #[test]
    fn row_spacing_separates_neighboring_rows() {
        let items = State::new(StateId::new(20_021), vec![0usize, 1]);
        let selected = State::new(StateId::new(20_022), None);
        let colors = [Color::rgb(220, 40, 40), Color::rgb(40, 80, 220)];
        let grid = GridView::new(items, selected, 1, 40.0, move |index, _, _| {
            crate::views::Rectangle::new()
                .fill(colors[index])
                .frame(f32::INFINITY, 40.0)
        })
        .row_spacing(10.0);

        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(grid.create_element());
        pipeline.layout_initial();
        let buffer = pipeline
            .render_with_damage()
            .map(|(buffer, _)| buffer)
            .expect("spaced grid should render");

        assert_eq!(buffer.get_pixel(20, 20), Some(colors[0].to_bgra()));
        assert_ne!(buffer.get_pixel(20, 45), Some(colors[0].to_bgra()));
        assert_ne!(buffer.get_pixel(20, 45), Some(colors[1].to_bgra()));
        assert_eq!(buffer.get_pixel(20, 55), Some(colors[1].to_bgra()));
    }

    #[test]
    fn wraps_items_into_multiple_rows_after_state_update() {
        let items = State::new(StateId::new(20_017), Vec::<usize>::new());
        let selected = State::new(StateId::new(20_018), None);
        let colors = [
            Color::rgb(220, 40, 40),
            Color::rgb(40, 180, 80),
            Color::rgb(40, 80, 220),
            Color::rgb(220, 180, 40),
            Color::rgb(180, 40, 220),
            Color::rgb(220, 100, 40),
            Color::rgb(40, 180, 180),
            Color::rgb(100, 40, 220),
            Color::rgb(180, 180, 40),
            Color::rgb(40, 120, 180),
        ];
        let grid = GridView::new(items.clone(), selected, 5, 120.0, move |index, _, _| {
            crate::views::Rectangle::new()
                .fill(colors[index])
                .frame(f32::INFINITY, 100.0)
        })
        .spacing(10.0);

        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(grid.create_element());
        pipeline.layout_initial();
        pipeline.render_with_damage();

        items.set((0usize..10).collect::<Vec<_>>());
        let buffer = pipeline
            .render_with_damage()
            .map(|(buffer, _)| buffer)
            .expect("updated multi-row grid should render");

        let cell_width = (buffer.width() - 4 * 10) / 5;
        for (index, color) in colors.into_iter().enumerate() {
            let row = (index / 5) as u32;
            let column = (index % 5) as u32;
            let left = column * (cell_width + 10);
            let right = left + cell_width;
            let top = row * 120;
            let bottom = top + 100;
            assert!(
                (left..right).any(|x| {
                    (top..bottom).any(|y| buffer.get_pixel(x, y) == Some(color.to_bgra()))
                }),
                "updated grid item {index} should be painted in row {row}, column {column}"
            );
        }
    }

    #[test]
    fn preserves_minimum_cell_width_and_wraps_on_resize() {
        let items = State::new(StateId::new(20_019), (0usize..6).collect::<Vec<_>>());
        let selected = State::new(StateId::new(20_020), None);
        let colors = [
            Color::rgb(220, 40, 40),
            Color::rgb(40, 180, 80),
            Color::rgb(40, 80, 220),
            Color::rgb(220, 180, 40),
            Color::rgb(180, 40, 220),
            Color::rgb(220, 100, 40),
        ];
        let grid = GridView::new(items, selected, 5, 120.0, move |index, _, _| {
            crate::views::Rectangle::new()
                .fill(colors[index])
                .frame(f32::INFINITY, 100.0)
        })
        .spacing(10.0)
        .minimum_cell_width(150.0);

        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(grid.create_element());
        pipeline.layout_initial();
        pipeline.render_with_damage();

        pipeline.resize(crate::geometry::Size::new(500.0, 400.0));
        let buffer = pipeline
            .render_with_damage()
            .map(|(buffer, _)| buffer)
            .expect("resized grid should render");

        for (index, color) in colors.into_iter().enumerate() {
            let row = (index / 3) as u32;
            let column = (index % 3) as u32;
            let left = column * 160;
            let right = left + 150;
            let top = row * 120;
            let bottom = top + 100;
            assert!(
                (left..right).any(|x| {
                    (top..bottom).any(|y| buffer.get_pixel(x, y) == Some(color.to_bgra()))
                }),
                "resized grid item {index} should remain at least 150px wide and wrap to row {row}"
            );
        }
    }

    #[test]
    fn wrapped_grid_click_targets_visual_cell_after_resize() {
        let items = State::new(
            crate::state::generate_state_id(),
            (0usize..8).collect::<Vec<_>>(),
        );
        let selected = State::new(crate::state::generate_state_id(), None);
        let activated = State::new(crate::state::generate_state_id(), None);
        let activated_for_grid = activated.clone();
        let grid = GridView::new(items, selected, 5, 120.0, move |index, _, _| {
            let activated = activated_for_grid.clone();
            crate::views::Rectangle::new()
                .fill(Color::rgb(80, 120, 180))
                .frame(f32::INFINITY, 120.0)
                .on_click(move || activated.set(Some(index)))
        })
        .spacing(10.0)
        .minimum_cell_width(150.0);

        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(grid.create_element());
        pipeline.layout_initial();
        pipeline.render_with_damage();

        // The initial 800px viewport fits five cells. At 640px the same grid
        // wraps to four columns, matching the Files window configuration.
        pipeline.resize(crate::geometry::Size::new(640.0, 400.0));
        pipeline.render_with_damage();

        assert!(
            pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 75,
                y: 60,
                click_count: 1,
            }))
        );
        assert_eq!(activated.get(), Some(0));

        activated.set(None);
        assert!(
            pipeline.handle_event(&Event::Mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                x: 75,
                y: 180,
                click_count: 1,
            }))
        );
        assert_eq!(activated.get(), Some(4));
    }
}

#[cfg(test)]
mod performance_tests {
    use super::*;
    use crate::testing::layout_probe::LayoutProbe;
    use core::cell::Cell;
    #[test]
    fn scrolling_lays_out_only_entering_grid_cells_and_resize_reflows_all_cells() {
        let calls = Rc::new(Cell::new(0));
        let layouts = calls.clone();
        let items = State::new(
            crate::state::generate_state_id(),
            (0..50_000).collect::<Vec<usize>>(),
        );
        let grid = GridView::new(
            items,
            State::new(crate::state::generate_state_id(), None),
            4,
            100.,
            move |_, _, _| LayoutProbe(layouts.clone()),
        )
        .spacing(0.)
        .minimum_cell_width(100.);
        let mut element = GridContentElement::new(grid);
        element.set_viewport_hint(Rect::from_xywh(0., 1000., 400., 200.));
        element.layout(LayoutConstraints::tight(400., 200.));
        let previous = element.child_indices.clone();
        calls.set(0);
        element.set_viewport_hint(Rect::from_xywh(0., 1100., 400., 200.));
        let entering = element
            .child_indices
            .iter()
            .filter(|index| !previous.contains(index))
            .count();
        assert_eq!(
            calls.get(),
            entering * element.current_columns,
            "Retained grid cells were unnecessarily measured"
        );
        calls.set(0);
        element.layout(LayoutConstraints::tight(200., 200.));
        assert_eq!(element.current_columns, 2);
        assert!(calls.get() >= element.children.len() * 2);
    }
}
