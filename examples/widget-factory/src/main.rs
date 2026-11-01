//! Native entry point; the showcase itself is shared with the WebGPU example.
#[cfg(not(target_arch = "wasm32"))]
mod factory;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> scarlet_ui::Result<()> {
    factory::run_native()
}

#[cfg(target_arch = "wasm32")]
fn main() {}
