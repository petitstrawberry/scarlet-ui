import test from 'node:test';
import assert from 'node:assert/strict';
import { createSpecimenController } from '../src/scripts/specimen-controller.mjs';

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function fixture(loadOverride) {
  const calls = [];
  const app = Object.fromEntries(['start_specimen', 'pause', 'resume', 'stop'].map(name => [name, () => { calls.push(name); }]));
  app.runtime_error = () => undefined;
  const controller = createSpecimenController({
    load: loadOverride ?? (() => { calls.push('load'); return app; }),
    onLive: () => calls.push('live'), onFallback: () => calls.push('fallback'),
    setInteractive: value => calls.push(`interactive:${value}`),
    onState: value => calls.push(`state:${value}`),
  });
  return { app, calls, controller, count: name => calls.filter(call => call === name).length };
}

test('lazy, one initialization, repeated visibility is idempotent; pause preserves app', async () => {
  const f = fixture();
  assert.equal(f.controller.status, 'fallback');
  f.controller.setActive(false);
  assert.equal(f.count('load'), 0);
  const first = f.controller.setActive(true);
  assert.equal(f.controller.status, 'loading');
  assert.equal(f.controller.setActive(true), first);
  await first;
  assert.equal(f.controller.status, 'live');
  await f.controller.setActive(true);
  f.controller.setActive(false);
  f.controller.setActive(false);
  assert.equal(f.controller.status, 'paused');
  f.controller.setActive(true);
  assert.equal(f.controller.status, 'live');
  assert.equal(f.count('load'), 1);
  assert.equal(f.count('start_specimen'), 1);
  assert.equal(f.count('pause'), 1);
  assert.equal(f.count('resume'), 2);
  assert.equal(f.count('stop'), 0);
});

test('hidden during load pauses the resolved app without exposing it', async () => {
  const d = deferred(); const f = fixture(() => d.promise);
  const ready = f.controller.setActive(true);
  f.controller.setActive(false);
  d.resolve(f.app); await ready;
  assert.equal(f.controller.status, 'paused');
  assert.equal(f.count('live'), 0);
  assert.equal(f.count('pause'), 1);
  f.controller.setActive(true);
  assert.equal(f.count('start_specimen'), 1);
  assert.equal(f.controller.status, 'live');
});

test('dispose during module load stops late app without starting', async () => {
  const d = deferred(); const f = fixture(() => d.promise);
  const ready = f.controller.setActive(true);
  f.controller.dispose(); d.resolve(f.app); await ready;
  assert.equal(f.controller.status, 'disposed');
  assert.equal(f.count('start_specimen'), 0);
  assert.equal(f.count('stop'), 1);
  f.controller.setActive(true); f.controller.dispose();
  assert.equal(f.count('stop'), 1);
});

test('dispose during asynchronous start stops any late runner', async () => {
  const d = deferred(); const f = fixture();
  f.app.start_specimen = () => d.promise;
  const ready = f.controller.setActive(true);
  await Promise.resolve(); await Promise.resolve();
  f.controller.dispose(); d.resolve(); await ready;
  assert.equal(f.controller.status, 'disposed');
  assert.equal(f.count('stop'), 2);
  assert.equal(f.count('live'), 0);
});

for (const synchronous of [false, true]) test(`loader failure is terminal and handled (sync=${synchronous})`, async () => {
  const f = fixture(() => { if (synchronous) throw Error('load'); return Promise.reject(Error('load')); });
  await f.controller.setActive(true);
  assert.equal(f.controller.status, 'fallback');
  await f.controller.setActive(true);
  assert.equal(f.count('fallback'), 1);
});

test('start failure cleans up and falls back', async () => {
  const f = fixture(); f.app.start_specimen = async () => { throw Error('start'); };
  await f.controller.setActive(true);
  assert.equal(f.controller.status, 'fallback');
  assert.equal(f.count('stop'), 1);
  assert.equal(f.count('fallback'), 1);
});

test('runtime failure stops once, disables interaction and cannot restart', async () => {
  const f = fixture(); await f.controller.setActive(true);
  f.app.runtime_error = () => 'device lost';
  f.controller.checkError(); f.controller.checkError();
  await f.controller.setActive(true);
  assert.equal(f.controller.status, 'fallback');
  assert.equal(f.count('stop'), 1);
  assert.equal(f.count('fallback'), 1);
  assert.equal(f.calls.at(-3), 'interactive:false');
});

test('disposed loader rejection is swallowed without fallback callback', async () => {
  const d = deferred(); const f = fixture(() => d.promise);
  const ready = f.controller.setActive(true); f.controller.dispose();
  d.reject(Error('late')); await ready;
  assert.equal(f.controller.status, 'disposed');
  assert.equal(f.count('fallback'), 0);
});

test('visibility changes during asynchronous start settle to latest desired state', async () => {
  const d = deferred(); const f = fixture();
  f.app.start_specimen = () => d.promise;
  const ready = f.controller.setActive(true);
  await Promise.resolve(); await Promise.resolve();
  f.controller.setActive(false); f.controller.setActive(true); f.controller.setActive(false);
  d.resolve(); await ready;
  assert.equal(f.controller.status, 'paused');
  assert.equal(f.count('pause'), 1);
  assert.equal(f.count('live'), 0);
});

test('dispose of live app is idempotent and terminal', async () => {
  const f = fixture(); await f.controller.setActive(true);
  f.controller.dispose(); f.controller.dispose(); f.controller.checkError();
  await f.controller.setActive(true);
  assert.equal(f.controller.status, 'disposed');
  assert.equal(f.count('stop'), 1);
  assert.equal(f.count('load'), 1);
});

test('exceptions from runtime error polling are handled as failures', async () => {
  const f = fixture(); await f.controller.setActive(true);
  f.app.runtime_error = () => { throw Error('runtime'); };
  f.controller.checkError();
  assert.equal(f.controller.status, 'fallback');
  assert.equal(f.count('stop'), 1);
});

test('separate embedded modules pause, fail and dispose independently', async () => {
  const hero = fixture();
  const counter = fixture();
  await Promise.all([hero.controller.setActive(true), counter.controller.setActive(true)]);
  hero.controller.setActive(false);
  assert.equal(hero.controller.status, 'paused');
  assert.equal(counter.controller.status, 'live');
  assert.equal(counter.count('pause'), 0);
  hero.app.runtime_error = () => 'device lost';
  hero.controller.checkError();
  assert.equal(hero.controller.status, 'fallback');
  assert.equal(hero.count('stop'), 1);
  assert.equal(counter.count('stop'), 0);
  counter.controller.setActive(false);
  counter.controller.setActive(true);
  assert.equal(counter.count('start_specimen'), 1);
  counter.controller.dispose();
  assert.equal(counter.count('stop'), 1);
  assert.equal(hero.count('stop'), 1);
});
