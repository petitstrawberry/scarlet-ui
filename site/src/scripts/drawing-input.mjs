/** Route a paper-only input region to the existing rendered canvas.
 * The region paints nothing: it only gives the browser an honest touch-action
 * boundary before a gesture starts. Toolbar and surrounding page can still pan.
 */
export function installDrawingInput(region, canvas, PointerEventType = globalThis.PointerEvent) {
  if (!region || typeof PointerEventType !== 'function') return () => {};
  const types = ['pointerdown', 'pointermove', 'pointerup', 'pointercancel'];
  const forward = event => {
    if (canvas.inert) return;
    const forwarded = new PointerEventType(event.type, {
      bubbles: true, cancelable: true, composed: true,
      pointerId: event.pointerId, pointerType: event.pointerType, isPrimary: event.isPrimary,
      clientX: event.clientX, clientY: event.clientY,
      screenX: event.screenX, screenY: event.screenY,
      button: event.button, buttons: event.buttons,
      pressure: event.pressure, width: event.width, height: event.height,
      tiltX: event.tiltX, tiltY: event.tiltY, twist: event.twist,
      ctrlKey: event.ctrlKey, shiftKey: event.shiftKey, altKey: event.altKey, metaKey: event.metaKey,
    });
    // Existing platform-web code owns pointer capture and cancellation. Once it
    // captures this active pointer, following events go directly to the canvas.
    canvas.dispatchEvent(forwarded);
    event.stopPropagation();
    if (forwarded.defaultPrevented) event.preventDefault();
  };
  for (const type of types) region.addEventListener(type, forward);
  return () => { for (const type of types) region.removeEventListener(type, forward); };
}
