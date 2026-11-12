use super::*;
use crate::generate_state_id;
use crate::{
    element::{ComponentElement, Element},
    pipeline::RenderingPipeline,
    prelude::*,
    views::{
        Spacer,
        modifiers::{
            BackgroundRenderObject, FrameRenderObject, OffsetRenderObject, OpacityRenderObject,
        },
    },
};
use alloc::{boxed::Box, rc::Rc, vec, vec::Vec};
use core::{any::Any, cell::Cell, time::Duration};

#[derive(Clone)]
struct Demo {
    expanded: State<bool>,
    implicit: bool,
    builds: Rc<Cell<usize>>,
}
impl Demo {
    fn body(&self) -> Box<dyn View> {
        self.builds.set(self.builds.get() + 1);
        let expanded = self.expanded.get();
        let content = Spacer::new()
            .frame(if expanded { 120.0 } else { 40.0 }, 40.0)
            .background(if expanded { Color::WHITE } else { Color::BLACK })
            .offset(if expanded { 60.0 } else { 0.0 }, 0.0)
            .opacity(if expanded { 0.5 } else { 1.0 });
        if self.implicit {
            Box::new(content.animation(Animation::linear(Duration::from_millis(300)), expanded))
        } else {
            Box::new(content)
        }
    }
}
impl View for Demo {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(self.clone(), Self::body))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.expanded]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
fn demo(implicit: bool) -> (Demo, RenderingPipeline) {
    let view = Demo {
        expanded: State::new(generate_state_id(), false),
        implicit,
        builds: Rc::new(Cell::new(0)),
    };
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_root(
        Window::new("Animation test", view.clone())
            .decorated(false)
            .size(Size::new(400.0, 200.0))
            .create_element(),
    );
    pipeline.layout_initial();
    pipeline.render_for_present().unwrap();
    (view, pipeline)
}
fn object<T: Any>(element: &dyn Element) -> Option<&T> {
    element
        .render_object()
        .and_then(|r| r.as_any().downcast_ref::<T>())
        .or_else(|| {
            element
                .children()
                .iter()
                .find_map(|child| object(child.as_ref()))
        })
}
fn get<T: Any>(pipeline: &RenderingPipeline) -> &T {
    object(pipeline.element_tree().root().unwrap()).unwrap()
}
fn width(pipeline: &RenderingPipeline) -> f32 {
    get::<FrameRenderObject>(pipeline).width_value().unwrap()
}
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.001, "{a} != {b}");
}

#[test]
fn explicit_state_change_interpolates_presentation_without_rebuilding_each_frame() {
    let (view, mut pipeline) = demo(false);
    assert!(!pipeline.has_active_animation());
    with_animation(Animation::linear(Duration::from_millis(300)), || {
        view.expanded.set(true)
    });
    assert!(view.expanded.get(), "model state changes synchronously");
    close(width(&pipeline), 40.0);
    pipeline.render_for_present().unwrap();
    assert!(pipeline.has_visual_animation());
    close(width(&pipeline), 40.0);
    let builds = view.builds.get();
    for _ in 0..3 {
        pipeline.advance_visual_animations(Duration::from_millis(50));
        pipeline.render_for_present().unwrap();
    }
    close(width(&pipeline), 80.0);
    close(get::<OffsetRenderObject>(&pipeline).offset().x, 30.0);
    close(get::<OpacityRenderObject>(&pipeline).opacity(), 0.75);
    close(get::<BackgroundRenderObject>(&pipeline).get_color().r, 0.5);
    assert_eq!(view.builds.get(), builds);
    pipeline.advance_visual_animations(Duration::from_millis(150));
    pipeline.render_for_present().unwrap();
    close(width(&pipeline), 120.0);
    assert!(!pipeline.has_active_animation());
    assert_eq!(view.builds.get(), builds);
    assert!(!pipeline.advance_visual_animations(Duration::from_secs(1)));
}

#[test]
fn implicit_animation_retargets_from_current_value_and_can_be_disabled() {
    let (view, mut pipeline) = demo(true);
    view.expanded.set(true);
    pipeline.render_for_present().unwrap();
    assert!(pipeline.has_visual_animation());
    pipeline.advance_visual_animations(Duration::from_millis(150));
    pipeline.render_for_present().unwrap();
    close(width(&pipeline), 80.0);
    view.expanded.set(false);
    pipeline.render_for_present().unwrap();
    close(width(&pipeline), 80.0);
    pipeline.advance_visual_animations(Duration::from_millis(150));
    pipeline.render_for_present().unwrap();
    close(width(&pipeline), 60.0);
    without_animation(|| view.expanded.set(false));
    pipeline.render_for_present().unwrap();
    close(width(&pipeline), 40.0);
    assert!(
        !pipeline.has_active_animation(),
        "an explicit immediate reset settles even the existing target"
    );
    without_animation(|| view.expanded.set(true));
    pipeline.render_for_present().unwrap();
    close(width(&pipeline), 120.0);
    assert!(!pipeline.has_active_animation());
}

#[test]
fn coalesced_writes_use_last_policy_and_shared_state_animates_each_window() {
    let (view, mut first) = demo(false);
    let mut second = RenderingPipeline::new();
    second.set_root(
        Window::new("Second", view.clone())
            .decorated(false)
            .size(Size::new(400., 200.))
            .create_element(),
    );
    second.layout_initial();
    second.render_for_present().unwrap();
    with_animation(Animation::linear(Duration::from_millis(300)), || {
        view.expanded.set(true)
    });
    first.render_for_present().unwrap();
    second.render_for_present().unwrap();
    assert!(first.has_visual_animation() && second.has_visual_animation());
    first.advance_visual_animations(Duration::from_millis(150));
    close(width(&first), 80.0);
    close(width(&second), 40.0);
    first.teardown();
    assert!(!first.has_visual_animation());
    assert!(second.has_visual_animation());
    second.cancel_animations();
    close(width(&second), 120.0);
    assert!(!second.has_visual_animation());
    with_animation(Animation::default(), || view.expanded.set(false));
    view.expanded.set(true);
    view.expanded.set(false);
    second.render_for_present().unwrap();
    close(width(&second), 40.0);
    assert!(
        !second.has_visual_animation(),
        "normal final notification overrides pending animation"
    );
}

struct RetainedPixels(crate::renderer::CpuPaintRenderer);
impl crate::renderer::PaintBackend for RetainedPixels {
    fn resize(&mut self, size: Size, scale: u32) {
        self.0.resize(size, scale);
    }
    fn supports_retained_display_lists(&self) -> bool {
        true
    }
    fn render<'a>(
        &'a mut self,
        ctx: &crate::renderer::PaintContext<'_>,
        background: Color,
        damage: Option<&[crate::Rect]>,
        _: Option<&[crate::compositor::DamageRect]>,
    ) -> crate::Result<crate::renderer::BackendFrame<'a>> {
        self.0.set_background_color(background);
        self.0.execute_with_damage(ctx, damage);
        Ok(crate::renderer::BackendFrame::Cpu {
            buffer: self.0.buffer(),
        })
    }
}

#[test]
fn opacity_multiplies_nested_scopes_and_restores_siblings() {
    use crate::views::{HStack, ScrollView};
    for retained in [false, true] {
        let white = || Spacer::new().frame(20., 20.).background(Color::WHITE);
        let grouped = HStack::new((white().opacity(0.5), white())).spacing(0.);
        let scroller = ScrollView::new(grouped)
            .content_size(40., 20.)
            .frame(40., 20.)
            .opacity(0.5);
        let content = HStack::new((scroller, white())).spacing(0.);
        let mut pipeline = RenderingPipeline::new();
        if retained {
            pipeline.set_paint_backend(Box::new(RetainedPixels(
                crate::renderer::CpuPaintRenderer::new(Size::new(60., 20.), 1000, Color::BLACK),
            )));
        }
        pipeline.set_root(
            Window::new("Opacity", content)
                .decorated(false)
                .background_color(Color::BLACK)
                .size(Size::new(60., 20.))
                .create_element(),
        );
        pipeline.layout_initial();
        let frame = pipeline.render_for_present().unwrap();
        let crate::renderer::PresentedFrame::Cpu { buffer, .. } = frame else {
            panic!("CPU frame")
        };
        let left = buffer.get_pixel(10, 10).unwrap();
        let middle = buffer.get_pixel(30, 10).unwrap();
        let right = buffer.get_pixel(50, 10).unwrap();
        assert!(
            (63..=65).contains(&(left & 255)),
            "retained={retained}: {left:?}"
        );
        assert!(
            (127..=129).contains(&(middle & 255)),
            "retained={retained}: {middle:?}"
        );
        assert_eq!(right, Color::WHITE.to_bgra());
    }
}

#[test]
fn nested_transactions_restore_after_unwind_and_do_not_leak_to_threads() {
    let outer = Animation::default();
    with_animation(outer, || {
        assert_eq!(current_transaction().animation, Some(outer));
        without_animation(|| assert!(current_transaction().disables_animations));
        assert_eq!(current_transaction().animation, Some(outer));
        std::thread::spawn(|| assert_eq!(current_transaction(), Transaction::default()))
            .join()
            .unwrap();
        let _ = std::panic::catch_unwind(|| with_animation(None, || panic!("scope unwind")));
        assert_eq!(current_transaction().animation, Some(outer));
    });
    assert_eq!(current_transaction(), Transaction::default());
}

#[test]
fn retained_offset_motion_reuses_paint_and_layout_and_tracks_hit_testing() {
    use crate::{
        element::{LayoutConstraints, RenderElement},
        geometry::Rect,
        renderer::{BackendFrame, PaintBackend, PaintContext},
    };
    #[derive(Clone)]
    struct Tile {
        layouts: Rc<Cell<usize>>,
        paints: Rc<Cell<usize>>,
    }
    struct TileObject {
        tile: Tile,
        size: Size,
    }
    impl View for Tile {
        fn create_element(&self) -> Box<dyn Element> {
            Box::new(RenderElement::new(
                self.clone(),
                TileObject {
                    tile: self.clone(),
                    size: Size::ZERO,
                },
            ))
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }
    impl ElementRenderObject for TileObject {
        fn layout(&mut self, constraints: LayoutConstraints) -> Size {
            self.tile.layouts.set(self.tile.layouts.get() + 1);
            self.size = constraints.constrain(Size::new(20., 20.));
            self.size
        }
        fn size(&self) -> Size {
            self.size
        }
        fn paint(&self, ctx: &mut PaintContext<'_>, origin: Point) -> bool {
            self.tile.paints.set(self.tile.paints.get() + 1);
            ctx.fill_rect(Rect::new(origin, self.size), Color::WHITE);
            true
        }
        fn render(&mut self) {}
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }
    struct Backend;
    impl PaintBackend for Backend {
        fn resize(&mut self, _: Size, _: u32) {}
        fn supports_retained_display_lists(&self) -> bool {
            true
        }
        fn render<'a>(
            &'a mut self,
            _: &PaintContext<'_>,
            _: Color,
            _: Option<&[Rect]>,
            _: Option<&[crate::compositor::DamageRect]>,
        ) -> crate::Result<BackendFrame<'a>> {
            Ok(BackendFrame::External)
        }
    }
    let tile = Tile {
        layouts: Rc::new(Cell::new(0)),
        paints: Rc::new(Cell::new(0)),
    };
    let make = |x| tile.clone().offset(x, 0.);
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_paint_backend(Box::new(Backend));
    pipeline.set_root(make(0.).create_element());
    pipeline.resize(Size::new(200., 40.));
    pipeline.render_for_present().unwrap();
    with_animation(Animation::linear(Duration::from_millis(200)), || {
        pipeline
            .element_tree_mut()
            .root_mut()
            .unwrap()
            .update(&make(100.));
    });
    pipeline.render_for_present().unwrap();
    let layouts = tile.layouts.get();
    let paints = tile.paints.get();
    for _ in 0..4 {
        pipeline.advance_visual_animations(Duration::from_millis(50));
        pipeline.render_for_present().unwrap();
    }
    assert_eq!(
        tile.layouts.get(),
        layouts,
        "translation must not relayout content"
    );
    assert_eq!(
        tile.paints.get(),
        paints,
        "translation must reuse retained geometry"
    );
    let root = pipeline.element_tree().root().unwrap();
    assert_eq!(root.children()[0].position(), Point::new(100., 0.));
    assert!(root.children()[0].hit_test(Point::new(110., 10.)));
    assert!(!root.children()[0].hit_test(Point::new(10., 10.)));
}

#[test]
fn moving_and_fading_outside_layout_bounds_clears_old_pixels() {
    for (opacity_scope, retained) in [(false, false), (true, false), (false, true), (true, true)] {
        let scene = |x, alpha| {
            let content = Spacer::new()
                .frame(20., 20.)
                .background(Color::WHITE)
                .offset(x, 0.);
            let content = if opacity_scope {
                crate::views::Either::A(content.opacity(alpha))
            } else {
                crate::views::Either::B(content)
            };
            Window::new("Damage", content)
                .decorated(false)
                .background_color(Color::BLACK)
                .size(Size::new(200., 40.))
        };
        let mut pipeline = RenderingPipeline::new();
        if retained {
            pipeline.set_paint_backend(Box::new(RetainedPixels(
                crate::renderer::CpuPaintRenderer::new(Size::new(200., 40.), 1000, Color::BLACK),
            )));
        }
        pipeline.set_root(scene(0., 1.).create_element());
        pipeline.layout_initial();
        pipeline.render_for_present().unwrap();
        let pixels = |pipeline: &mut RenderingPipeline| {
            let crate::renderer::PresentedFrame::Cpu { buffer, .. } =
                pipeline.render_for_present().unwrap()
            else {
                panic!("CPU frame")
            };
            [
                buffer.get_pixel(10, 10).unwrap(),
                buffer.get_pixel(110, 10).unwrap(),
            ]
        };
        with_animation(Animation::linear(Duration::from_millis(100)), || {
            pipeline
                .element_tree_mut()
                .root_mut()
                .unwrap()
                .update(&scene(100., 1.));
        });
        pipeline.render_for_present().unwrap();
        pipeline.advance_visual_animations(Duration::from_millis(100));
        assert_eq!(
            pixels(&mut pipeline),
            [Color::BLACK.to_bgra(), Color::WHITE.to_bgra()]
        );
        with_animation(Animation::linear(Duration::from_millis(100)), || {
            pipeline
                .element_tree_mut()
                .root_mut()
                .unwrap()
                .update(&scene(0., 1.));
        });
        pipeline.render_for_present().unwrap();
        pipeline.advance_visual_animations(Duration::from_millis(100));
        assert_eq!(
            pixels(&mut pipeline),
            [Color::WHITE.to_bgra(), Color::BLACK.to_bgra()]
        );
        if opacity_scope {
            with_animation(Animation::linear(Duration::from_millis(100)), || {
                pipeline
                    .element_tree_mut()
                    .root_mut()
                    .unwrap()
                    .update(&scene(100., 0.));
            });
            pipeline.render_for_present().unwrap();
            pipeline.advance_visual_animations(Duration::from_millis(100));
            assert_eq!(pixels(&mut pipeline), [Color::BLACK.to_bgra(); 2]);
        }
    }
}

#[test]
fn dense_animation_targets_complete_in_one_tree_pass() {
    #[derive(Clone)]
    struct Rows(Vec<Box<dyn View>>);
    impl crate::views::containers::ViewTuple for Rows {
        fn create_elements(&self) -> Vec<Box<dyn Element>> {
            self.0.iter().map(|v| v.create_element()).collect()
        }
        fn clone_views(&self) -> Vec<Box<dyn View>> {
            self.0.clone()
        }
        fn collect_listenables<'a>(&'a self, _: &mut Vec<&'a dyn Listenable>) {}
    }
    let scene = |width| {
        Window::new(
            "Dense",
            crate::views::VStack::new(Rows(
                (0..96)
                    .map(|_| Box::new(Spacer::new().frame(width, 1.)) as Box<dyn View>)
                    .collect(),
            ))
            .spacing(0.),
        )
        .decorated(false)
        .size(Size::new(200., 120.))
    };
    let mut pipeline = RenderingPipeline::new();
    pipeline.set_root(scene(40.).create_element());
    pipeline.layout_initial();
    pipeline.render_for_present().unwrap();
    with_animation(Animation::linear(Duration::from_millis(100)), || {
        pipeline
            .element_tree_mut()
            .root_mut()
            .unwrap()
            .update(&scene(80.));
    });
    pipeline.render_for_present().unwrap();
    fn widths(element: &dyn Element, result: &mut Vec<f32>) {
        if let Some(frame) = element
            .render_object()
            .and_then(|r| r.as_any().downcast_ref::<FrameRenderObject>())
        {
            result.push(frame.width_value().unwrap());
        }
        for child in element.children() {
            widths(child.as_ref(), result);
        }
    }
    for expected in [60., 80.] {
        assert!(pipeline.advance_visual_animations(Duration::from_millis(50)));
        let mut values = Vec::new();
        widths(pipeline.element_tree().root().unwrap(), &mut values);
        assert_eq!(values, vec![expected; 96]);
        pipeline.render_for_present().unwrap();
    }
    assert!(!pipeline.has_visual_animation());
}
