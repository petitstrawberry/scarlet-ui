/** Extract the homepage example from the Rust source compiled into its live result. */
import { readFile } from 'node:fs/promises';

const sourceUrl = new URL('../../examples/web-counter/src/lib.rs', import.meta.url);
const startMarker = '// docs:counter-controls:start';
const endMarker = '// docs:counter-controls:end';

export function extractCounterControls(source) {
  const lines = source.split(/\r?\n/);
  const starts = lines.flatMap((line, index) => line.trim() === startMarker ? [index] : []);
  const ends = lines.flatMap((line, index) => line.trim() === endMarker ? [index] : []);
  if (starts.length !== 1 || ends.length !== 1 || ends[0] <= starts[0]) {
    throw new Error('Counter source must contain exactly one ordered controls snippet marker pair.');
  }
  const selected = lines.slice(starts[0] + 1, ends[0]);
  while (selected[0]?.trim() === '') selected.shift();
  while (selected.at(-1)?.trim() === '') selected.pop();
  if (!selected.length) throw new Error('The counter controls snippet is empty.');
  const indentation = Math.min(...selected.filter(line => line.trim()).map(line => line.match(/^\s*/)[0].length));
  return selected.map(line => line.slice(indentation)).join('\n');
}

export async function loadCounterControls() {
  return extractCounterControls(await readFile(sourceUrl, 'utf8'));
}
