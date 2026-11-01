import test from 'node:test';
import assert from 'node:assert/strict';
import { installDrawingInput } from '../src/scripts/drawing-input.mjs';
class Pointer extends Event {
  constructor(type, options = {}) {
    super(type, options);
    for (const [key, value] of Object.entries(options)) {
      if (!['bubbles', 'cancelable', 'composed'].includes(key)) Object.defineProperty(this, key, { value });
    }
  }
}
function setup() {
  const region = new EventTarget();
  const canvas = new EventTarget();
  canvas.inert = false;
  const events = [];
  for (const type of ['pointerdown', 'pointermove', 'pointerup', 'pointercancel']) {
    canvas.addEventListener(type, event => { events.push(event); event.preventDefault(); });
  }
  return { region, canvas, events, cleanup: installDrawingInput(region, canvas, Pointer) };
}
test('paper forwards genuine input coordinates, identity and cancellation to the canvas', () => {
  const { region, events, cleanup } = setup();
  for (const type of ['pointerdown', 'pointermove', 'pointerup', 'pointercancel']) {
    const event = new Pointer(type, { cancelable: true, pointerId: 7, pointerType: 'touch', isPrimary: true, clientX: 230, clientY: 180, button: 0, buttons: 1, pressure: 0.7 });
    region.dispatchEvent(event);
    const copy = events.at(-1);
    assert.equal(copy.type, type); assert.equal(copy.pointerId, 7);
    assert.equal(copy.pointerType, 'touch'); assert.equal(copy.clientX, 230);
    assert.equal(copy.clientY, 180); assert.equal(copy.pressure, 0.7);
    assert.equal(event.defaultPrevented, true);
  }
  cleanup();
  region.dispatchEvent(new Pointer('pointerdown'));
  assert.equal(events.length, 4);
});
test('paused/fallback canvas receives no drawing events; wheel and keyboard remain browser-owned', () => {
  const { region, canvas, events } = setup();
  canvas.inert = true;
  region.dispatchEvent(new Pointer('pointerdown'));
  const wheel = new Event('wheel', { cancelable: true });
  const key = new Event('keydown', { cancelable: true });
  region.dispatchEvent(wheel); region.dispatchEvent(key);
  assert.equal(events.length, 0);
  assert.equal(wheel.defaultPrevented, false); assert.equal(key.defaultPrevented, false);
});
test('missing input region is a no-op for the independent counter specimen', () => {
  assert.doesNotThrow(() => installDrawingInput(null, new EventTarget(), Pointer)());
});
