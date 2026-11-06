/** Static sharing artwork. Uses the repository font and real UI captures only. */
import { mkdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const { default: sharp } = await import('sharp');

const fontfile = fileURLToPath(new URL('../../examples/web-counter/assets/DejaVuSans.ttf', import.meta.url));
const boldfile = fileURLToPath(new URL('../assets/fonts/DejaVuSans-Bold.ttf', import.meta.url));
const escape = text => text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
const controls = new URL('../assets/sharing/', import.meta.url);
async function controlArtwork() {
  const parts = [
    ['tabs.png', 20, 45], ['button.png', 20, 125],
    ['toggle.png', 240, 130], ['slider.png', 20, 205],
  ];
  const layers = [];
  for (const [name, left, top] of parts) {
    const source = fileURLToPath(new URL(name, controls));
    const { width } = await sharp(source).metadata();
    // Native 2× captures are reduced to 1.6× logical size for crisp sharing
    // previews; wide controls fit within 360 pixels. No controls are redrawn or relabeled.
    const input = await sharp(source).resize(Math.round(Math.min(width * 0.8, 360)), null, { kernel: 'lanczos3' }).png().toBuffer();
    layers.push({ input, left, top });
  }
  return sharp({ create: { width: 420, height: 280, channels: 3, background: '#fff' } })
    .composite(layers).removeAlpha().png().toBuffer();
}

const colors = { ink: '#202024', muted: '#55545a', scarlet: '#bd2034' };

async function lettering(text, width, size, maxHeight, color, bold = false) {
  for (let px = size; px >= 24; px -= 2) {
    const input = await sharp({ text: {
      text: `<span foreground="${color}">${escape(text)}</span>`,
      font: `DejaVu Sans ${bold ? 'Bold ' : ''}${px}`, fontfile: bold ? boldfile : fontfile, width, wrap: 'word',
      spacing: Math.round(px * 0.18), rgba: true, dpi: 72,
    } }).png().toBuffer();
    const { height, width: renderedWidth } = await sharp(input).metadata();
    if (height <= maxHeight && renderedWidth <= width) return { input, height };
  }
  throw new Error(`Sharing image text does not fit: ${text}`);
}

export async function renderSocialImage({ title, subtitle, imagePath, output, homepage = false }) {
  if (!title || !subtitle || !output) throw new Error('title, subtitle and output are required');
  const root = homepage;
  // The separate brand line already supplies this prefix on document cards.
  const displayTitle = root ? title : title.replace(/^ScarletUI\s+/, '');
  const artwork = root ? await controlArtwork() : imagePath ? await sharp(imagePath).resize(400, 300, { fit: 'inside' }).png().toBuffer() : undefined;
  const width = artwork ? 560 : 1040;
  const layers = [];
  if (!root) layers.push({ ...(await lettering('ScarletUI', 1040, 32, 50, colors.scarlet)), left: 80, top: 76 });
  const heading = await lettering(displayTitle, width, root ? 92 : 72, artwork ? 180 : 250, root ? colors.scarlet : colors.ink, true);
  const top = root ? 220 : 190;
  layers.push({ input: heading.input, left: 80, top });
  const definition = await lettering(root ? 'Cross-platform, declarative\nUI for Rust.' : subtitle, width, 30, 550 - (top + heading.height + 32), colors.muted);
  layers.push({ input: definition.input, left: 80, top: top + heading.height + 32 });
  if (artwork) {
    const capture = artwork;
    const dimensions = await sharp(capture).metadata();
    layers.push({ input: capture, left: 700 + Math.round((420 - dimensions.width) / 2), top: Math.round((630 - dimensions.height) / 2) });
  }
  await mkdir(path.dirname(output), { recursive: true });
  await sharp({ create: { width: 1200, height: 630, channels: 3, background: '#ffffff' } })
    .composite(layers.map(({ input, left, top }) => ({ input, left, top })))
    .removeAlpha().png({ compressionLevel: 9 }).toFile(output);
}
