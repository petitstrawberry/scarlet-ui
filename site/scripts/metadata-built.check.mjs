// Run after `npm run build`; verify actual Starlight output, not frontmatter alone.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';
const dist = process.env.SCARLET_SITE_DIR ? pathToFileURL(resolve(process.env.SCARLET_SITE_DIR) + '/') : new URL('../dist/', import.meta.url);
const html = readFileSync(new URL('index.html', dist), 'utf8');
const head = html.match(/<head\b[^>]*>([\s\S]*?)<\/head>/i)[1];
const title = 'ScarletUI — Cross-platform UI for Rust';
const description = 'A declarative UI toolkit for Rust. Build applications for Linux, macOS, Windows, Scarlet, and the web.';
const image = 'https://ui.scarlet-os.org/assets/social-card.png';
const alt = 'ScarletUI — Cross-platform, declarative UI for Rust, with standard tabs, button, toggle, and slider.';
function attributes(tag) {
  return Object.fromEntries([...tag.matchAll(/([\w:-]+)="([^"]*)"/g)].map(([, key, value]) => [key, value]));
}
function meta(key, expected) {
  const tags = [...head.matchAll(/<meta\b[^>]*>/gi)].map(([tag]) => attributes(tag));
  const matches = tags.filter(tag => tag.name === key || tag.property === key);
  assert.equal(matches.length, 1, `${key} must occur exactly once`);
  assert.equal(matches[0].content, expected, key);
}

test('homepage title overrides metadata only, with no duplicate social tags', () => {
  assert.deepEqual([...head.matchAll(/<title>(.*?)<\/title>/g)].map(match => match[1]), [title]);
  assert.match(html, /<h1\b[^>]*>ScarletUI<\/h1>/);
  meta('description', description); meta('og:description', description);
  meta('og:title', title); meta('og:type', 'website');
  meta('twitter:title', title); meta('twitter:description', description);
  meta('twitter:card', 'summary_large_image');
});

test('homepage retains exactly one production root canonical and og:url', () => {
  const links = [...head.matchAll(/<link\b[^>]*>/gi)].map(([tag]) => attributes(tag));
  assert.deepEqual(links.filter(link => link.rel === 'canonical').map(link => link.href), ['https://ui.scarlet-os.org/']);
  meta('og:url', 'https://ui.scarlet-os.org/');
});

test('social image metadata matches a real 1200×630 PNG in the generated artifact', () => {
  meta('og:image', image); meta('twitter:image', image);
  meta('og:image:width', '1200'); meta('og:image:height', '630');
  meta('og:image:type', 'image/png'); meta('og:image:alt', alt); meta('twitter:image:alt', alt);
  const png = readFileSync(new URL('assets/social-card.png', dist));
  assert.deepEqual([...png.subarray(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10]);
  assert.equal(png.toString('ascii', 12, 16), 'IHDR');
  assert.equal(png.readUInt32BE(16), 1200); assert.equal(png.readUInt32BE(20), 630);
});

test('documentation titles do not inherit the homepage title or image', () => {
  const docs = readFileSync(new URL('getting-started/index.html', dist), 'utf8');
  assert.deepEqual([...docs.matchAll(/<title>(.*?)<\/title>/g)].map(match => match[1]), ['Getting started | ScarletUI']);
  assert.ok(!docs.includes(image));
});

test('each indexable generated page has its own unique social image and truthful title', async () => {
  const { readdirSync } = await import('node:fs');
  const pages = readdirSync(dist, { recursive: true }).filter(path => path.endsWith('.html') && path !== '404.html');
  const images = new Set();
  for (const path of pages) {
    const source = readFileSync(new URL(path, dist), 'utf8');
    const metas = [...source.matchAll(/<meta\b[^>]*>/gi)].map(([tag]) => attributes(tag));
    if (metas.some(tag => tag.name === 'robots' && tag.content?.includes('noindex'))) continue;
    const only = key => {
      const matches = metas.filter(tag => tag.property === key || tag.name === key);
      assert.equal(matches.length, 1, `${path}: ${key}`);
      return matches[0].content;
    };
    const title = source.match(/<title>(.*?)<\/title>/)[1];
    assert.equal(only('og:title'), title);
    assert.equal(only('twitter:title'), title);
    const url = only('og:image');
    assert.equal(only('twitter:image'), url);
    assert.ok(!images.has(url), `${path} must have its own social card`);
    images.add(url);
    const png = readFileSync(new URL(new URL(url).pathname.slice(1), dist));
    assert.equal(png.readUInt32BE(16), 1200); assert.equal(png.readUInt32BE(20), 630);
    assert.equal(only('og:image:width'), '1200'); assert.equal(only('og:image:height'), '630');
  }
  assert.ok(pages.length >= 7);
});
