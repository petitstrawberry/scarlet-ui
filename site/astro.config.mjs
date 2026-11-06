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
      components: { Footer: './src/components/Footer.astro' },
      sidebar: [
        { label: 'Introduction', slug: '' },
        { label: 'Getting started', slug: 'getting-started' },
        { label: 'Try in your browser', slug: 'try' },
        { label: 'Guides', items: [
          { label: 'Project overview', slug: 'reference/project' },
          { label: 'Documentation index', slug: 'reference/documentation' },
          { label: 'Application guide', slug: 'reference/application-guide' },
          { label: 'Platforms and backends', slug: 'reference/platform-guide' },
          { label: 'Live preview', slug: 'reference/live-preview' },
          { label: 'Development', slug: 'reference/development' },
          { label: 'Architecture', slug: 'reference/architecture' },
          { label: 'Application contract', slug: 'reference/application-contract' },
          { label: 'Browser guide', slug: 'reference/browser-samples' },
          { label: 'Preview server protocol', slug: 'reference/preview-protocol' },
        ] },
        { label: 'API Reference', link: '/docs/' },
      ],
    }),
  ],
});
