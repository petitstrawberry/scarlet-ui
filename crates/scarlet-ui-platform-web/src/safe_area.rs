//! Browser/host safe-area acquisition, independent of the GPU backend.
use scarlet_ui_core::geometry::{EdgeInsets, Size};

/// Who owns safe-area layout for a browser canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SafeAreaMode {
    /// The canvas fills the browser viewport; Window content respects CSS env().
    #[default]
    Viewport,
    /// A DOM container already positions this embedded canvas safely.
    Contained,
}

fn normalized(insets: EdgeInsets) -> EdgeInsets {
    let edge = |v: f32| if v.is_finite() { v.max(0.) } else { 0. };
    EdgeInsets::new(
        edge(insets.left),
        edge(insets.top),
        edge(insets.right),
        edge(insets.bottom),
    )
}

/// Convert API values to canvas logical units. Ownership is explicit: viewport
/// comparisons and document scrolling must not silently discard system insets.
pub(crate) fn canvas_insets(
    mode: SafeAreaMode,
    css: Size,
    logical: Size,
    insets: EdgeInsets,
) -> EdgeInsets {
    if mode == SafeAreaMode::Contained || css.width <= 0. || css.height <= 0. {
        return EdgeInsets::ZERO;
    }
    let insets = normalized(insets);
    let x = logical.width / css.width;
    let y = logical.height / css.height;
    EdgeInsets::new(
        insets.left * x,
        insets.top * y,
        insets.right * x,
        insets.bottom * y,
    )
}

#[cfg(target_arch = "wasm32")]
pub(crate) use browser::SafeArea;
#[cfg(target_arch = "wasm32")]
pub use browser::browser_safe_area_insets;

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use std::{cell::Cell, rc::Rc};
    use wasm_bindgen::{JsCast, JsValue, closure::Closure};
    use web_sys::{HtmlCanvasElement, HtmlElement};

    pub(crate) struct SafeArea {
        canvas: HtmlCanvasElement,
        mode: SafeAreaMode,
        probe: HtmlElement,
        pending: Rc<Cell<bool>>,
        current: Cell<EdgeInsets>,
        host_override: Cell<Option<EdgeInsets>>,
        observer: web_sys::ResizeObserver,
        _observer_callback: Closure<dyn FnMut()>,
    }
    impl SafeArea {
        pub fn new(canvas: HtmlCanvasElement, mode: SafeAreaMode) -> Result<Self, JsValue> {
            let doc = canvas
                .owner_document()
                .ok_or_else(|| JsValue::from_str("Canvas has no document"))?;
            let probe: HtmlElement = doc.create_element("div")?.dyn_into()?;
            probe.set_attribute("aria-hidden", "true")?;
            probe.style().set_css_text("position:fixed;left:0;top:0;width:env(safe-area-inset-left,0px);height:env(safe-area-inset-top,0px);visibility:hidden;pointer-events:none;padding:env(safe-area-inset-top,0px) env(safe-area-inset-right,0px) env(safe-area-inset-bottom,0px) env(safe-area-inset-left,0px)");
            let trailing: HtmlElement = doc.create_element("div")?.dyn_into()?;
            trailing.style().set_css_text("position:absolute;left:0;top:0;width:env(safe-area-inset-right,0px);height:env(safe-area-inset-bottom,0px)");
            probe.append_child(&trailing)?;
            // Two content-box sensors watch all four individual env values,
            // including changes that leave both viewport size and inset sums
            // unchanged. Only the invalidation flag is set in this callback.
            let pending = Rc::new(Cell::new(true));
            let flag = pending.clone();
            let callback = Closure::wrap(Box::new(move || flag.set(true)) as Box<dyn FnMut()>);
            let observer = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref())?;
            observer.observe(&probe);
            observer.observe(&trailing);
            doc.body()
                .ok_or_else(|| JsValue::from_str("Document has no body"))?
                .append_child(&probe)?;
            Ok(Self {
                canvas,
                mode,
                probe,
                pending,
                current: Cell::new(EdgeInsets::ZERO),
                host_override: Cell::new(None),
                observer,
                _observer_callback: callback,
            })
        }
        pub fn invalidate(&self) {
            self.pending.set(true);
        }
        /// Native hosts pass their actual safeAreaInsets in canvas logical units.
        /// None restores automatic browser CSS env() acquisition.
        pub fn set_host_override(&self, insets: Option<EdgeInsets>) {
            self.host_override.set(insets.map(normalized));
            self.invalidate();
        }
        pub fn insets(&self) -> EdgeInsets {
            if self.pending.replace(false) {
                let insets = self
                    .host_override
                    .get()
                    .unwrap_or_else(|| self.browser_insets());
                self.current.set(insets);
            }
            self.current.get()
        }
        fn browser_insets(&self) -> EdgeInsets {
            if self.mode == SafeAreaMode::Contained {
                return EdgeInsets::ZERO;
            }
            let Ok(insets) = read_probe(&self.probe) else {
                // Preserve the last known safe area on a DOM read failure.
                // The explicit getter below reports failures to its caller.
                return self.current.get();
            };
            let rect = self.canvas.get_bounding_client_rect();
            canvas_insets(
                self.mode,
                Size::new(rect.width() as f32, rect.height() as f32),
                Size::new(
                    self.canvas.client_width() as f32,
                    self.canvas.client_height() as f32,
                ),
                insets,
            )
        }
    }

    fn read_probe(probe: &HtmlElement) -> Result<EdgeInsets, JsValue> {
        let window = probe
            .owner_document()
            .and_then(|doc| doc.default_view())
            .ok_or_else(|| JsValue::from_str("Safe-area document has no window"))?;
        let style = window
            .get_computed_style(probe)?
            .ok_or_else(|| JsValue::from_str("Safe-area computed style is unavailable"))?;
        let edge = |name| -> Result<f32, JsValue> {
            style
                .get_property_value(name)?
                .trim()
                .trim_end_matches("px")
                .parse::<f32>()
                .map_err(|_| JsValue::from_str("Safe-area value is not a CSS pixel length"))
        };
        Ok(normalized(EdgeInsets::new(
            edge("padding-left")?,
            edge("padding-top")?,
            edge("padding-right")?,
            edge("padding-bottom")?,
        )))
    }

    /// Read the browser's raw CSS env(safe-area-inset-*) values, in CSS pixels.
    ///
    /// This one-shot API needs a document body but no canvas, WebGPU adapter,
    /// or running WebApp. Zero means the browser reported zero; DOM failures
    /// return an error. Full-screen pages opt in with viewport-fit=cover.
    /// Native hosts that do not expose these CSS variables use their OS API
    /// and WebApp::set_safe_area_insets instead of inferring device dimensions.
    pub fn browser_safe_area_insets() -> Result<EdgeInsets, JsValue> {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .ok_or_else(|| JsValue::from_str("Safe-area document is unavailable"))?;
        let probe: HtmlElement = document.create_element("div")?.dyn_into()?;
        probe.set_attribute("aria-hidden", "true")?;
        probe.style().set_css_text("position:fixed;visibility:hidden;pointer-events:none;width:0;height:0;padding:env(safe-area-inset-top,0px) env(safe-area-inset-right,0px) env(safe-area-inset-bottom,0px) env(safe-area-inset-left,0px)");
        document
            .body()
            .ok_or_else(|| JsValue::from_str("Safe-area document has no body"))?
            .append_child(&probe)?;
        let result = read_probe(&probe);
        probe.remove();
        result
    }
    impl Drop for SafeArea {
        fn drop(&mut self) {
            self.observer.disconnect();
            self.probe.remove();
        }
    }
}
