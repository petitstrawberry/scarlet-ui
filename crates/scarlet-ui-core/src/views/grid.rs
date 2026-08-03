//! State-backed grid primitives.
//!
//! `GridView` provides the layout and virtualization needed by icon-oriented
//! applications while leaving the item model and cell appearance to callers.
//! It is suitable for file managers, launchers, galleries, and media views.

use crate::element::{
    ComponentElement, Element, ElementRenderObject, LayoutConstraints, RenderElement,
};
use crate::geometry::{Point, Size};
use crate::state::{Listenable, State};
use crate::view::View;
use crate::views::{LazyVStack, ScrollView, ScrollbarVisibility, Spacer};
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::Any;

type CellBuilder<T> = Rc<dyn Fn(usize, T, Option<usize>) -> Box<dyn View>>;

/// A state-backed, vertically scrollable grid.
///
/// The grid creates fixed-size rows and delegates each cell's content to the
/// caller. The selected index is optional so the same component can be used as
/// a selectable file view or as a plain icon gallery.
#[derive(Clone)]
pub struct GridView<T: Clone + 'static> {
    items: State<Vec<T>>,
    selected: State<Option<usize>>,
    columns: usize,
    row_height: f32,
    spacing: f32,
    cell_builder: CellBuilder<T>,
}

impl<T: Clone + 'static> GridView<T> {
    /// Create a grid view.
    ///
    /// # Arguments
    ///
    /// * `items` - Reactive collection displayed by the grid.
    /// * `selected` - Reactive optional selected item index.
    /// * `columns` - Number of cells in each row.
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
            cell_builder: Rc::new(move |index, item, selected| {
                Box::new(cell_builder(index, item, selected))
            }),
        }
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
        self.view.create_element()
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        self.view.listenables()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
struct GridRow {
    cells: Vec<AnyView>,
    row_height: f32,
    spacing: f32,
}

impl GridRow {
    fn new(cells: Vec<AnyView>, row_height: f32, spacing: f32) -> Self {
        Self {
            cells,
            row_height,
            spacing,
        }
    }
}

impl View for GridRow {
    fn create_element(&self) -> Box<dyn Element> {
        let children = self
            .cells
            .iter()
            .map(|cell| cell.create_element())
            .collect();
        Box::new(RenderElement::with_children(
            self.clone(),
            GridRowRenderObject::new(self.cells.len(), self.row_height, self.spacing),
            children,
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
    size: Size,
}

impl GridRowRenderObject {
    fn new(cell_count: usize, row_height: f32, spacing: f32) -> Self {
        Self {
            cell_count,
            row_height,
            spacing,
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
        let cell_width = ((size.width - total_spacing).max(0.0) / count as f32).max(0.0);

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

fn build_grid_child<T: Clone + 'static>(view: &GridView<T>) -> Box<dyn Element> {
    let items = view.items.clone();
    let selected = view.selected.clone();
    let cell_builder = view.cell_builder.clone();
    let columns = view.columns;
    let row_height = view.row_height;
    let spacing = view.spacing;
    let item_count = items.get().len();
    let row_count = item_count.div_ceil(columns);

    let rows = LazyVStack::new(row_count, row_height, move |row_index| {
        let mut cells = Vec::with_capacity(columns);
        for column in 0..columns {
            let index = row_index * columns + column;
            if let Some(item) = items.get().get(index).cloned() {
                cells.push(AnyView::new((cell_builder)(index, item, selected.get())));
            } else {
                cells.push(AnyView::new(Box::new(Spacer::new())));
            }
        }
        GridRow::new(cells, row_height, spacing)
    });

    ScrollView::new(rows)
        .scrollbar_visibility(ScrollbarVisibility::Automatic)
        .create_element()
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
