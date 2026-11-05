//! Browser/host safe-area acquisition, independent of the GPU backend.
use scarlet_ui_core::geometry::{EdgeInsets, Rect, Size};

fn normalized(insets: EdgeInsets) -> EdgeInsets {
    let edge = |v: f32| if v.is_finite() { v.max(0.) } else { 0. };
    EdgeInsets::new(
        edge(insets.left),
        edge(insets.top),
        edge(insets.right),
        edge(insets.bottom),
    )
}

/// Project browser-reserved edges into canvas-local logical coordinates.
/// A canvas already inset by its host gets zero. Offscreen document specimens
/// keep their authored layout instead of reflowing as the document scrolls.
pub(crate) fn canvas_insets(
    viewport: Size,
    canvas: Rect,
    logical: Size,
    insets: EdgeInsets,
) -> EdgeInsets {
    if canvas.size.width <= 0.
        || canvas.size.height <= 0.
        || canvas.left() < -0.5
        || canvas.top() < -0.5
        || canvas.right() > viewport.width + 0.5
        || canvas.bottom() > viewport.height + 0.5
    {
        return EdgeInsets::ZERO;
    }
    let insets = normalized(insets);
    let x = logical.width / canvas.size.width;
    let y = logical.height / canvas.size.height;
    EdgeInsets::new(
        (insets.left - canvas.left()).max(0.).min(canvas.size.width) * x,
        (insets.top - canvas.top()).max(0.).min(canvas.size.height) * y,
        (canvas.right() - viewport.width + insets.right)
            .max(0.)
            .min(canvas.size.width)
            * x,
        (canvas.bottom() - viewport.height + insets.bottom)
            .max(0.)
            .min(canvas.size.height)
            * y,
    )
}

#[cfg(target_arch = "wasm32")]
pub(crate) use browser::SafeArea;

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use std::{cell::Cell, rc::Rc};
    use wasm_bindgen::{JsCast, JsValue, closure::Closure};
    use web_sys::{HtmlCanvasElement, HtmlElement};

    pub(crate) struct SafeArea {
        canvas: HtmlCanvasElement,
        probe: HtmlElement,
        pending: Rc<Cell<bool>>,
        current: Cell<EdgeInsets>,
        host_override: Cell<Option<EdgeInsets>>,
        observer: web_sys::ResizeObserver,
        _observer_callback: Closure<dyn FnMut()>,
    }
    impl SafeArea {
        pub fn new(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
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
            let Some(window) = web_sys::window() else {
                return EdgeInsets::ZERO;
            };
            let Some(style) = window.get_computed_style(&self.probe).ok().flatten() else {
                return EdgeInsets::ZERO;
            };
            let edge = |name| {
                style
                    .get_property_value(name)
                    .ok()
                    .and_then(|s| s.trim().trim_end_matches("px").parse::<f32>().ok())
                    .unwrap_or(0.)
            };
            let insets = EdgeInsets::new(
                edge("padding-left"),
                edge("padding-top"),
                edge("padding-right"),
                edge("padding-bottom"),
            );
            let rect = self.canvas.get_bounding_client_rect();
            let width = window
                .inner_width()
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.) as f32;
            let height = window
                .inner_height()
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.) as f32;
            canvas_insets(
                Size::new(width, height),
                Rect::from_xywh(
                    rect.x() as f32,
                    rect.y() as f32,
                    rect.width() as f32,
                    rect.height() as f32,
                ),
                Size::new(
                    self.canvas.client_width() as f32,
                    self.canvas.client_height() as f32,
                ),
                insets,
            )
        }
    }
    impl Drop for SafeArea {
        fn drop(&mut self) {
            self.observer.disconnect();
            self.probe.remove();
        }
    }
}
