"""Static package validation regressions (no browser, compiler or network)."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import os
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
        self.api = self.root / 'docs'
        for package in builder.API_PACKAGES:
            folder = self.api / package.replace('-', '_')
            folder.mkdir(parents=True)
            (folder / 'index.html').write_text('<a href="../scarlet_ui/index.html">API</a>')
        (self.api / 'index.html').write_text('<a href="./scarlet_ui/index.html">API</a>')
        (self.api / 'crates.js').write_text('window.ALL_CRATES = [];')
        (self.api / 'static.files').mkdir()
        (self.api / 'static.files/rustdoc.css').touch()
        (self.api / 'static.files/search.js').touch()
        (self.api / 'search.index').mkdir()
        (self.api / 'search.index/root.js').touch()
        (self.api / 'src/scarlet_ui').mkdir(parents=True)
        (self.api / 'src/scarlet_ui/lib.rs.html').write_text('<a href="../../scarlet_ui/index.html">API</a>')
        for route, _, module in builder.DEMOS:
            folder = self.root / route
            (folder / 'pkg').mkdir(parents=True)
            (folder / 'index.html').write_text(f"<script>import('./pkg/{module}.js')</script>")
            (folder / 'pkg' / f'{module}.js').write_text(f"new URL('{module}_bg.wasm', import.meta.url)")
            (folder / 'pkg' / f'{module}_bg.wasm').write_bytes(b'\0asm\x01\0\0\0')

    def test_api_reference_has_crates_sources_search_and_static_assets(self):
        (self.api / 'scarlet_ui/index.html').write_text(
            '<a href="../scarlet_ui_core/index.html">Core</a>'
            '<a href="../src/scarlet_ui/lib.rs.html">Source</a>'
            '<script src="../static.files/search.js"></script>'
            '<link href="../static.files/rustdoc.css">')
        builder.validate(self.root)

    def test_missing_api_crate_is_rejected(self):
        (self.api / 'scarlet_ui_macros/index.html').unlink()
        with self.assertRaisesRegex(ValueError, 'Missing API crate'):
            builder.validate(self.root)

    def test_missing_rustdoc_search_index_is_rejected(self):
        (self.api / 'search.index/root.js').unlink()
        with self.assertRaisesRegex(ValueError, 'Missing rustdoc search index'):
            builder.validate(self.root)

    def test_older_rustdoc_search_index_is_supported(self):
        (self.api / 'search.index/root.js').unlink()
        (self.api / 'search-index.js').touch()
        builder.validate(self.root)

    def test_missing_rustdoc_source_is_rejected(self):
        (self.api / 'src/scarlet_ui/lib.rs.html').unlink()
        with self.assertRaisesRegex(ValueError, 'Missing rustdoc assets'):
            builder.validate(self.root)

    def test_api_staging_keeps_dynamic_assets_and_source_pages(self):
        stage = self.root / 'staged'
        stage.mkdir()
        builder.stage_api_docs(stage, self.api)
        for path in self.api.rglob('*'):
            if path.is_file() and path.name != 'index.html':
                self.assertEqual((stage / 'docs' / path.relative_to(self.api)).read_bytes(), path.read_bytes())
        entry = (stage / 'docs/index.html').read_text()
        self.assertIn('content="0; url=./scarlet_ui/index.html"', entry)
        self.assertIn('href="./scarlet_ui/index.html"', entry)
        builder.validate_api_docs(stage / 'docs')

    def test_upstream_rustdoc_workarounds_are_narrow_and_idempotent(self):
        page = self.api / 'scarlet_ui/index.html'
        missing = '../trait.impl/scarlet_ui_core/application/trait.Application.js'
        present = '../trait.impl/scarlet_ui_core/preview/trait.PreviewSession.js'
        canonical = self.api / 'scarlet_ui_core/application/trait.Application.html'
        canonical.parent.mkdir(parents=True)
        canonical.write_text('<div id="implementors-list"></div>')
        actual = (page.parent / present).resolve()
        actual.parent.mkdir(parents=True)
        actual.write_text('window.pending_implementors = {existing: true};')
        page.write_text('<a href="dispatcher#setting-the-default-subscriber">Default</a>'
                        '<a href="super::Subscriber">Subscriber</a>'
                        f'<script src="{missing}" async></script>'
                        f'<script src="{present}" async></script>')
        builder.normalize_api_docs(self.api)
        html = page.read_text()
        self.assertIn('/tracing/dispatcher/index.html#setting-the-default-subscriber', html)
        self.assertIn('/tracing/trait.Subscriber.html', html)
        self.assertIn(missing, html)
        generated = (page.parent / missing).resolve()
        data = generated.read_text()
        self.assertIn('window.register_implementors(implementors)', data)
        self.assertIn('window.pending_implementors = implementors', data)
        self.assertIn(present, html)
        self.assertEqual(actual.read_text(), 'window.pending_implementors = {existing: true};')
        builder.normalize_api_docs(self.api)
        self.assertEqual(page.read_text(), html)
        self.assertEqual(generated.read_text(), data)
        builder.validate(self.root)

    def test_empty_clipboard_implementors_are_packaged_for_canonical_and_reexported_traits(self):
        source = '../../trait.impl/scarlet_ui_core/clipboard/trait.ClipboardProvider.js'
        html = f'<div id="implementors-list"></div><script src="{source}" async></script>'
        for crate in ('scarlet_ui_core', 'scarlet_ui'):
            page = self.api / crate / 'clipboard/trait.ClipboardProvider.html'
            page.parent.mkdir(parents=True)
            page.write_text(html)
        with self.assertRaisesRegex(ValueError, 'Missing local link'):
            builder.validate(self.root)
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            builder.stage_api_docs(stage, self.api)
            for crate in ('scarlet_ui_core', 'scarlet_ui'):
                page = stage / 'docs' / crate / 'clipboard/trait.ClipboardProvider.html'
                self.assertEqual(page.read_text(), html)
                self.assertTrue((page.parent / source).is_file())
        # Staging repairs the package, not Cargo's generated output.
        self.assertFalse((self.api / 'trait.impl/scarlet_ui_core/clipboard/trait.ClipboardProvider.js').exists())
        builder.normalize_api_docs(self.api)
        builder.validate(self.root)

    def test_known_trait_with_missing_or_nonempty_canonical_page_is_not_filled(self):
        source = '../trait.impl/scarlet_ui_core/clipboard/trait.ClipboardProvider.js'
        page = self.api / 'scarlet_ui/index.html'
        page.write_text(f'<script src="{source}" async></script>')
        canonical = self.api / 'scarlet_ui_core/clipboard/trait.ClipboardProvider.html'
        for html in (None, '<div id="implementors-list"><section class="impl">Provider</section></div>'):
            with self.subTest(canonical=html):
                if html is not None:
                    canonical.parent.mkdir(parents=True)
                    canonical.write_text(html)
                builder.normalize_api_docs(self.api)
                self.assertFalse((page.parent / source).exists())
                with self.assertRaisesRegex(ValueError, 'Missing local link'):
                    builder.validate(self.root)

    def test_unexpected_missing_implementation_script_still_fails(self):
        page = self.api / 'scarlet_ui/index.html'
        page.write_text('<script src="../trait.impl/scarlet_ui/trait.Unknown.js" async></script>')
        builder.normalize_api_docs(self.api)
        with self.assertRaisesRegex(ValueError, 'Missing local link'):
            builder.validate(self.root)

    def test_api_build_uses_isolated_native_docs_and_compatible_features(self):
        target = self.root / 'custom-target'
        docs = target / 'api-reference/doc'
        for package in builder.API_PACKAGES:
            folder = docs / package.replace('-', '_')
            folder.mkdir(parents=True)
            (folder / 'index.html').touch()
        with patch.dict(os.environ, {'CARGO_BUILD_TARGET': 'wasm32-unknown-unknown'}):
            with patch.object(builder.subprocess, 'run') as run:
                self.assertEqual(builder.build_api_docs(target), docs)
        clean, build = run.call_args_list
        self.assertEqual(clean.args[0], ['cargo', 'clean', '--doc', '--target-dir', str(target / 'api-reference')])
        command = build.args[0]
        self.assertIn('--no-deps', command)
        self.assertIn('--locked', command)
        self.assertIn('--no-default-features', command)
        self.assertIn('scarlet-ui/std,scarlet-ui/platform-winit,scarlet-ui/preview', command)
        self.assertNotIn('--all-features', command)
        self.assertNotIn('--target', command)
        self.assertNotIn('CARGO_BUILD_TARGET', build.kwargs['env'])
        self.assertIn('--deny=rustdoc::broken_intra_doc_links', build.kwargs['env']['CARGO_ENCODED_RUSTDOCFLAGS'])
        self.assertIn('--deny=rustdoc::invalid_html_tags', build.kwargs['env']['CARGO_ENCODED_RUSTDOCFLAGS'])
        for package in builder.API_PACKAGES:
            self.assertIn(package, command)

    def test_incomplete_or_cross_target_api_build_is_rejected(self):
        with patch.object(builder.subprocess, 'run'):
            with self.assertRaisesRegex(ValueError, 'build.target Cargo configuration'):
                builder.build_api_docs(self.root / 'empty-target')

    def test_diagnostic_is_opt_in_and_reuses_factory_runtime(self):
        import json
        before = {str(p.relative_to(self.root)): p.read_bytes()
                  for p in self.root.rglob('*') if p.is_file()}
        identity = {'source_commit': 'test-commit', 'source_dirty': False}
        builder.stage_input_diagnostic(self.root, identity)
        page = self.root / 'diagnostics/input/index.html'
        html = page.read_text()
        self.assertIn('name="robots" content="noindex,nofollow"', html)
        self.assertIn('../../widget-factory/pkg/scarlet_ui_widget_factory.js', html)
        self.assertIn('"source_commit": "test-commit"', html)
        self.assertIn('<script src="./focus-trace.js"></script>', html)
        self.assertFalse((page.parent / 'pkg').exists())
        safe_area = self.root / 'diagnostics/safe-area/index.html'
        self.assertTrue(safe_area.is_file())
        self.assertIn('browser_safe_area_insets', safe_area.read_text())
        self.assertIn('viewport-fit=cover', safe_area.read_text())
        self.assertNotIn('await start()', safe_area.read_text())
        for name, data in before.items():
            self.assertEqual((self.root / name).read_bytes(), data, name)
        builder.validate(self.root)
        self.assertTrue((page.parent / '../../widget-factory/pkg/scarlet_ui_widget_factory.js').resolve().is_file())

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
