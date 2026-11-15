//! Invalidation regressions on the production frame path, without a GPU.
use super::*;
use crate::element::{
    ComponentElement, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult,
};
use crate::event::{Event, MouseButton, MouseEvent};
use crate::prelude::*;
use crate::state::{State, generate_state_id};
use core::any::Any;

struct Commands;
impl PaintBackend for Commands {
    fn supports_retained_display_lists(&self) -> bool {
        true
    }
    fn resize(&mut self, _: Size, _: u32) {}
    fn render<'a>(
        &'a mut self,
        _: &PaintContext<'_>,
        _: Color,
        _: Option<&[Rect]>,
        _: Option<&[crate::compositor::DamageRect]>,
    ) -> crate::error::Result<BackendFrame<'a>> {
        Ok(BackendFrame::External)
    }
}
fn pipeline(view: impl View + Clone + 'static) -> RenderingPipeline {
    let mut p = RenderingPipeline::new();
    p.set_root(
        Window::new("Metadata", view)
            .decorated(false)
            .shadow(false)
            .size(Size::new(240., 160.))
            .create_element(),
    );
    p.set_paint_backend(Box::new(Commands));
    p.layout_initial();
    p.render_for_present().unwrap();
    p
}
fn find_id(element: &dyn Element, matches: &dyn Fn(&dyn Element) -> bool) -> Option<ElementId> {
    if matches(element) {
        return Some(element.id());
    }
    element
        .children()
        .iter()
        .find_map(|child| find_id(child.as_ref(), matches))
}
#[test]
fn slider_motion_and_noop_rebuild_do_not_rescan_focus_popups_or_sibling_metadata() {
    let value = State::new(generate_state_id(), 0.5);
    let tile = Rectangle::new().frame(200., 2.);
    let row = VStack::new((
        tile.clone(),
        tile.clone(),
        tile.clone(),
        tile.clone(),
        tile.clone(),
        tile.clone(),
        tile.clone(),
        tile,
    ));
    let mut p = pipeline(VStack::new((
        Slider::new(value.clone()).frame(200., 20.),
        VStack::new((row.clone(), row.clone(), row.clone(), row.clone(), row)),
    )));
    let slider = find_id(p.element_tree.root().unwrap(), &|e| {
        e.render_object()
            .is_some_and(|r| r.as_any().is::<crate::views::SliderRenderObject>())
    })
    .unwrap();
    let path = p.element_tree.find_path_ids(slider).unwrap();
    let (_, origin) = p
        .element_tree
        .element_and_absolute_origin_for_path(&path)
        .unwrap();
    let mouse = |x| {
        Event::Mouse(MouseEvent::Moved {
            x: origin.x as i32 + x,
            y: origin.y as i32 + 10,
        })
    };
    p.handle_event(&Event::Mouse(MouseEvent::ButtonPressed {
        button: MouseButton::Left,
        x: origin.x as i32 + 60,
        y: origin.y as i32 + 10,
        click_count: 1,
    }));
    p.render_for_present().unwrap();
    // Reconcile the newly acquired focus once, then every motion is pure paint.
    for x in [70, 90, 110, 130] {
        p.reset_paint_test_counters();
        p.handle_event(&mouse(x));
        p.render_for_present().unwrap();
        assert_eq!(p.paint_test_counters.focus_reconciliations, 0);
        assert_eq!(p.paint_test_counters.popup_viewport_syncs, 0);
        assert!(
            p.scene_metadata.visits <= 2,
            "paint must query only the slider, not its 40 siblings: {}",
            p.scene_metadata.visits
        );
    }
    p.reset_paint_test_counters();
    value.set(value.get());
    assert!(matches!(
        p.render_for_present().unwrap(),
        PresentedFrame::Idle
    ));
    assert_eq!(p.paint_test_counters.focus_reconciliations, 0);
    assert_eq!(p.paint_test_counters.popup_viewport_syncs, 0);
    assert_eq!(p.scene_metadata.visits, 1);
    // The dense-paint fallback may refresh every cached fact, but that does
    // not make unchanged topology/focus/popup state dirty.
    fn collect(element: &dyn Element, ids: &mut Vec<ElementId>) {
        ids.push(element.id());
        for child in element.children() {
            collect(child.as_ref(), ids);
        }
    }
    let mut ids = Vec::new();
    collect(p.element_tree.root().unwrap(), &mut ids);
    assert!(ids.len() > 64);
    for id in ids {
        p.pipeline_owner.mark_needs_paint(id);
    }
    p.reset_paint_test_counters();
    p.render_for_present().unwrap();
    assert_eq!(p.paint_test_counters.focus_reconciliations, 0);
    assert_eq!(p.paint_test_counters.popup_viewport_syncs, 0);
    p.teardown();
}

#[derive(Clone)]
struct Material(State<[bool; 3]>);
impl View for Material {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::new(
            self.clone(),
            MaterialObject(self.0.clone()),
        ))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
struct MaterialObject(State<[bool; 3]>);
impl ElementRenderObject for MaterialObject {
    fn layout(&mut self, _: LayoutConstraints) -> Size {
        Size::new(40., 40.)
    }
    fn size(&self) -> Size {
        Size::new(40., 40.)
    }
    fn render(&mut self) {}
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn reads_backdrop(&self) -> bool {
        self.0.get()[0]
    }
    fn emits_paint_extension(&self) -> bool {
        self.0.get()[1]
    }
    fn has_opacity_scope(&self) -> bool {
        self.0.get()[2]
    }
    fn subtree_opacity(&self) -> f32 {
        if self.0.get()[2] { 0.5 } else { 1. }
    }
    fn update(&mut self, _: &dyn View) -> UpdateResult {
        UpdateResult::NoChange
    }
    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        let rect = Rect::new(origin, self.size());
        ctx.fill_rect(rect, Color::RED);
        if self.reads_backdrop() {
            ctx.backdrop_filter_rounded_rect(rect, 4., 2., Color::CLEAR);
        }
        if self.emits_paint_extension() {
            ctx.draw_extension(rect, alloc::sync::Arc::new(1u32));
        }
        true
    }
}
#[test]
fn paint_only_dependency_changes_keep_other_readers_and_remove_the_last_reader() {
    let a = State::new(generate_state_id(), [false; 3]);
    let b = State::new(generate_state_id(), [false; 3]);
    let mut p = pipeline(HStack::new((Material(a.clone()), Material(b.clone()))));
    let a_id = find_id(p.element_tree.root().unwrap(), &|e| {
        e.render_object()
            .and_then(|r| r.as_any().downcast_ref::<MaterialObject>())
            .is_some_and(|r| r.0.id() == a.id())
    })
    .unwrap();
    let b_id = find_id(p.element_tree.root().unwrap(), &|e| {
        e.render_object()
            .and_then(|r| r.as_any().downcast_ref::<MaterialObject>())
            .is_some_and(|r| r.0.id() == b.id())
    })
    .unwrap();
    for flags in [
        [true, false, false],
        [false, true, false],
        [false, false, true],
        [true; 3],
        [false; 3],
    ] {
        a.set(flags);
        p.pipeline_owner.mark_needs_self_paint(a_id);
        p.render_for_present().unwrap();
        assert_eq!(
            (
                p.scene_reads_backdrop,
                p.scene_has_extensions,
                p.scene_has_opacity
            ),
            (flags[0], flags[1], flags[2])
        );
    }
    a.set([true; 3]);
    b.set([true; 3]);
    p.pipeline_owner.mark_needs_self_paint(a_id);
    p.pipeline_owner.mark_needs_self_paint(b_id);
    p.render_for_present().unwrap();
    a.set([false; 3]);
    p.pipeline_owner.mark_needs_self_paint(a_id);
    p.render_for_present().unwrap();
    assert_eq!(
        p.scene_metadata.dependencies(),
        (true, true, true),
        "the unchanged sibling still contributes all three flags"
    );
    b.set([true, true, false]);
    // Opacity/presentation changes may notify composition without a paint ID.
    p.pipeline_owner.mark_needs_composite(b_id);
    p.render_for_present().unwrap();
    assert_eq!(p.scene_metadata.dependencies(), (true, true, false));
    b.set([false; 3]);
    p.pipeline_owner.mark_needs_self_paint(b_id);
    p.render_for_present().unwrap();
    assert_eq!(p.scene_metadata.dependencies(), (false, false, false));
    p.teardown();
}

#[derive(Clone)]
struct Page {
    show: State<bool>,
    flags: State<[bool; 3]>,
    text: State<String>,
}
impl View for Page {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(self.clone(), |v| {
            if v.show.get() {
                Box::new(VStack::new((
                    Material(v.flags.clone()),
                    TextField::new(v.text.clone()).autofocus(true),
                )))
            } else {
                Box::new(Rectangle::new())
            }
        }))
    }
    fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
        vec![&self.show]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
#[test]
fn structural_rebuild_adopts_autofocus_and_removes_retired_dependency_flags() {
    let show = State::new(generate_state_id(), false);
    let mut p = pipeline(Page {
        show: show.clone(),
        flags: State::new(generate_state_id(), [true; 3]),
        text: State::new(generate_state_id(), String::from("focus")),
    });
    assert!(p.focused_text_input_state().is_none());
    show.set(true);
    p.render_for_present().unwrap();
    assert!(p.focused_text_input_state().is_some());
    assert_eq!(p.scene_metadata.dependencies(), (true, true, true));
    show.set(false);
    p.render_for_present().unwrap();
    assert!(p.focused_text_input_state().is_none());
    assert_eq!(p.scene_metadata.dependencies(), (false, false, false));
    p.teardown();
}
