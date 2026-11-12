#[test]
fn declarative_animation_waits_for_frame_grants_and_finishes_on_suspend() {
    use crate::{
        animation::{Animation, with_animation},
        view::ViewExt,
        views::modifiers::OpacityRenderObject,
    };
    let _environment_guard = install_test_input_environment(InputEnvironment::desktop());
    let mut app = RenderFailureApp(Rc::new(RefCell::new(Vec::new())));
    let platform_probe = Rc::new(RefCell::new(EnvironmentRunnerProbe::default()));
    let mut runner = ApplicationRunner::new(Box::new(EnvironmentTestBackend {
        probe: platform_probe.clone(),
        next_window: 0,
        negotiated_size: None,
    }));
    let declaration = collect_scene_declarations(&app).unwrap().remove(0);
    let mut slot = runner.create_slot(&mut app, declaration, true).unwrap();
    let view = |alpha| {
        crate::views::Spacer::new()
            .frame(20., 20.)
            .background(crate::Color::WHITE)
            .opacity(alpha)
    };
    slot.pipeline.set_root(view(1.0).create_element());
    slot.pipeline
        .set_paint_backend(Box::new(FailingPaintBackend(Rc::new(RefCell::new(
            RenderProbe::default(),
        )))));
    present_window(&mut app, &mut slot).unwrap();
    with_animation(Animation::linear(Duration::from_secs(1)), || {
        slot.pipeline
            .element_tree_mut()
            .root_mut()
            .unwrap()
            .update(&view(0.0));
    });
    present_window(&mut app, &mut slot).unwrap();
    let opacity = |slot: &WindowSlot<RenderFailureApp>| {
        slot.pipeline
            .element_tree()
            .root()
            .unwrap()
            .render_object()
            .unwrap()
            .as_any()
            .downcast_ref::<OpacityRenderObject>()
            .unwrap()
            .opacity()
    };
    slot.frame_pacing_enabled = true;
    slot.frame_ready = false;
    slot.frame_request_outstanding = true;
    slot.visual_animation_tick = Instant::now() - Duration::from_millis(200);
    let mut slots = vec![slot];
    let mut state = RunnerLoopState::new();
    for _ in 0..3 {
        ApplicationRunner::run_cycle(runner.backend.as_mut(), &mut app, &mut slots, &mut state)
            .unwrap();
        assert_eq!(
            opacity(&slots[0]),
            1.0,
            "poll cycles must not consume an ungranted visual frame"
        );
    }
    handle_window_event(
        &mut app,
        &mut slots[0],
        Event::FrameReady {
            presentation_time_ns: 1,
        },
        &mut Vec::new(),
    )
    .unwrap();
    ApplicationRunner::run_cycle(runner.backend.as_mut(), &mut app, &mut slots, &mut state)
        .unwrap();
    assert!(opacity(&slots[0]) < 0.9 && opacity(&slots[0]) > 0.0);
    assert!(!slots[0].frame_ready);
    assert!(slots[0].frame_request_outstanding);
    let once = opacity(&slots[0]);
    ApplicationRunner::run_cycle(runner.backend.as_mut(), &mut app, &mut slots, &mut state)
        .unwrap();
    assert_eq!(opacity(&slots[0]), once);
    handle_window_event(
        &mut app,
        &mut slots[0],
        Event::WindowSuspendedChanged { suspended: true },
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(opacity(&slots[0]), 0.0);
    assert!(!slots[0].pipeline.has_visual_animation());
}
