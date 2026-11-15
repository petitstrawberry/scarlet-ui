// Count actual view construction through the runner, not just resolver calls.
#[derive(Clone)]
struct TargetedSceneApp {
    revisions: [crate::state::State<u32>; 2],
    panel_available: crate::state::State<bool>,
    declarations: Rc<Cell<usize>>,
    bodies: Rc<Cell<[usize; 2]>>,
}

impl TargetedSceneApp {
    fn new() -> Self {
        Self {
            revisions: core::array::from_fn(|_| {
                crate::state::State::new(crate::state::generate_state_id(), 0)
            }),
            panel_available: crate::state::State::new(crate::state::generate_state_id(), true),
            declarations: Rc::new(Cell::new(0)),
            bodies: Rc::new(Cell::new([0; 2])),
        }
    }

    fn window(&self, index: usize) -> Window<Text> {
        let mut bodies = self.bodies.get();
        bodies[index] += 1;
        self.bodies.set(bodies);
        let label = alloc::format!("scene {index}: {}", self.revisions[index].get());
        Window::new(label.clone(), Text::new(label))
            .scene_key(if index == 0 { "main" } else { "panel" })
            .open_at_launch(index == 0)
            .size(Size::new(100.0, 40.0))
    }
}

impl Scene for TargetedSceneApp {
    fn build(self, builder: &mut SceneBuilder) {
        self.window(0).build(builder);
        if self.panel_available.get() {
            self.window(1).build(builder);
        }
    }
}

impl Application for TargetedSceneApp {
    fn scenes(&self) -> impl Scene {
        self.declarations.set(self.declarations.get() + 1);
        self.clone()
    }

    fn scene_view(&self, key: &SceneWindowKey) -> Option<Box<dyn View>> {
        match key.as_str() {
            "main" => Some(Box::new(self.window(0))),
            "panel" if self.panel_available.get() => Some(Box::new(self.window(1))),
            _ => None,
        }
    }

    fn scene_listenables(&self, key: &SceneWindowKey) -> Option<Vec<&dyn Listenable>> {
        Some(if key.as_str() == "main" {
            vec![&self.revisions[0]]
        } else {
            vec![&self.revisions[1], &self.panel_available]
        })
    }
}

impl View for TargetedSceneApp {
    fn create_element(&self) -> Box<dyn Element> {
        Text::new("scenes").create_element()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[test]
fn targeted_scene_rebuild_constructs_only_its_body_and_preserves_live_state() {
    let _environment_guard = install_test_input_environment(InputEnvironment::desktop());
    let mut app = TargetedSceneApp::new();
    let mut runner = scene_contract_runner();
    let declarations = collect_scene_declarations(&app).unwrap();
    let mut slots = runner.create_slots(&mut app, declarations).unwrap();
    assert_eq!(app.declarations.get(), 1);
    assert_eq!(app.bodies.get(), [1, 1], "mount reuses collected views");

    // State changed after construction must be read by the first frame.
    app.revisions[0].set(7);
    for slot in &mut slots {
        present_window(&mut app, slot).unwrap();
    }
    assert_eq!(app.bodies.get(), [2, 2]);
    let window_id = slots[0].pipeline.element_tree().root().unwrap().children()[0].id();
    assert_eq!(
        find_window_info(slots[0].pipeline.element_tree().root().unwrap())
            .unwrap()
            .title,
        "scene 0: 7"
    );

    for revision in 8..12 {
        app.revisions[0].set(revision);
        assert!(slots[0].pipeline.has_dirty());
        assert!(!slots[1].pipeline.has_dirty());
        present_window(&mut app, &mut slots[0]).unwrap();
        let root = slots[0].pipeline.element_tree().root().unwrap();
        assert_eq!(root.children()[0].id(), window_id, "reconcile in place");
        assert_eq!(
            find_window_info(root).unwrap().title,
            alloc::format!("scene 0: {revision}")
        );
    }
    assert_eq!(
        app.bodies.get(),
        [6, 2],
        "bar updates never construct the panel"
    );
    app.revisions[1].set(3);
    present_window(&mut app, &mut slots[1]).unwrap();
    assert_eq!(
        app.bodies.get(),
        [6, 3],
        "panel updates never construct the bar"
    );
    assert_eq!(
        app.declarations.get(),
        1,
        "hot rebuilds never collect all scenes"
    );

    // A dynamic missing view unmounts its content and can later reappear.
    app.panel_available.set(false);
    present_window(&mut app, &mut slots[1]).unwrap();
    assert!(
        slots[1]
            .pipeline
            .element_tree()
            .root()
            .unwrap()
            .children()
            .is_empty()
    );
    app.panel_available.set(true);
    present_window(&mut app, &mut slots[1]).unwrap();
    assert_eq!(
        find_window_info(slots[1].pipeline.element_tree().root().unwrap())
            .unwrap()
            .title,
        "scene 1: 3"
    );
    for slot in &mut slots {
        slot.pipeline.teardown();
    }
    app.revisions[0].set(20);
    app.revisions[1].set(20);
    assert!(slots.iter().all(|slot| !slot.pipeline.has_dirty()));
}

#[test]
fn targeted_scene_commands_recheck_dynamic_declarations_on_reopen() {
    let _environment_guard = install_test_input_environment(InputEnvironment::desktop());
    let mut app = TargetedSceneApp::new();
    let mut runner = scene_contract_runner();
    let mut slots = Vec::new();
    app.panel_available.set(false);
    crate::open_window("panel");
    runner
        .handle_application_commands(&mut app, &mut slots)
        .unwrap();
    assert!(slots.is_empty());

    app.panel_available.set(true);
    crate::open_window("panel");
    runner
        .handle_application_commands(&mut app, &mut slots)
        .unwrap();
    assert_eq!(slots.len(), 1);
    present_window(&mut app, &mut slots[0]).unwrap();
    let first_id = slots[0].context.window_id;
    let declarations = app.declarations.get();
    crate::open_window("panel");
    runner
        .handle_application_commands(&mut app, &mut slots)
        .unwrap();
    assert_eq!(
        app.declarations.get(),
        declarations,
        "existing window is reused"
    );

    crate::dismiss_window("panel");
    runner
        .handle_application_commands(&mut app, &mut slots)
        .unwrap();
    assert!(slots.is_empty());
    app.revisions[1].set(9);
    crate::open_window("panel");
    runner
        .handle_application_commands(&mut app, &mut slots)
        .unwrap();
    assert_eq!(slots.len(), 1);
    assert_ne!(slots[0].context.window_id, first_id);
    assert_eq!(app.declarations.get(), declarations + 1);
    present_window(&mut app, &mut slots[0]).unwrap();
    assert_eq!(
        find_window_info(slots[0].pipeline.element_tree().root().unwrap())
            .unwrap()
            .title,
        "scene 1: 9"
    );
    crate::dismiss_window("panel");
    runner
        .handle_application_commands(&mut app, &mut slots)
        .unwrap();
}
