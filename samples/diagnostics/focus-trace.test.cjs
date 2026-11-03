'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');
const source = fs.readFileSync(path.join(__dirname, 'focus-trace.js'), 'utf8');

// Execute the shipping tracer unchanged. This fixture deliberately provides no
// browser editing/focus implementation: observation must not invoke either.
function fixture() {
  let reads = 0;
  let now = 0;
  let blob;
  const downloads = [];
  const observers = [];
  const timers = [];
  class Target {
    constructor() { this.listeners = []; }
    addEventListener(kind, callback, options) { this.listeners.push({ kind, callback, options }); }
    emit(kind, event = {}) {
      event.type = kind;
      for (const item of this.listeners) if (item.kind === kind) item.callback(event);
    }
  }
  class Element extends Target {
    constructor(tag) {
      super(); this.tagName = tag.toUpperCase(); this.dataset = {}; this.style = {};
      this.children = []; this.inert = false;
    }
    matches(selector) { return selector === '[data-scarlet-text-input]' && this === editor; }
    closest() { return this.dataset.focusTraceUi !== undefined ? this : this.parent?.closest() ?? null; }
    append(child) { child.parent = this; this.children.push(child); }
    getBoundingClientRect() { reads++; return { x: 10, y: 20, width: 300, height: 40 }; }
    focus() { throw new Error('Tracer must not focus'); }
    blur() { throw new Error('Tracer must not blur'); }
    click() { downloads.push({ href: this.href, download: this.download }); }
  }
  class MouseEvent {}
  class PointerEvent extends MouseEvent {}
  class InputEvent {}
  class MutationObserver {
    constructor(callback) { this.callback = callback; observers.push(this); }
    observe(target, options) { this.target = target; this.options = options; }
    disconnect() { this.target = null; }
  }
  const canvas = new Element('canvas'); canvas.id = 'ui';
  const editor = new Element('textarea');
  Object.assign(editor.style, { left: '10px', top: '20px', height: '16px' });
  for (const property of ['value', 'selectionStart', 'selectionEnd', 'selectionDirection']) {
    Object.defineProperty(editor, property, { get() { throw new Error(`Private editor property read: ${property}`); } });
  }
  const identity = { source: 'ScarletUI focus QA', commit: 'test-revision', artifact: 'test-artifact' };
  const build = { textContent: JSON.stringify(identity) };
  const document = {
    activeElement: editor, visibilityState: 'visible', documentElement: new Element('html'), body: new Element('body'),
    hasFocus: () => true,
    createElement: tag => new Element(tag),
    querySelector: selector => ({ canvas, '[data-scarlet-text-input]': editor, '#scarlet-diagnostic-build': build })[selector] ?? null,
  };
  const window = new Target();
  window.visualViewport = Object.assign(new Target(), { width: 390, height: 600, offsetLeft: 0, offsetTop: 0, scale: 1 });
  const forbidden = () => { throw new Error('Unexpected network/interception operation'); };
  const prototypeBefore = Object.getOwnPropertyDescriptors(Element.prototype);
  const context = vm.createContext({
    window, document, Element, MouseEvent, PointerEvent, InputEvent, MutationObserver,
    performance: { now: () => ++now }, navigator: { userAgent: 'test-browser', sendBeacon: forbidden },
    innerWidth: 390, innerHeight: 800, scrollX: 0, scrollY: 0, devicePixelRatio: 3, isSecureContext: true,
    fetch: forbidden, XMLHttpRequest: forbidden, WebSocket: forbidden,
    Blob: class { constructor(parts, options) { this.text = parts.join(''); this.type = options.type; } },
    URL: { createObjectURL(value) { blob = value; return 'blob:local-test'; }, revokeObjectURL() {} },
    setTimeout: (callback, delay) => timers.push({ callback, delay }),
  });
  vm.runInContext(source, context, { filename: 'focus-trace.js' });
  assert.deepEqual(Object.getOwnPropertyDescriptors(Element.prototype), prototypeBefore);
  const button = document.body.children[0].children.find(child => child.tagName === 'BUTTON');
  return {
    window, editor, document, identity, observers, downloads, timers, InputEvent, PointerEvent,
    get reads() { return reads; },
    mutation() { observers.find(observer => observer.target === editor).callback(); },
    export() { button.emit('click'); assert.equal(blob.type, 'application/json'); return JSON.parse(blob.text); },
  };
}

function event(target, extra = {}) {
  return Object.assign({ target, relatedTarget: null, isTrusted: true,
    preventDefault() { throw new Error('Must not prevent defaults'); },
    stopPropagation() { throw new Error('Must not intercept propagation'); },
    stopImmediatePropagation() { throw new Error('Must not intercept propagation'); },
  }, extra);
}

test('exports metadata and event categories without reading typed text or selection', () => {
  const f = fixture();
  const input = Object.assign(new f.InputEvent(), event(f.editor), { inputType: 'insertText', isComposing: false });
  Object.defineProperty(input, 'data', { get() { throw new Error('Private input data read'); } });
  f.window.emit('input', input);
  f.window.emit('compositionend', event(f.editor, { data: 'SECRET_TYPED_TEXT' }));
  f.window.emit('focus', event(f.editor));
  const payload = f.export();
  assert.equal(payload.commit, f.identity.commit);
  assert.equal(payload.artifact, f.identity.artifact);
  assert.equal(payload.secureContext, true);
  const record = payload.events.find(item => item.kind === 'input');
  assert.equal(record.target, 'scarlet-textarea');
  assert.equal(record.inputType, 'insertText');
  assert.equal(record.composing, false);
  const forbiddenKeys = new Set(['value', 'data', 'selectionStart', 'selectionEnd', 'selectionDirection', 'key', 'code']);
  function inspect(value) {
    if (value && typeof value === 'object') for (const [key, child] of Object.entries(value)) {
      assert.equal(forbiddenKeys.has(key), false, key); inspect(child);
    }
  }
  inspect(payload);
  assert.equal(JSON.stringify(payload).includes('SECRET_TYPED_TEXT'), false);
  assert.ok(payload.events.find(item => item.kind === 'focus').stack.length <= 1800);
  assert.deepEqual(f.downloads, [{ href: 'blob:local-test', download: 'scarlet-focus-diagnostic.json' }]);
  assert.equal(f.timers[0].delay, 60000);
});

test('all observation listeners are passive and do not install touch or keyboard interception', () => {
  const f = fixture();
  for (const target of [f.window, f.window.visualViewport]) for (const listener of target.listeners) {
    assert.equal(listener.options.passive, true);
    if (!['resize', 'scroll'].includes(listener.kind)) assert.equal(listener.options.capture, true);
    assert.doesNotMatch(listener.kind, /^(touch|key)/);
    target.emit(listener.kind, event(f.editor));
  }
  const pointer = Object.assign(new f.PointerEvent(), event(f.editor), {
    pointerType: 'touch', pointerId: 7, clientX: 20, clientY: 30, buttons: 1, isPrimary: true,
  });
  f.window.emit('pointerdown', pointer);
  const payload = f.export();
  const record = payload.events.findLast(item => item.kind === 'pointerdown');
  assert.equal(record.pointer, 'touch');
  assert.equal(record.id, 7);
});

test('causal and geometry rings are independently bounded and report exact drops', () => {
  const f = fixture();
  for (let i = 0; i < 405; i++) f.window.emit('input', event(f.editor));
  for (let i = 0; i < 205; i++) f.window.visualViewport.emit('resize');
  const payload = f.export();
  assert.equal(payload.eventCapacity, 600);
  assert.equal(payload.events.length, 600);
  assert.equal(payload.events.filter(item => item.kind === 'visualViewport.resize').length, 200);
  assert.equal(payload.events.filter(item => item.kind === 'input').length, 399);
  assert.equal(payload.droppedEvents, 8); // attached + ready + 405 inputs + export -400
  assert.equal(payload.droppedGeometry, 5);
  assert.equal(payload.events.at(-1).kind, 'export');
  assert.ok(payload.events.every((item, i, rows) => !i || item.ms >= rows[i - 1].ms));
});

test('unchanged editor styles avoid geometry reads and duplicate records', () => {
  const f = fixture();
  f.mutation();
  const first = f.reads;
  for (let i = 0; i < 1000; i++) f.mutation();
  assert.equal(f.reads, first);
  f.editor.style.top = '40px'; f.mutation();
  assert.ok(f.reads > first);
  const second = f.reads;
  f.mutation(); assert.equal(f.reads, second);
  f.editor.inert = true; f.mutation();
  const records = f.export().events.filter(item => item.kind === 'editor.geometry');
  assert.equal(records.length, 3);
  assert.equal(records[1].style.top, '40px');
  assert.equal(records[2].editorInert, true);
});
