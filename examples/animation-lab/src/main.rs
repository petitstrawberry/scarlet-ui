//! Native entry point for the shared Animation Lab.
#[cfg(not(target_arch = "wasm32"))]
mod basics;
#[cfg(not(target_arch = "wasm32"))]
mod code;
#[cfg(not(target_arch = "wasm32"))]
mod lab;
#[cfg(not(target_arch = "wasm32"))]
mod layout;
#[cfg(not(target_arch = "wasm32"))]
mod paint;
#[cfg(not(target_arch = "wasm32"))]
mod play;
#[cfg(not(target_arch = "wasm32"))]
mod source;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> scarlet_ui::Result<()> {
    use scarlet_ui::prelude::*;
    let smoke = std::env::var("SCARLET_UI_ANIMATION_LAB_SMOKE").is_ok_and(|value| value == "1");
    let mut app = lab::AnimationLab::new(smoke);
    app.run()
}
#[cfg(target_arch = "wasm32")]
fn main() {}
