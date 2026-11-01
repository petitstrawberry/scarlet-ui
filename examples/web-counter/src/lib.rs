#![cfg(target_arch = "wasm32")]
use scarlet_ui::{hstack, prelude::*, vstack};
use wasm_bindgen::{JsCast, prelude::*};

#[derive(Clone)]
struct Counter {
    count: State<i32>,
    compact: bool,
}
impl Counter {
    fn controls(&self) -> impl View + Clone + use<> {
        let count = self.count.clone();
        // docs:counter-controls:start
        vstack! {
            Text::new(format!(
                "Count: {}", count.get()
            ))
                .font_size(24.0),
            Button::new("+1")
                .on_click(move || {
                    count.update(|n| {
                        *n += 1;
                    });
                }),
        }
        .spacing(16.0)
        // docs:counter-controls:end
    }

    fn body(&self) -> impl View + Clone + use<> {
        if self.compact {
            Either::A(vstack![
                Spacer::new(),
                hstack![Spacer::new(), self.controls(), Spacer::new()],
                Spacer::new(),
            ])
        } else {
            // Keep the standalone showcase unchanged; the homepage excerpt is compact.
            let count = self.count.clone();
            Either::B(
                vstack![
                    Text::new("ScarletUI / WebGPU").font_size(28.0),
                    Text::new(format!("Count: {}", self.count.get())).font_size(24.0),
                    Button::new("Increment").on_click(move || count.set(count.get() + 1)),
                ]
                .spacing(20.0)
                .padding(32.0),
            )
        }
    }
}
impl View for Counter {
    fn create_element(&self) -> Box<dyn Element> {
        self.body().create_element()
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.count]
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl Application for Counter {
    fn scenes(&self) -> impl Scene {
        WindowGroup::new(
            "main",
            Window::new("WebGPU counter", self.body()).size(Size::new(640.0, 480.0)),
        )
    }
}
thread_local! {
    static APP: std::cell::RefCell<Option<scarlet_ui::WebApp>> = const { std::cell::RefCell::new(None) };
    static GENERATION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
#[wasm_bindgen]
pub async fn start() -> std::result::Result<(), JsValue> {
    start_on("ui", false).await
}

/// Run the documentation's compiled counter controls without the showcase title.
#[wasm_bindgen]
pub async fn start_example() -> std::result::Result<(), JsValue> {
    start_on("counter-example", true).await
}

async fn start_on(canvas_id: &str, compact: bool) -> std::result::Result<(), JsValue> {
    console_error_panic_hook::set_once();
    stop();
    let generation = GENERATION.with(std::cell::Cell::get);
    scarlet_ui::graphics::set_default_font(include_bytes!("../assets/DejaVuSans.ttf"))
        .map_err(|_| JsValue::from_str("Invalid embedded font"))?;
    let canvas = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(canvas_id))
        .ok_or_else(|| JsValue::from_str(&format!("Missing #{canvas_id} canvas")))?
        .dyn_into()?;
    let app = scarlet_ui::run_web(
        canvas,
        Counter {
            count: State::new(StateId::new(1), 0),
            compact,
        },
    )
    .await?;
    // An intervening stop/restart invalidates this asynchronous initialization.
    // Dropping the stale handle cancels its frame before it can run.
    if GENERATION.with(std::cell::Cell::get) == generation {
        APP.with(|slot| *slot.borrow_mut() = Some(app));
    }
    Ok(())
}

/// Stop the example and immediately release its frame callback and listeners.
#[wasm_bindgen]
pub fn stop() {
    GENERATION.with(|generation| generation.set(generation.get().wrapping_add(1)));
    APP.with(|slot| {
        slot.borrow_mut().take();
    });
}

/// Runtime rendering errors, distinct from startup/adapter failures.
#[wasm_bindgen]
pub fn runtime_error() -> Option<String> {
    APP.with(|slot| slot.borrow().as_ref().and_then(scarlet_ui::WebApp::error))
}

/// Suspend frames while preserving the current count.
#[wasm_bindgen]
pub fn pause() {
    APP.with(|slot| {
        if let Some(app) = slot.borrow().as_ref() {
            app.pause();
        }
    });
}

/// Resume after the counter example becomes visible again.
#[wasm_bindgen]
pub fn resume() {
    APP.with(|slot| {
        if let Some(app) = slot.borrow().as_ref() {
            app.resume();
        }
    });
}
