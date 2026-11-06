import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { extractCounterControls, loadCounterControls } from './counter-snippet.mjs';

const start = '// docs:counter-controls:start';
const end = '// docs:counter-controls:end';
test('extracts the marked compiled method, preserving its code and relative indentation', () => {
  const source = `ignored\n    ${start}\n    fn controls(&self) {\n        count.set(count.get() + 1);\n    }\n    ${end}\nignored`;
  assert.equal(extractCounterControls(source), 'fn controls(&self) {\n    count.set(count.get() + 1);\n}');
});
test('accepts CRLF and trims blank edge lines without replacing code', () => {
  assert.equal(extractCounterControls(`  ${start}\r\n\r\n  first\r\n    second\r\n\r\n  ${end}`), 'first\n  second');
});
test('fails closed for missing, repeated, reversed or empty markers', () => {
  for (const source of ['unmarked', `${end}\ncode\n${start}`, `${start}\ncode\n${start}\n${end}`, `${start}\n${end}`, `${start}\ncode\n${end}\n${end}`]) {
    assert.throws(() => extractCounterControls(source), /marker|empty/);
  }
});
test('homepage snippet is the compact compiled view body without method plumbing', async () => {
  const source = await readFile(new URL('../../examples/web-counter/src/lib.rs', import.meta.url), 'utf8');
  const snippet = await loadCounterControls();
  assert.ok(source.includes('count: State<i32>'));
  assert.match(source, /fn controls\(&self\)/);
  assert.ok(snippet.startsWith('vstack! {'));
  assert.match(snippet, /Text::new\(format!\(/);
  assert.match(snippet, /"Count: \{\}", count\.get\(\)/);
  assert.match(snippet, /vstack! \{/);
  assert.ok(!snippet.includes('impl View'));
  assert.ok(!snippet.includes('self.'));
  assert.ok(snippet.split('\n').every(line => line.length <= 36));
  assert.match(snippet, /Text::new/);
  assert.match(snippet, /Button::new/);
  assert.match(snippet, /\.on_click/);
  assert.match(snippet, /count\.update\(\|n\|/);
  assert.match(snippet, /\*n \+= 1/);
  assert.ok(!snippet.includes('wasm_bindgen'));
});
