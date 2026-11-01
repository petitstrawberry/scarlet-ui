import { mkdir } from 'node:fs/promises';
import { dirname } from 'node:path';
import { renderSocialImage } from './social-image.mjs';

let input = '';
for await (const chunk of process.stdin) input += chunk;
for (const card of JSON.parse(input)) {
  await mkdir(dirname(card.output), { recursive: true });
  await renderSocialImage({ ...card, imagePath: card.imagePath ?? undefined });
}
