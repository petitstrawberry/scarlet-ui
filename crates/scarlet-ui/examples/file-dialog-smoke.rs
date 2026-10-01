//! Isolated interactive AppKit smoke test. No document or filesystem writes.
//! Run with `cargo run -p scarlet-ui --example file-dialog-smoke --features platform-winit`.
use scarlet_ui::{PlatformWindow, WindowContext, WindowId, file_dialog::*, prelude::*};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Probe {
    inner: Rc<RefCell<Model>>,
}
struct Model {
    owner: Option<WindowId>,
    receipt: Option<FileDialogHandle>,
    stage: usize,
    started: Instant,
    shutdown_at: Option<Instant>,
}
impl Application for Probe {
    fn scenes(&self) -> impl Scene {
        Window::new(
            "ScarletUI file dialog verification",
            Text::new("Isolated dialog probe — no files are written"),
        )
        .scene_key("probe")
        .size(Size::new(520., 180.))
    }
    fn on_window_created(&mut self, ctx: &WindowContext, _: &mut dyn PlatformWindow) {
        self.inner.borrow_mut().owner = Some(ctx.window_id);
    }
    fn on_idle(&mut self) {
        let mut m = self.inner.borrow_mut();
        if let Some(end) = m.shutdown_at {
            if end.elapsed() > Duration::from_secs(2) {
                scarlet_ui::dismiss_window("probe");
            }
            return;
        }
        if let Some(receipt) = &m.receipt {
            if let Some(result) = receipt.take_result() {
                println!("dialog stage {}: {:?}", m.stage, result);
                m.receipt = None;
                m.stage += 1;
                m.started = Instant::now();
                if m.stage == 4 {
                    m.shutdown_at = Some(Instant::now());
                }
            } else if m.stage == 3 && m.started.elapsed() > Duration::from_secs(2) {
                scarlet_ui::dismiss_window("probe");
            } else if m.started.elapsed() > Duration::from_secs(30) {
                receipt.cancel();
            }
            return;
        }
        if m.started.elapsed() < Duration::from_secs(2) {
            return;
        }
        let mode = [
            FileDialogMode::Open,
            FileDialogMode::OpenMultiple,
            FileDialogMode::Save,
            FileDialogMode::Open,
        ][m.stage];
        let mut options = FileDialog::new(mode);
        options.title = format!("ScarletUI isolated probe: {mode:?}");
        options.initial_directory =
            Some(std::path::PathBuf::from("/tmp/scarlet-dialog-fixtures").into());
        options.default_name = Some(
            if m.stage == 2 {
                "検証.wav"
            } else {
                "alpha.wav"
            }
            .into(),
        );
        options.filters = vec![FileDialogFilter {
            name: "WAV".into(),
            extensions: vec!["wav".into()],
        }];
        println!("opening stage {}: {mode:?}", m.stage);
        m.receipt = Some(options.show(m.owner.unwrap()));
        m.started = Instant::now();
    }
}
impl View for Probe {
    fn create_element(&self) -> Box<dyn Element> {
        Text::new("Dialog probe").create_element()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
fn main() {
    let mut app = Probe {
        inner: Rc::new(RefCell::new(Model {
            owner: None,
            receipt: None,
            stage: 0,
            started: Instant::now(),
            shutdown_at: None,
        })),
    };
    app.run().unwrap();
    if let Some(receipt) = &app.inner.borrow().receipt {
        println!("owner teardown: {:?}", receipt.take_result());
    }
    println!("file-dialog smoke finished");
}
