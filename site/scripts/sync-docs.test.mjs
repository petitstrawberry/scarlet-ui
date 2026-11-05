import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile, mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { importMarkdown, rewriteDestination, sourceBase, assetBase, syncDocs, documents, repositoryRoot } from './sync-docs.mjs';

test('first H1 becomes a quoted frontmatter title, with source attribution', () => {
  const result = importMarkdown('# Title: example\n\nIntro.\n\n# Another heading\n', 'docs/Test.md');
  assert.match(result, /^---\ntitle: "Title: example"\n---/);
  assert.ok(result.includes(`${sourceBase}docs/Test.md`));
  assert.ok(!result.includes('# Title: example'));
  assert.ok(result.includes('# Another heading'));
});
test('inline and reference links resolve against original source location', () => {
  const result = importMarkdown('# Docs\n[Root](../README.md#intro) ![Image](./shot.png "Screenshot")\n[link]: <../samples/README.md> "Guide"\n', 'docs/Test.md');
  assert.ok(result.includes(`[Root](/reference/project/#intro)`));
  assert.ok(result.includes(`![Image](${assetBase}docs/shot.png "Screenshot")`));
  assert.ok(result.includes(`[link]: </reference/browser-samples/> "Guide"`));
});
test('external URLs, local anchors, protocol-relative and mail links remain unchanged', () => {
  for (const url of ['https://example.test/x', '#heading', '//example.test/x', 'mailto:hello@example.test']) {
    assert.equal(rewriteDestination(url, 'README.md'), url);
  }
});
test('fenced examples retain links and H1 text exactly, including long and tilde fences', () => {
  for (const fence of ['```', '````', '~~~']) {
    const block = `${fence}markdown\n# Example\n[do not change](../example.md)\n${fence}\n`;
    const result = importMarkdown(block + '\n# Real title\n[change](docs/ARCHITECTURE.md)\n', 'README.md');
    assert.ok(result.includes(block));
    assert.match(result, /title: "Real title"/);
    assert.ok(result.includes(`/reference/architecture/`));
  }
});
test('inline code remains byte-identical while adjacent Markdown links change', () => {
  const result = importMarkdown('# Docs\n`[keep](local.md)` and ``[also](other.md)`` [change](real.md)\n', 'README.md');
  assert.ok(result.includes('`[keep](local.md)` and ``[also](other.md)``'));
  assert.ok(result.includes(`[change](${sourceBase}real.md)`));
});
test('repository-escaping links fail explicitly', () => {
  assert.throws(() => rewriteDestination('../outside.md', 'README.md'), /escapes the repository/);
});
test('all current repository documents import with complete fenced code retained', async () => {
  for (const document of documents) {
    const source = await readFile(path.join(repositoryRoot, document.source), 'utf8');
    const result = importMarkdown(source, document.source);
    for (const block of source.match(/```[^]*?```/g) ?? []) assert.ok(result.includes(block), document.source);
  }
});
test('sync writes generated pages without modifying repository sources', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'scarlet-docs-'));
  try {
    for (const { source } of documents) {
      await mkdir(path.dirname(path.join(root, source)), { recursive: true });
      await writeFile(path.join(root, source), '# Test\n\nSource stays unchanged.\n');
    }
    await syncDocs(root);
    for (const { source, slug } of documents) {
      assert.equal(await readFile(path.join(root, source), 'utf8'), '# Test\n\nSource stays unchanged.\n');
      assert.match(await readFile(path.join(root, 'site/src/content/docs/reference', `${slug}.md`), 'utf8'), /title: "Test"/);
    }
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('image references retain query fragments and use packaged assets or raw bytes', () => {
  const result = importMarkdown('# Docs\n![shot][screen]\n[screen]: <../site/public/assets/widget-factory.png?raw=1#view> "Preview"\n', 'docs/Test.md');
  assert.ok(result.includes(`</assets/widget-factory.png?raw=1#view>`));
  assert.equal(rewriteDestination('site/public/assets/project-card.png?raw=1', 'README.md'), '/assets/project-card.png?raw=1');
  assert.equal(rewriteDestination('docs/PREVIEW_PROTOCOL.md#protocol-v1', 'README.md'), '/reference/preview-protocol/#protocol-v1');
});
