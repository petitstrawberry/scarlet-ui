# Opt-in input diagnostic

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
