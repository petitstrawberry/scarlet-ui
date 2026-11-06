// Node's built-in test runner; no installed dependencies or browser flags.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync(`${__dirname}/capability.js`, 'utf8');

function setup({ secure = true, gpu } = {}) {
  const status = { textContent: '' };
  const button = { disabled: false, addEventListener(type, fn) { this.click = fn; } };
  vm.runInNewContext(source, {
    isSecureContext: secure,
    navigator: { gpu },
    document: { querySelector: selector => selector === '#gpu-status' ? status : button },
  });
  return { status, button };
}
test('insecure context gives a reason without requesting an adapter', async () => {
  const page = setup({ secure: false, gpu: { requestAdapter() { throw Error('must not call'); } } });
  await page.button.click();
  assert.match(page.status.textContent, /not a secure context/);
  assert.equal(page.button.disabled, false);
});
test('missing API is distinguished from a missing adapter', async () => {
  const missing = setup();
  await missing.button.click();
  assert.match(missing.status.textContent, /not exposed/);
  const noAdapter = setup({ gpu: { requestAdapter: async () => null } });
  await noAdapter.button.click();
  assert.match(noAdapter.status.textContent, /no adapter/);
});
test('available adapter is checked only on request and does not request a device', async () => {
  let calls = 0;
  const page = setup({ gpu: { requestAdapter: async () => { calls++; return {}; } } });
  assert.equal(calls, 0);
  await page.button.click();
  assert.equal(calls, 1);
  assert.match(page.status.textContent, /adapter is available/);
});
test('rejected adapter check restores the button and displays text safely', async () => {
  const page = setup({ gpu: { requestAdapter: async () => { throw '<test error>'; } } });
  await page.button.click();
  assert.match(page.status.textContent, /could not complete.*<test error>/);
  assert.equal(page.button.disabled, false);
});
