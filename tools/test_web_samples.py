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
        for name in ('styles.css', 'capability.js', '.nojekyll', 'font-license.txt'):
            (self.root / name).touch()
        (self.root / 'index.html').write_text('<a href="./counter/">Counter</a><a href="./widget-factory/">Factory</a><link href="./styles.css">')
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

    def test_relative_routes_work_under_arbitrary_prefix(self):
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

    def test_root_relative_route_is_rejected(self):
        (self.root / 'index.html').write_text('<a href="/counter/">Counter</a>')
        with self.assertRaisesRegex(ValueError, 'Root-relative'):
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
