//! State-backed scrollable list primitives.
//!
//! `ListView` deliberately knows nothing about files or application models. A
//! caller supplies the item state and a row builder, so the same component can
//! power a file manager, settings list, contact list, or search results.

use crate::element::{ComponentElement, Element};
use crate::state::{Listenable, State};
use crate::view::View;
use crate::views::{LazyVStack, ScrollView, ScrollbarVisibility};
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::Any;

type RowBuilder<T> = Rc<dyn Fn(usize, T, Option<usize>) -> Box<dyn View>>;

/// A state-backed, vertically scrollable list with caller-defined rows.
///
/// Rows are virtualized through [`LazyVStack`]. The selected index is optional
/// so callers can use the component as either a selectable list or a plain
/// collection. The row builder receives the item index, a cloned item, and the
/// current selected index.
#[derive(Clone)]
pub struct ListView<T: Clone + 'static> {
    items: State<Vec<T>>,
    selected: State<Option<usize>>,
    row_height: f32,
    row_builder: RowBuilder<T>,
}

impl<T: Clone + 'static> ListView<T> {
    /// Create a list view.
    ///
    /// # Arguments
    ///
    /// * `items` - Reactive collection displayed by the list.
    /// * `selected` - Reactive optional selected row index.
    /// * `row_height` - Fixed logical height allocated to each row.
    /// * `row_builder` - Closure producing a row view.
    ///
    /// # Returns
    ///
    /// A virtualized list view.
    pub fn new<V>(
        items: State<Vec<T>>,
        selected: State<Option<usize>>,
        row_height: f32,
        row_builder: impl Fn(usize, T, Option<usize>) -> V + 'static,
    ) -> Self
    where
        V: View + 'static,
    {
        Self {
            items,
            selected,
            row_height: row_height.max(1.0),
            row_builder: Rc::new(move |index, item, selected| {
                Box::new(row_builder(index, item, selected))
            }),
        }
    }

    /// Return the item state used by this list.
    pub fn items(&self) -> &State<Vec<T>> {
        &self.items
    }

    /// Return the optional selected-row state.
    pub fn selected(&self) -> &State<Option<usize>> {
        &self.selected
    }

    /// Return the fixed row height.
    pub fn row_height(&self) -> f32 {
        self.row_height
    }
}

/// Type-erased cloneable view used for dynamically built list rows.
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

fn build_list_child<T: Clone + 'static>(view: &ListView<T>) -> Box<dyn Element> {
    let items = view.items.clone();
    let selected = view.selected.clone();
    let row_builder = view.row_builder.clone();
    let row_height = view.row_height;
    let item_count = items.get().len();

    let rows = LazyVStack::new(item_count, row_height, move |index| {
        let item = items
            .get()
            .get(index)
            .cloned()
            .expect("ListView row index must be within item count");
        AnyView::new((row_builder)(index, item, selected.get()))
    });
    ScrollView::new(rows)
        .scrollbar_visibility(ScrollbarVisibility::Automatic)
        .create_element()
}

impl<T: Clone + 'static> View for ListView<T> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(
            self.clone(),
            build_list_child::<T>,
        ))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        alloc::vec![&self.items, &self.selected]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
