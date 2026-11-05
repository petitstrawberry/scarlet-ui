//! Browser entry point for the unchanged native Widget Factory showcase.
#![cfg(target_arch = "wasm32")]
mod factory;
use wasm_bindgen::{JsCast, prelude::*};

thread_local! {
    static APP: std::cell::RefCell<Option<scarlet_ui::WebApp>> = const { std::cell::RefCell::new(None) };
    static GENERATION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
#[wasm_bindgen]
pub async fn start() -> std::result::Result<(), JsValue> {
    start_app(
        factory::WidgetFactory::new(),
        scarlet_ui::SafeAreaMode::Viewport,
    )
    .await
}

/// Run the small shared-widget specimen used by the homepage.
#[wasm_bindgen]
pub async fn start_specimen() -> std::result::Result<(), JsValue> {
    start_app(
        factory::Specimen::new(),
        scarlet_ui::SafeAreaMode::Contained,
    )
    .await
}

async fn start_app<A: scarlet_ui::Application + scarlet_ui::View>(
    app: A,
    safe_area: scarlet_ui::SafeAreaMode,
) -> std::result::Result<(), JsValue> {
    console_error_panic_hook::set_once();
    stop();
    let generation = GENERATION.with(std::cell::Cell::get);
    scarlet_ui::graphics::set_default_font(include_bytes!(
        "../../web-counter/assets/DejaVuSans.ttf"
    ))
    .map_err(|_| JsValue::from_str("Invalid embedded font"))?;
    let canvas = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("ui"))
        .ok_or_else(|| JsValue::from_str("Missing #ui canvas"))?
        .dyn_into()?;
    let app =
        scarlet_ui::run_web_with_options(canvas, app, scarlet_ui::WebOptions { safe_area }).await?;
    // An intervening stop/restart invalidates this asynchronous initialization.
    // Dropping the stale handle cancels its frame before it can run.
    if GENERATION.with(std::cell::Cell::get) == generation {
        APP.with(|slot| *slot.borrow_mut() = Some(app));
    }
    Ok(())
}

/// Raw browser safe-area API values in CSS pixels: left, top, right, bottom.
/// Available before start(), including on devices without a WebGPU adapter.
#[wasm_bindgen]
pub fn browser_safe_area_insets() -> std::result::Result<Vec<f32>, JsValue> {
    let insets = scarlet_ui::browser_safe_area_insets()?;
    Ok(vec![insets.left, insets.top, insets.right, insets.bottom])
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

/// Pause frames without resetting the specimen's controls.
#[wasm_bindgen]
pub fn pause() {
    APP.with(|slot| {
        if let Some(app) = slot.borrow().as_ref() {
            app.pause();
        }
    });
}

/// Resume frames after the embed becomes visible again.
#[wasm_bindgen]
pub fn resume() {
    APP.with(|slot| {
        if let Some(app) = slot.borrow().as_ref() {
            app.resume();
        }
    });
}
