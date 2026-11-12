#!/usr/bin/env python3
"""Build the root-hosted Starlight guides, rustdoc API reference and browser samples. Requires Node 24+, Python 3.11+, Cargo, wasm-bindgen."""
from __future__ import annotations

import argparse
import hashlib
from html.parser import HTMLParser
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
from urllib.parse import urlsplit, unquote

ROOT = Path(__file__).resolve().parents[1]
DEMOS = (
    ('widget-factory', 'scarlet-ui-widget-factory', 'scarlet_ui_widget_factory'),
    ('animation-lab', 'scarlet-ui-animation-lab', 'scarlet_ui_animation_lab'),
    ('counter', 'scarlet-ui-web-counter', 'scarlet_ui_web_counter'),
)
LIFECYCLE_EXPORTS = ('start', 'stop', 'runtime_error', 'pause', 'resume')
MARKER = '.scarletui-samples'
API_PACKAGES = (
    'scarlet-ui', 'scarlet-ui-core', 'scarlet-ui-macros',
    'scarlet-ui-icons-tabler', 'scarlet-scroll-motion',
    'scarlet-ui-platform-winit', 'scarlet-ui-renderer-sgfx',
)
API_FEATURES = 'scarlet-ui/std,scarlet-ui/platform-winit,scarlet-ui/preview'
SITE_FILES = ('docs/index.html', 'docs/scarlet_ui/index.html', 'docs/crates.js', 'index.html', 'try/index.html', 'capability.js', 'assets/widget-factory.png', 'assets/counter-example.png')


def run(*args: str) -> str:
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def source_identity(root: Path = ROOT) -> dict:
    # Source archives have no Git metadata; never accidentally report a parent repo.
    if not (root / '.git').exists():
        return {'source_commit': None, 'source_dirty': None}
    try:
        def git(*args: str) -> str:
            return subprocess.check_output(['git', '-C', str(root), *args], text=True, stderr=subprocess.DEVNULL).strip()
        return {'source_commit': git('rev-parse', 'HEAD'),
                'source_dirty': bool(git('status', '--porcelain'))}
    except (subprocess.CalledProcessError, FileNotFoundError):
        return {'source_commit': None, 'source_dirty': None}


class References(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.urls: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        for key, value in attrs:
            if key in ('href', 'src') and value:
                self.urls.append(value)


def validate(directory: Path) -> None:
    """Check local references for the approved custom-domain root deployment."""
    directory = directory.resolve()
    if (directory / 'pagefind').exists():
        raise ValueError('Search indexing must remain disabled')
    for file in directory.rglob('*.html'):
        parser = References()
        html = file.read_text()
        if '<site-search' in html.lower():
            raise ValueError(f'Search UI must remain disabled: {file}')
        parser.feed(html)
        for url in parser.urls:
            parts = urlsplit(url)
            if parts.scheme or parts.netloc or not parts.path:
                continue
            path = unquote(parts.path)
            # Root-relative URLs are expected in generated Starlight pages.
            # Resolve them inside the artifact, never the host filesystem.
            destination = ((directory / path.lstrip('/')) if path.startswith('/')
                           else (file.parent / path)).resolve()
            if not destination.is_relative_to(directory):
                raise ValueError(f'URL escapes artifact: {file}: {url}')
            if destination.is_dir():
                destination /= 'index.html'
            if not destination.is_file():
                raise ValueError(f'Missing local link: {file}: {url}')
    for route, _, module in DEMOS:
        page = directory / route / 'index.html'
        glue = directory / route / 'pkg' / f'{module}.js'
        wasm = directory / route / 'pkg' / f'{module}_bg.wasm'
        if not page.is_file():
            raise ValueError(f'Missing sample page: {page}')
        if f'./pkg/{module}.js' not in page.read_text():
            raise ValueError(f'Missing module import: {page}')
        if not glue.is_file() or f'{module}_bg.wasm' not in glue.read_text():
            raise ValueError(f'Missing WASM reference: {glue}')
        exports = set(re.findall(r'^export (?:async )?function (\w+)\(', glue.read_text(), re.MULTILINE))
        missing = set(LIFECYCLE_EXPORTS) - exports
        if missing:
            raise ValueError(f'Missing sample lifecycle exports: {glue}: {", ".join(sorted(missing))}')
        if not wasm.is_file() or wasm.read_bytes()[:8] != b'\0asm\x01\0\0\0':
            raise ValueError(f'Invalid WASM artifact: {wasm}')
    validate_api_docs(directory / 'docs')
    for filename in (*SITE_FILES, '.nojekyll', 'font-license.txt', 'tabler-license.txt'):
        if not (directory / filename).is_file():
            raise ValueError(f'Missing site asset: {filename}')


def validate_api_docs(docs: Path) -> None:
    """Check assets loaded by rustdoc JavaScript as well as ordinary HTML links."""
    for package in API_PACKAGES:
        if not (docs / package.replace('-', '_') / 'index.html').is_file():
            raise ValueError(f'Missing API crate: {package}')
    for pattern in ('static.files/*.css', 'static.files/*.js', 'src/**/*.html'):
        if not any(docs.glob(pattern)):
            raise ValueError(f'Missing rustdoc assets: {pattern}')
    # Rustdoc has used both a flat search index and a sharded search.index tree.
    if not (any(docs.glob('search-index*.js')) or any((docs / 'search.index').rglob('*.js'))):
        raise ValueError('Missing rustdoc search index')


def build_api_docs(target: Path) -> Path:
    """Build the native public API without conflicting Scarlet/Web features.

    An isolated target directory and a clean doc tree keep old packages/features
    out of rustdoc's cumulative search index. Compiler intermediates stay cached.
    Use Cargo native mode so proc-macro and library docs share one search index.
    Cross-target API documentation is a separate build, not this native reference.
    """
    target = target / 'api-reference'
    options = ['--target-dir', str(target)]
    env = {key: value for key, value in os.environ.items() if key != 'CARGO_BUILD_TARGET'}
    subprocess.run(['cargo', 'clean', '--doc', *options], cwd=ROOT, check=True, env=env)
    command = ['cargo', 'doc', '--locked', '--lib', '--no-deps', '--no-default-features',
               '--features', API_FEATURES, *options]
    for package in API_PACKAGES:
        command += ['-p', package]
    # Fail publication on broken source links or malformed Rustdoc HTML while
    # preserving caller-supplied flags (including encoded paths with spaces).
    flags = env.get('CARGO_ENCODED_RUSTDOCFLAGS')
    if flags is None:
        flags = '\x1f'.join(env.get('RUSTDOCFLAGS', '').split())
    env['CARGO_ENCODED_RUSTDOCFLAGS'] = '\x1f'.join(filter(None, (
        flags, '--deny=rustdoc::broken_intra_doc_links', '--deny=rustdoc::invalid_html_tags')))
    subprocess.run(command, cwd=ROOT, check=True, env=env)
    docs = target / 'doc'
    for package in API_PACKAGES:
        if not (docs / package.replace('-', '_') / 'index.html').is_file():
            raise ValueError(f'Missing native API reference: {package}. Build without a build.target Cargo configuration.')
    return docs


def stage_api_docs(stage: Path, docs: Path) -> None:
    """Keep rustdoc's layout, search data, source pages and assets together."""
    shutil.copytree(docs, stage / 'docs')
    shutil.copy2(ROOT / 'site/rustdoc/index.html', stage / 'docs/index.html')
    normalize_api_docs(stage / 'docs')


def normalize_api_docs(docs: Path) -> None:
    """Repair only known upstream rustdoc artifacts, before strict link checks.

    tracing's blanket impl snippets retain module-relative Markdown links when
    inlined into unrelated crates. Rustdoc references implementor scripts even
    for traits absent from its implementor map. Package empty data for the known
    cases below, only while their canonical pages have no public implementors.
    Keep the script references and strict validation of all other missing assets.
    """
    lock = tomllib.loads((ROOT / 'Cargo.lock').read_text())
    tracing = next(package['version'] for package in lock['package'] if package['name'] == 'tracing')
    upstream = f'https://docs.rs/tracing/{tracing}/tracing'
    empty_traits = {
        'trait.impl/scarlet_ui_core/application/trait.Application.js',
        'trait.impl/scarlet_ui_core/clipboard/trait.ClipboardProvider.js',
        'trait.impl/scarlet_ui_core/file_dialog/trait.FileDialogSession.js',
        'trait.impl/scarlet_ui_core/preview/trait.PreviewSession.js',
        'trait.impl/scarlet_ui_core/preview/trait.PreviewLibrary.js',
    }
    empty_implementors = '''(function() {
    const implementors = {};
    if (window.register_implementors) {
        window.register_implementors(implementors);
    } else {
        window.pending_implementors = implementors;
    }
})()
'''
    root = docs.resolve()
    for file in docs.rglob('*.html'):
        original = file.read_text()
        html = original.replace('href="dispatcher#setting-the-default-subscriber"',
                                f'href="{upstream}/dispatcher/index.html#setting-the-default-subscriber"')
        html = html.replace('href="super::Subscriber"', f'href="{upstream}/trait.Subscriber.html"')

        for source in re.findall(r'<script src="([^"<>]+)" async></script>', html):
            target = (file.parent / source).resolve()
            if not target.is_relative_to(root) or target.exists():
                continue
            relative = target.relative_to(root)
            if relative.as_posix() not in empty_traits:
                continue
            canonical = root.joinpath(*relative.parts[1:]).with_suffix('.html')
            if not canonical.is_file() or '<div id="implementors-list"></div>' not in canonical.read_text():
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(empty_implementors)
        if html != original:
            file.write_text(html)


def stage_input_diagnostic(stage: Path, identity: dict) -> None:
    """Opt-in top-level page; reuse Factory runtime without changing normal pages."""
    destination = stage / 'diagnostics/input'
    destination.mkdir(parents=True)
    html = (ROOT / 'examples/widget-factory/index.html').read_text()
    original = "'./pkg/scarlet_ui_widget_factory.js'"
    if html.count(original) != 1:
        raise ValueError('Factory module import changed; update diagnostic route explicitly')
    for marker in ('<meta charset=', '<script type="module">'):
        if html.count(marker) != 1:
            raise ValueError(f'Factory HTML marker changed: {marker}')
    html = html.replace(original, "'../../widget-factory/pkg/scarlet_ui_widget_factory.js'")
    html = html.replace('<meta charset=', '<meta name="robots" content="noindex,nofollow">\n<meta charset=', 1)
    metadata = json.dumps({'source': 'ScarletUI iOS focus diagnostic', **identity}).replace('<', '\\u003c')
    trace = ('<script type="application/json" id="scarlet-diagnostic-build">' + metadata + '</script>'
             '<script src="./focus-trace.js"></script>\n')
    html = html.replace('<script type="module">', trace + '<script type="module">', 1)
    (destination / 'index.html').write_text(html)
    shutil.copy2(ROOT / 'samples/diagnostics/focus-trace.js', destination / 'focus-trace.js')
    safe_area = stage / 'diagnostics/safe-area'
    safe_area.mkdir(parents=True)
    shutil.copy2(ROOT / 'samples/diagnostics/safe-area.html', safe_area / 'index.html')


def build(output: Path, profile: str) -> None:
    output = output.resolve()
    if output == ROOT or ROOT.is_relative_to(output):
        raise ValueError('Output cannot replace the repository or its ancestors')
    if output.exists() and (not output.is_dir() or not (output / MARKER).is_file()):
        raise ValueError(f'Refusing to replace unmarked output directory: {output}')
    lock = tomllib.loads((ROOT / 'Cargo.lock').read_text())
    version = next(p['version'] for p in lock['package'] if p['name'] == 'wasm-bindgen')
    installed = run('wasm-bindgen', '--version')
    if installed != f'wasm-bindgen {version}':
        raise ValueError(f'Expected wasm-bindgen {version}, found {installed}. Install the matching CLI with --locked.')
    command = ['cargo', 'build', '--locked', '--lib', '--target', 'wasm32-unknown-unknown']
    if profile == 'release':
        command.append('--release')
    for _, package, _ in DEMOS:
        command += ['-p', package]
    subprocess.run(command, cwd=ROOT, check=True)
    subprocess.run(['npm', 'run', 'build', '--prefix', 'site'], cwd=ROOT, check=True,
                   env={**os.environ, 'ASTRO_TELEMETRY_DISABLED': '1'})
    # Cargo metadata honors CARGO_TARGET_DIR and workspace configuration.
    metadata = json.loads(run('cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'))
    api_docs = build_api_docs(Path(metadata['target_directory']))
    target = Path(metadata['target_directory']) / 'wasm32-unknown-unknown' / ('release' if profile == 'release' else 'debug')
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.scarletui-samples-', dir=output.parent) as temporary:
        stage = Path(temporary) / 'site'
        shutil.copytree(ROOT / 'site/dist', stage)
        shutil.copy2(ROOT / 'samples/capability.js', stage / 'capability.js')
        (stage / '.nojekyll').touch()
        (stage / MARKER).write_text('Generated by tools/build-web-samples.py\n')
        shutil.copy2(ROOT / 'examples/web-counter/assets/LICENSE', stage / 'font-license.txt')
        shutil.copy2(ROOT / 'crates/scarlet-ui-icons-tabler/LICENSE', stage / 'tabler-license.txt')
        for route, _, module in DEMOS:
            demo = stage / route
            demo.mkdir()
            source = 'web-counter' if route == 'counter' else route
            shutil.copy2(ROOT / 'examples' / source / 'index.html', demo / 'index.html')
            subprocess.run(['wasm-bindgen', str(target / f'{module}.wasm'), '--target', 'web', '--no-typescript', '--out-dir', str(demo / 'pkg')], check=True, cwd=ROOT)
        stage_input_diagnostic(stage, source_identity())
        subprocess.run(['python3', str(ROOT / 'site/scripts/social-metadata.py'), str(stage)], check=True, cwd=ROOT)
        subprocess.run(['node', '--test', str(ROOT / 'site/scripts/metadata-built.check.mjs')], check=True, cwd=ROOT,
                       env={**os.environ, 'SCARLET_SITE_DIR': str(stage)})
        # API reference keeps rustdoc HTML intact; do not create thousands of
        # Starlight social cards for generated items, sources and redirect pages.
        stage_api_docs(stage, api_docs)
        validate(stage)
        manifest = {
            'format': 1,
            'profile': profile,
            'wasm_bindgen': version,
            'api_reference': {'packages': list(API_PACKAGES), 'features': API_FEATURES},
            **source_identity(),
            'files': {str(p.relative_to(stage)): {'bytes': p.stat().st_size, 'sha256': hashlib.sha256(p.read_bytes()).hexdigest()}
                      for p in sorted(stage.rglob('*')) if p.is_file()},
        }
        (stage / 'build-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        if output.exists():
            shutil.rmtree(output)
        shutil.move(str(stage), output)
    print(f'Samples ready: {output}')


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / 'target/web-samples')
    parser.add_argument('--profile', choices=('release', 'dev'), default='release')
    parser.add_argument('--check', type=Path, help='Validate an existing package without building')
    args = parser.parse_args()
    if args.check:
        validate(args.check)
        print(f'Samples package valid: {args.check}')
    else:
        build(args.output, args.profile)


if __name__ == '__main__':
    main()
