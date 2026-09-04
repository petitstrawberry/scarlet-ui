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
struct ListContentView<T: Clone + 'static> {
    items: State<Vec<T>>,
    selected: State<Option<usize>>,
    row_height: f32,
    row_builder: RowBuilder<T>,
}

fn build_list_content<T: Clone + 'static>(view: &ListContentView<T>) -> Box<dyn View> {
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
    Box::new(rows)
}

impl<T: Clone + 'static> View for ListContentView<T> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(
            self.clone(),
            build_list_content::<T>,
        ))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        alloc::vec![&self.items, &self.selected]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn build_list_child<T: Clone + 'static>(view: &ListView<T>) -> Box<dyn View> {
    let row_height = view.row_height;
    let content = ListContentView {
        items: view.items.clone(),
        selected: view.selected.clone(),
        row_height,
        row_builder: view.row_builder.clone(),
    };
    Box::new(
        ScrollView::new(content)
            .scroll_to_index_state(view.selected.clone(), row_height)
            .scrollbar_visibility(ScrollbarVisibility::Automatic),
    )
}

impl<T: Clone + 'static> View for ListView<T> {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(
            self.clone(),
            build_list_child::<T>,
        ))
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        // The nested content view and ScrollView subscribe to these states.
        // Keeping the outer component unsubscribed preserves the scroll
        // render object while rows and selection are rebuilt underneath it.
        Vec::new()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::element::{Element, ElementId};
    use crate::event::{Event, MouseEvent, Phase, ScrollSource, WheelPhase};
    use crate::geometry::Size;
    use crate::pipeline::RenderingPipeline;
    use crate::state::{State, StateId};
    use crate::view::ViewExt;
    use crate::views::{TabItem, TabView, Text, Window};

    fn find_scroll_id(element: &dyn Element) -> Option<ElementId> {
        if element.type_name_debug().contains("ScrollView<") {
            return Some(element.id());
        }
        element
            .children()
            .iter()
            .find_map(|child| find_scroll_id(child.as_ref()))
    }

    #[test]
    fn selected_state_repaints_visible_row_inside_scroll_view() {
        let items = State::new(crate::state::generate_state_id(), vec![0usize]);
        let selected = State::new(crate::state::generate_state_id(), None);
        let off = Color::rgb(180, 40, 40);
        let on = Color::rgb(40, 180, 80);
        let list = ListView::new(items, selected.clone(), 80.0, move |_, _, selected| {
            crate::views::Rectangle::new()
                .fill(if selected == Some(0) { on } else { off })
                .frame(f32::INFINITY, 80.0)
        });

        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(list.create_element());
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
    fn state_rebuild_preserves_nested_scroll_element_and_offset() {
        let items = State::new(StateId::new(31_001), (0usize..100).collect::<Vec<_>>());
        let selected = State::new(StateId::new(31_002), None);
        let selected_tab = State::new(StateId::new(31_003), 0usize);
        let tab_items = items.clone();
        let tab_selected = selected.clone();
        let tabs = TabView::with_selected_index(
            alloc::vec![TabItem::new("Processes", move || {
                ListView::new(
                    tab_items.clone(),
                    tab_selected.clone(),
                    24.0,
                    |_, item, _| Text::new(format!("Process {item}")),
                )
            })],
            selected_tab,
        );
        let window = Window::new("Task Manager", tabs).size(Size::new(320.0, 220.0));

        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(window.create_element());
        pipeline.layout_initial();

        let scroll_id = find_scroll_id(
            pipeline
                .element_tree()
                .root()
                .expect("window should have an element root"),
        )
        .expect("list should contain a ScrollView element");
        let wheel = Event::Mouse(MouseEvent::Wheel {
            delta_x: 0,
            delta_y: -400,
            x: 10,
            y: 10,
            phase: WheelPhase::Moved,
            source: ScrollSource::Trackpad,
        });
        assert!(
            pipeline
                .element_tree_mut()
                .find_element_mut(scroll_id)
                .expect("scroll element should exist")
                .handle_event(&wheel, Phase::Target)
        );
        let offset_before = -pipeline
            .element_tree()
            .find_element(scroll_id)
            .expect("scroll element should exist")
            .children()[0]
            .position()
            .y;
        assert!(offset_before > 0.0);

        items.set((100usize..200).collect());
        let _ = pipeline.render();

        let root = pipeline
            .element_tree()
            .root()
            .expect("window should remain mounted");
        assert_eq!(find_scroll_id(root), Some(scroll_id));
        let offset_after = -pipeline
            .element_tree()
            .find_element(scroll_id)
            .expect("same scroll element should remain mounted")
            .children()[0]
            .position()
            .y;
        assert!((offset_after - offset_before).abs() < 0.01);
    }
}
