import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('social_metadata', Path(__file__).with_name('social-metadata.py'))
metadata = importlib.util.module_from_spec(spec)
spec.loader.exec_module(metadata)


class MetadataTests(unittest.TestCase):
    def test_generated_routes_derive_titles_and_escape_content_idempotently(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'guide').mkdir()
            page = root / 'guide/index.html'
            page.write_text('<html><head><title>A &amp; B | ScarletUI</title><meta name="description" content="Use &quot;widgets&quot;"><meta property="og:type" content="article"><meta property="og:site_name" content="ScarletUI"><meta property="og:locale" content="en_US"><meta property="og:title" content="Old"><meta property="og:title" content="Duplicate"></head><body>Content</body></html>')
            cards = metadata.process(root)
            first = page.read_text()
            self.assertEqual(cards[0]['title'], 'A & B')
            self.assertEqual(cards[0]['subtitle'], 'Use "widgets"')
            self.assertEqual(first.count('property="og:title"'), 1)
            self.assertIn('content="A &amp; B | ScarletUI"', first)
            self.assertIn('https://ui.scarlet-os.org/guide/', first)
            self.assertIn('https://ui.scarlet-os.org/assets/social/guide/index.png', first)
            self.assertIn('<body>Content</body>', first)
            parsed = metadata.Page(first)
            self.assertEqual(parsed.meta['og:type'], 'article')
            self.assertEqual(parsed.meta['og:site_name'], 'ScarletUI')
            self.assertEqual(parsed.meta['og:locale'], 'en_US')
            metadata.process(root)
            self.assertEqual(page.read_text(), first)

    def test_implicit_head_samples_and_noindex_pages(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'counter').mkdir()
            (root / 'counter/index.html').write_text('<!doctype html><html><title>Counter</title><canvas></canvas>')
            skipped = '<head><title>Private</title><meta name="robots" content="noindex,nofollow"></head>'
            (root / 'private.html').write_text(skipped)
            (root / '404.html').write_text('<title>Not found</title>')
            cards = metadata.process(root)
            self.assertEqual(len(cards), 1)
            self.assertEqual(cards[0]['subtitle'], 'Counter')
            self.assertIsNone(cards[0]['imagePath'])
            self.assertEqual(metadata.Page((root / 'counter/index.html').read_text()).meta['og:type'], 'website')
            self.assertEqual((root / 'private.html').read_text(), skipped)
            self.assertEqual((root / '404.html').read_text(), '<title>Not found</title>')
            self.assertLess((root / 'counter/index.html').read_text().index('<meta'), (root / 'counter/index.html').read_text().index('<canvas'))

    def test_generic_description_uses_bounded_source_paragraph(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'guide.html').write_text('<head><title>Guide | ScarletUI</title><meta name="description" content="Cross-platform, declarative UI for Rust."></head><h1>Guide</h1><div class="sl-markdown-content"><p>This page is generated from the repository.</p><p>Build views with Rust. More details follow.</p></div>')
            cards = metadata.process(root)
            self.assertEqual(cards[0]['title'], 'Guide')
            self.assertEqual(cards[0]['subtitle'], 'Build views with Rust.')
            self.assertIn('content="Build views with Rust."', (root / 'guide.html').read_text())
            self.assertLessEqual(len(metadata.excerpt('long ' * 100)), 200)

    def test_root_preserves_explicit_canonical_and_alt(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'index.html').write_text('<head><title>ScarletUI</title><link rel="canonical" href="https://ui.scarlet-os.org/"><meta property="og:image:alt" content="Real UI"></head>')
            cards = metadata.process(root)
            self.assertTrue(cards[0]['output'].endswith('/assets/social-card.png'))
            self.assertTrue(cards[0]['homepage'])
            self.assertIsNone(cards[0]['imagePath'])
            output = (root / 'index.html').read_text()
            self.assertEqual(output.count('rel="canonical"'), 1)
            self.assertIn('content="Real UI"', output)


if __name__ == '__main__':
    unittest.main()
