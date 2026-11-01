import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import sharp from 'sharp';
import { renderSocialImage } from './social-image.mjs';

test('sharing artwork is a reproducible 1200 by 630 opaque PNG', async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'scarlet-social-'));
  try {
    const options = { title: 'Architecture', subtitle: 'Views, state, and rendering.' };
    const first = path.join(directory, 'first.png');
    const second = path.join(directory, 'second.png');
    await renderSocialImage({ ...options, output: first });
    await renderSocialImage({ ...options, output: second });
    const metadata = await sharp(first).metadata();
    assert.equal(metadata.format, 'png');
    assert.equal(metadata.width, 1200);
    assert.equal(metadata.height, 630);
    assert.equal(metadata.hasAlpha, false);
    assert.deepEqual(await readFile(first), await readFile(second));
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test('source markup is escaped and long document titles fit without clipping', async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'scarlet-social-'));
  try {
    const output = path.join(directory, 'long.png');
    await renderSocialImage({ title: 'ScarletUI Application Contract and Retained State', subtitle: 'State<T> & view updates use the same renderer.', output });
    assert.equal((await sharp(output).metadata()).width, 1200);
  } finally { await rm(directory, { recursive: true, force: true }); }
});
