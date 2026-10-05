//! Counts leaf layout work for virtualization regressions.
use crate::{
    element::{Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult},
    geometry::{Point, Size},
    view::View,
};
use alloc::{boxed::Box, rc::Rc};
use core::{any::Any, cell::Cell};
#[derive(Clone)]
pub(crate) struct LayoutProbe(pub Rc<Cell<usize>>);
impl View for LayoutProbe {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::new(
            self.clone(),
            ProbeObject {
                calls: self.0.clone(),
                size: Size::ZERO,
            },
        ))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
struct ProbeObject {
    calls: Rc<Cell<usize>>,
    size: Size,
}
impl ElementRenderObject for ProbeObject {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.calls.set(self.calls.get() + 1);
        self.size = constraints.constrain(Size::new(10., 10.));
        self.size
    }
    fn size(&self) -> Size {
        self.size
    }
    fn hit_test(&self, _: Point) -> bool {
        false
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn render(&mut self) {}
    fn update(&mut self, _: &dyn View) -> UpdateResult {
        UpdateResult::NoChange
    }
}
