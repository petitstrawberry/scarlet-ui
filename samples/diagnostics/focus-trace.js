// QA only. No interception, default prevention, typed content, or network upload.
(() => {
  'use strict';
  const capacity = 600;
  const events = [];
  const geometryEvents = [];
  let droppedEvents = 0;
  let droppedGeometry = 0;
  const started = performance.now();
  const identity = JSON.parse(document.querySelector('#scarlet-diagnostic-build').textContent);
  const describe = node => {
    if (node === window) return 'window';
    if (node === document) return 'document';
    if (!(node instanceof Element)) return null;
    if (node.matches('[data-scarlet-text-input]')) return 'scarlet-textarea';
    if (node.closest('[data-focus-trace-ui]')) return 'diagnostic-control';
    return node.tagName.toLowerCase() + (node.id === 'ui' ? '#ui' : '');
  };
  const rect = node => {
    if (!(node instanceof Element)) return null;
    const r = node.getBoundingClientRect();
    return { x:r.x, y:r.y, width:r.width, height:r.height };
  };
  const geometry = () => {
    const v = window.visualViewport;
    const canvas = document.querySelector('canvas');
    const editor = document.querySelector('[data-scarlet-text-input]');
    return {
      active:describe(document.activeElement), focused:document.hasFocus(), visibility:document.visibilityState,
      viewport:v ? { width:v.width, height:v.height, x:v.offsetLeft, y:v.offsetTop, scale:v.scale } : null,
      window:{ width:innerWidth, height:innerHeight, scrollX, scrollY, dpr:devicePixelRatio },
      canvas:rect(canvas), editor:rect(editor), editorInert:editor?.inert ?? null
    };
  };
  const record = (kind, detail = {}) => {
    const geometryOnly = /^(editor\.geometry|window\.(scroll|resize)|visualViewport\.)/.test(kind);
    const queue = geometryOnly ? geometryEvents : events;
    queue.push({ ms:Math.round((performance.now()-started)*10)/10, kind, ...detail });
    if (queue.length > (geometryOnly ? 200 : 400)) {
      queue.shift();
      if (geometryOnly) droppedGeometry++; else droppedEvents++;
    }
  };
  function observe(event) {
    const detail = { target:describe(event.target), related:describe(event.relatedTarget), trusted:event.isTrusted,
      ...geometry() };
    if (event instanceof PointerEvent) Object.assign(detail, { pointer:event.pointerType, id:event.pointerId,
      x:event.clientX, y:event.clientY, buttons:event.buttons, primary:event.isPrimary });
    if (event instanceof MouseEvent && !(event instanceof PointerEvent)) Object.assign(detail, { x:event.clientX, y:event.clientY, button:event.button });
    if (event instanceof InputEvent) Object.assign(detail, { inputType:event.inputType, composing:event.isComposing });
    // A synchronous programmatic focus dispatch retains its JS/Wasm caller stack.
    // Native-delivered focus can have only this listener; absence isn't proof.
    if (/^(focus|blur)/.test(event.type)) detail.stack = String(new Error().stack || '').slice(0,1800);
    record(event.type, detail);
  }
  const kinds = ['pointerdown','pointerup','pointercancel','gotpointercapture','lostpointercapture',
    'mousedown','mouseup','click','focus','blur','focusin','focusout',
    'beforeinput','input','compositionstart','compositionend','visibilitychange','pagehide','pageshow'];
  for (const kind of kinds) window.addEventListener(kind, observe, { capture:true, passive:true });
  for (const kind of ['resize','scroll']) {
    window.addEventListener(kind, () => record('window.'+kind, geometry()), { passive:true });
    window.visualViewport?.addEventListener(kind, () => record('visualViewport.'+kind, geometry()), { passive:true });
  }
  let editor = null;
  let lastEditorGeometry = null;
  const editorObserver = new MutationObserver(() => {
    if (!editor) return;
    const style = {left:editor.style.left,top:editor.style.top,height:editor.style.height,inert:editor.inert};
    const signature = JSON.stringify(style);
    if (signature !== lastEditorGeometry) {
      lastEditorGeometry = signature;
      record('editor.geometry', { ...geometry(), style });
    }
  });
  const attachEditor = () => {
    const next = document.querySelector('[data-scarlet-text-input]');
    if (next === editor) return;
    editorObserver.disconnect(); editor = next;
    record('editor.attached', geometry());
    if (editor) editorObserver.observe(editor, { attributes:true, attributeFilter:['style','inert'] });
  };
  new MutationObserver(attachEditor).observe(document.documentElement, {childList:true, subtree:true});
  attachEditor();
  const ui = document.createElement('details');
  ui.dataset.focusTraceUi = '';
  ui.style.cssText = 'position:fixed;right:6px;top:3px;z-index:2147483647;font:12px/1.4 system-ui;color:#222;background:#fff;border:1px solid #bbb;border-radius:4px;max-width:280px;padding:3px 7px;';
  const summary = document.createElement('summary'); summary.textContent = '診断ログ'; ui.append(summary);
  const note = document.createElement('p'); note.textContent = '入力欄を試し、症状が出た後にログを保存してください。入力した文字は記録しません。'; ui.append(note);
  const save = document.createElement('button'); save.textContent = 'ログをダウンロード'; save.type = 'button';
  save.addEventListener('click', () => {
    record('export', geometry());
    const payload = { ...identity, generatedAt:new Date().toISOString(), userAgent:navigator.userAgent,
      secureContext:isSecureContext, eventCapacity:capacity, droppedEvents, droppedGeometry,
      events:[...events,...geometryEvents].sort((a,b)=>a.ms-b.ms) };
    const url = URL.createObjectURL(new Blob([JSON.stringify(payload,null,2)],{type:'application/json'}));
    const link = document.createElement('a'); link.href=url; link.download='scarlet-focus-diagnostic.json'; link.click();
    setTimeout(()=>URL.revokeObjectURL(url),60000);
  }); ui.append(save);
  document.body.append(ui);
  record('ready', geometry());
})();
