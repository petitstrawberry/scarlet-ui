//! The public View derive must build its content rather than recursively wrap itself.

use std::sync::atomic::{AtomicUsize, Ordering};

use scarlet_ui::{
    Application, ComponentElement, Scene, SceneBuilder, State, StateId, Text, View, Window,
    WindowGroup,
};

static CLONE_COUNT: AtomicUsize = AtomicUsize::new(0);
static CONTENT_COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(scarlet_ui_macros::View)]
#[view(body = build_counter)]
struct Counter {
    count: State<i32>,
}

impl Clone for Counter {
    fn clone(&self) -> Self {
        // Bound the old recursion so a regression fails without a stack overflow.
        let clone_count = CLONE_COUNT.fetch_add(1, Ordering::SeqCst);
        if clone_count >= 4 {
            eprintln!("derived View recursively creates itself instead of its content");
        }
        assert!(
            clone_count < 4,
            "derived View recursively creates itself instead of its content"
        );
        Self {
            count: self.count.clone(),
        }
    }
}

impl Counter {
    fn build_counter(&self) -> impl View + Clone + 'static {
        CONTENT_COUNT.fetch_add(1, Ordering::SeqCst);
        Text::new(format!("Counter: {}", self.count.get()))
    }
}

#[test]
fn derived_view_builds_content_and_retains_state() {
    CLONE_COUNT.store(0, Ordering::SeqCst);
    CONTENT_COUNT.store(0, Ordering::SeqCst);
    let counter = Counter::default();
    assert_eq!(counter.count.id(), StateId::new(0));
    assert_eq!(counter.count.get(), 0);
    assert_eq!(counter.listenables().len(), 1);

    let mut element = counter.create_element();
    assert_eq!(CONTENT_COUNT.load(Ordering::SeqCst), 1);
    assert_eq!(CLONE_COUNT.load(Ordering::SeqCst), 1);
    let component = element
        .as_any()
        .downcast_ref::<ComponentElement<Counter>>()
        .expect("the derive should create a ComponentElement");

    counter.count.set(7);
    assert_eq!(component.view().count.get(), 7);
    let child_id = element.children()[0].id();
    element.rebuild();
    assert_eq!(CONTENT_COUNT.load(Ordering::SeqCst), 2);
    assert_eq!(CLONE_COUNT.load(Ordering::SeqCst), 1);
    assert_eq!(element.children()[0].id(), child_id);
}

#[derive(scarlet_ui_macros::View, Clone)]
struct SceneApp {
    count: State<i32>,
    label: State<String>,
}

impl Application for SceneApp {
    fn scenes(&self) -> impl Scene {
        WindowGroup::new(
            "main",
            Window::new("Scene app", Text::new(self.label.get())),
        )
    }
}

#[test]
fn application_state_does_not_require_a_component_body() {
    let app = SceneApp::default();
    assert_eq!(app.count.id(), StateId::new(0));
    assert_eq!(app.label.id(), StateId::new(1));
    assert_eq!(app.listenables().len(), 2);
    let mut builder = SceneBuilder::new();
    app.scenes().build(&mut builder);
    let declarations = builder.into_declarations();
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].key.as_str(), "main");
    let _element = declarations[0].view.create_element();
}

#[test]
fn mounting_bodyless_derive_reports_configuration_error() {
    const CHILD_ENV: &str = "SCARLET_UI_DERIVE_BODYLESS_TEST_CHILD";
    if std::env::var_os(CHILD_ENV).is_some() {
        SceneApp::default().create_element();
        return;
    }

    // Isolate the expected panic so this also checks aborting host runtimes.
    let output = std::process::Command::new(std::env::current_exe().expect("test executable path"))
        .args([
            "--exact",
            "mounting_bodyless_derive_reports_configuration_error",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .output()
        .expect("run the body-less derive misuse in a child process");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("add #[view(body = method_name)]"),
        "expected a body configuration diagnostic, got: {stderr}"
    );
    assert!(!stderr.contains("stack overflow"));
}
