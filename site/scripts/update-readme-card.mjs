/** Keep the repository-visible README thumbnail in sync with site sharing artwork. */
import { fileURLToPath } from 'node:url';
import { renderSocialImage } from './social-image.mjs';

await renderSocialImage({
  title: 'ScarletUI',
  subtitle: 'Cross-platform, declarative UI for Rust.',
  homepage: true,
  output: fileURLToPath(new URL('../public/assets/project-card.png', import.meta.url)),
});
