"""Static package validation regressions (no browser, compiler or network)."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from urllib.parse import urljoin, urlsplit

spec = importlib.util.spec_from_file_location('build_web_samples', Path(__file__).with_name('build-web-samples.py'))
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


class PackageValidation(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        for name in ('capability.js', '.nojekyll', 'font-license.txt', 'tabler-license.txt'):
            (self.root / name).touch()
        (self.root / '_astro').mkdir()
        (self.root / '_astro/theme.css').touch()
        (self.root / 'guide').mkdir()
        (self.root / 'guide/index.html').write_text('<a href="/">ScarletUI</a>')
        (self.root / 'assets').mkdir()
        (self.root / 'assets/widget-factory.png').touch()
        (self.root / 'assets/counter-example.png').touch()
        (self.root / 'try').mkdir()
        (self.root / 'index.html').write_text('<a href="/try/">Try</a><a href="/guide/">Guide</a><link href="/_astro/theme.css"><img src="./assets/widget-factory.png">')
        (self.root / 'try/index.html').write_text('<a href="../">Home</a><a href="../counter/">Counter</a><a href="../widget-factory/">Factory</a><link href="/_astro/theme.css"><script src="../capability.js"></script>')
        for route, _, module in builder.DEMOS:
            folder = self.root / route
            (folder / 'pkg').mkdir(parents=True)
            (folder / 'index.html').write_text(f"<script>import('./pkg/{module}.js')</script>")
            (folder / 'pkg' / f'{module}.js').write_text(f"new URL('{module}_bg.wasm', import.meta.url)")
            (folder / 'pkg' / f'{module}_bg.wasm').write_bytes(b'\0asm\x01\0\0\0')

    def tearDown(self):
        self.temp.cleanup()

    def test_source_archive_has_unknown_git_identity(self):
        self.assertEqual(builder.source_identity(self.root), {"source_commit": None, "source_dirty": None})

    def test_generated_starlight_routes_resolve_at_custom_domain_root(self):
        builder.validate(self.root)

    def test_nested_assets_resolve_at_root_and_project_prefix(self):
        for base in ('https://ui.scarlet-os.org/', 'https://example.test/scarlet-ui/'):
            for route, _, module in builder.DEMOS:
                page = urljoin(base, f'./{route}/')
                glue = urljoin(page, f'./pkg/{module}.js')
                wasm = urljoin(glue, f'{module}_bg.wasm')
                with self.subTest(base=base, route=route):
                    self.assertEqual(page, f'{base}{route}/')
                    self.assertEqual(glue, f'{base}{route}/pkg/{module}.js')
                    self.assertEqual(wasm, f'{base}{route}/pkg/{module}_bg.wasm')
                    for url in (page + 'index.html', glue, wasm):
                        relative = urlsplit(url).path.removeprefix(urlsplit(base).path)
                        self.assertTrue((self.root / relative).is_file())

    def test_homepage_and_try_links_resolve_at_custom_domain_root(self):
        for base in ('https://ui.scarlet-os.org/',):
            for page_path in ('index.html', 'try/index.html'):
                references = builder.References()
                references.feed((self.root / page_path).read_text())
                for reference in references.urls:
                    url = urljoin(urljoin(base, page_path), reference)
                    with self.subTest(base=base, page=page_path, reference=reference):
                        self.assertTrue(url.startswith(base))
                        relative = urlsplit(url).path.removeprefix(urlsplit(base).path)
                        target = self.root / relative
                        if target.is_dir():
                            target /= 'index.html'
                        self.assertTrue(target.is_file())

    def test_missing_try_page_is_rejected(self):
        (self.root / 'try/index.html').unlink()
        with self.assertRaisesRegex(ValueError, 'Missing local link|Missing site asset'):
            builder.validate(self.root)

    def test_search_component_is_not_hidden_in_generated_pages(self):
        (self.root / 'index.html').write_text('<site-search style="display:none"></site-search>')
        with self.assertRaisesRegex(ValueError, 'Search UI must remain disabled'):
            builder.validate(self.root)

    def test_search_index_is_not_shipped(self):
        (self.root / 'pagefind').mkdir()
        with self.assertRaisesRegex(ValueError, 'Search indexing must remain disabled'):
            builder.validate(self.root)

    def test_missing_tabler_license_is_rejected(self):
        (self.root / 'tabler-license.txt').unlink()
        with self.assertRaisesRegex(ValueError, 'Missing site asset'):
            builder.validate(self.root)

    def test_missing_counter_capture_is_rejected(self):
        (self.root / 'assets/counter-example.png').unlink()
        with self.assertRaisesRegex(ValueError, 'Missing site asset'):
            builder.validate(self.root)

    def test_missing_homepage_screenshot_is_rejected(self):
        (self.root / 'assets/widget-factory.png').unlink()
        with self.assertRaisesRegex(ValueError, 'Missing local link|Missing site asset'):
            builder.validate(self.root)

    def test_try_page_cannot_escape_artifact(self):
        (self.root / 'try/index.html').write_text('<a href="../../outside.html">Outside</a>')
        with self.assertRaisesRegex(ValueError, 'escapes artifact'):
            builder.validate(self.root)

    def test_root_relative_route_resolves_inside_artifact(self):
        (self.root / 'index.html').write_text('<a href="/counter/">Counter</a>')
        builder.validate(self.root)

    def test_encoded_root_relative_escape_is_rejected(self):
        (self.root / 'index.html').write_text('<a href="/%2e%2e/outside.html">Outside</a>')
        with self.assertRaisesRegex(ValueError, 'escapes artifact'):
            builder.validate(self.root)

    def test_missing_link_is_rejected(self):
        (self.root / 'index.html').write_text('<a href="./missing/">Missing</a>')
        with self.assertRaisesRegex(ValueError, 'Missing local link'):
            builder.validate(self.root)

    def test_path_escape_is_rejected(self):
        (self.root / 'index.html').write_text('<a href="../outside.html">Outside</a>')
        with self.assertRaisesRegex(ValueError, 'escapes artifact'):
            builder.validate(self.root)

    def test_invalid_wasm_is_rejected(self):
        (self.root / 'counter/pkg/scarlet_ui_web_counter_bg.wasm').write_bytes(b'not wasm')
        with self.assertRaisesRegex(ValueError, 'Invalid WASM'):
            builder.validate(self.root)

    def test_missing_glue_is_rejected(self):
        (self.root / 'counter/pkg/scarlet_ui_web_counter.js').unlink()
        with self.assertRaisesRegex(ValueError, 'Missing WASM reference'):
            builder.validate(self.root)


if __name__ == '__main__':
    unittest.main()
