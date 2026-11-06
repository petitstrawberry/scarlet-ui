# Opt-in diagnostics

## Safe-area API

`/diagnostics/safe-area/` reads `browser_safe_area_insets()` from the ordinary
Widget Factory WebAssembly module before starting an application. It requires
no canvas or WebGPU adapter, so it also works when an iOS Simulator cannot
create a WebGPU device. The page shows raw CSS pixel values and viewport
geometry; rotate the device or use **再取得** to read them again. DOM read errors
are displayed as errors, separately from valid zero values reported by the browser.

The Rust facade exposes `browser_safe_area_insets()` without a running WebApp.
Full-window apps use `SafeAreaMode::Viewport`; embeds whose DOM container owns
safe-area placement explicitly use `SafeAreaMode::Contained` through
`run_web_with_options`. Native hosts forward their OS API result with
`WebApp::set_safe_area_insets` when CSS env values are unavailable.

This route is opt-in and is published only by the normal main deployment.

## Input diagnostic

`tools/build-web-samples.py` generates `/diagnostics/input/` from the same
Widget Factory HTML, importing the ordinary `/widget-factory/pkg/` runtime.
Normal pages do not load this recorder. The route is `noindex,nofollow` and
is not linked in navigation. Pages still deploys only through the existing
main workflow; creating a PR does not deploy the route.

After reproducing the problem on trusted HTTPS, open **診断ログ** at the top
right and choose **ログをダウンロード**. Share that JSON for diagnosis. Opening
this control itself changes focus; these events are labeled diagnostic-control.

The recorder observes pointer/click, focus, input-event *types*, composition
start/end, viewport and editor geometry. It never reads entered text, input
`data`, selection ranges or clipboard contents, and never uploads a log.
A local download contains the build commit, browser user agent, timestamps,
and synchronous focus-event stacks. The last 400 causal events and 200
geometry events are retained separately, with dropped-event counts.

Listeners do not cancel events or override browser focus APIs. Unchanged
editor style mutations are discarded before layout reads. Instrumentation
still adds some timing overhead; a stack without a JavaScript focus caller
does not prove the browser caused dismissal. This route diagnoses behavior;
it does not fix the keyboard or claim physical-iOS verification.
