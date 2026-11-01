// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://ui.scarlet-os.org',
  base: '/',
  integrations: [
    starlight({
      title: 'ScarletUI',
      pagefind: false,
      description: 'Cross-platform, declarative UI for Rust.',
      social: [{ icon: 'github', label: 'Source on GitHub', href: 'https://github.com/petitstrawberry/scarlet-ui' }],
      customCss: ['./src/styles/brand.css'],
      sidebar: [
        { label: 'Introduction', slug: '' },
        { label: 'Getting started', slug: 'getting-started' },
        { label: 'Try in your browser', slug: 'try' },
        { label: 'Reference', items: [
          { label: 'Project guide', slug: 'reference/project' },
          { label: 'Architecture', slug: 'reference/architecture' },
          { label: 'Application contract', slug: 'reference/application-contract' },
          { label: 'Browser guide', slug: 'reference/browser-samples' },
        ] },
      ],
    }),
  ],
});
