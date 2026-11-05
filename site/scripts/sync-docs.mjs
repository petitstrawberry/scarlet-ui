/** Import repository-owned documentation for Starlight without maintaining copies. */
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
export const repositoryRoot = path.resolve(here, '../..');
export const sourceBase = 'https://github.com/petitstrawberry/scarlet-ui/blob/main/';
export const assetBase = 'https://raw.githubusercontent.com/petitstrawberry/scarlet-ui/main/';
export const documents = [
  { source: 'README.md', slug: 'project' },
  { source: 'docs/ARCHITECTURE.md', slug: 'architecture' },
  { source: 'docs/1.0-contract.md', slug: 'application-contract' },
  { source: 'samples/README.md', slug: 'browser-samples' },
  { source: 'docs/PREVIEW_PROTOCOL.md', slug: 'preview-protocol' },
  { source: 'docs/APPLICATION_GUIDE.md', slug: 'application-guide' },
  { source: 'docs/PLATFORM_GUIDE.md', slug: 'platform-guide' },
  { source: 'docs/LIVE_PREVIEW.md', slug: 'live-preview' },
  { source: 'docs/DEVELOPMENT.md', slug: 'development' },
  { source: 'docs/README.md', slug: 'documentation' },
];

export function rewriteDestination(destination, source) {
  const angled = destination.startsWith('<') && destination.endsWith('>');
  const value = angled ? destination.slice(1, -1) : destination;
  if (!value || /^(?:[a-z][a-z0-9+.-]*:|\/\/|#)/i.test(value)) return destination;
  const suffixAt = value.search(/[?#]/);
  const pathname = suffixAt < 0 ? value : value.slice(0, suffixAt);
  const suffix = suffixAt < 0 ? '' : value.slice(suffixAt);
  const resolved = path.posix.normalize(pathname.startsWith('/')
    ? pathname.slice(1)
    : path.posix.join(path.posix.dirname(source), pathname));
  if (resolved === '..' || resolved.startsWith('../')) {
    throw new Error(`Documentation link escapes the repository: ${source}: ${value}`);
  }
  // Images must resolve to bytes, never GitHub's HTML blob viewer. This also
  // covers reference-style image definitions and linked full-size images.
  const isImage = /\.(?:png|jpe?g|gif|webp|avif|svg)$/i.test(resolved);
  const document = documents.find((entry) => entry.source === resolved);
  const publicAsset = isImage && resolved.startsWith('site/public/');
  const rewritten = publicAsset ? '/' + resolved.slice('site/public/'.length) + suffix
    : document ? `/reference/${document.slug}/${suffix}`
    : (isImage ? assetBase : sourceBase) + resolved + suffix;
  return angled ? `<${rewritten}>` : rewritten;
}

function rewriteProse(text, source) {
  // Preserve labels and optional link titles. Current imported files use standard
  // inline/reference links, not raw HTML links or Markdown extensions.
  return text.replace(/(!?\[[^\]\n]*\]\(\s*)(<[^>\n]*>|[^\s)\n]+)([^)\n]*\))/g,
    (_, start, destination, end) => start + rewriteDestination(destination, source) + end)
    .replace(/^( {0,3}\[[^\]\n]+\]:\s*)(<[^>\n]*>|[^\s]+)(.*)$/gm,
      (_, start, destination, end) => start + rewriteDestination(destination, source) + end);
}

function rewriteOutsideInlineCode(text, source) {
  let result = '';
  let offset = 0;
  const code = /(`+)([^]*?)\1/g;
  // Code spans in the source docs use matching backtick runs. Keep them byte-for-byte.
  for (const match of text.matchAll(code)) {
    result += rewriteProse(text.slice(offset, match.index), source) + match[0];
    offset = match.index + match[0].length;
  }
  return result + rewriteProse(text.slice(offset), source);
}

export function importMarkdown(markdown, source) {
  let title;
  let fence;
  let prose = '';
  let body = '';
  const flush = () => { body += rewriteOutsideInlineCode(prose, source); prose = ''; };
  for (const line of markdown.split(/(?<=\n)/)) {
    const marker = line.match(/^ {0,3}(`{3,}|~{3,})(.*)/);
    if (fence) {
      body += line;
      if (marker && marker[1][0] === fence.character && marker[1].length >= fence.length && !marker[2].trim()) fence = null;
    } else if (marker) {
      flush();
      fence = { character: marker[1][0], length: marker[1].length };
      body += line;
    } else if (!title && /^#\s+/.test(line)) {
      title = line.replace(/^#\s+/, '').replace(/\s+#+\s*$/, '').trim();
    } else {
      prose += line;
    }
  }
  flush();
  if (!title) throw new Error(`Imported document has no H1 title: ${source}`);
  return `---\ntitle: ${JSON.stringify(title)}\n---\n\n` +
    `This page is generated from the repository's [\`${source}\`](${sourceBase + source}). ` +
    'Edit that source to update this reference.\n\n' + body.replace(/^\s*\n/, '');
}

export async function syncDocs(root = repositoryRoot) {
  const directory = path.join(root, 'site/src/content/docs/reference');
  await mkdir(directory, { recursive: true });
  for (const { source, slug } of documents) {
    const markdown = await readFile(path.join(root, source), 'utf8');
    await writeFile(path.join(directory, `${slug}.md`), importMarkdown(markdown, source));
  }
  console.log(`Imported ${documents.length} repository reference pages.`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await syncDocs();
}
