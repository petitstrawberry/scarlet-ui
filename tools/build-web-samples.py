#!/usr/bin/env python3
"""Build a relocatable static Samples artifact. Requires Python 3.11+, Cargo, wasm-bindgen."""
from __future__ import annotations

import argparse
import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
from urllib.parse import urlsplit, unquote

ROOT = Path(__file__).resolve().parents[1]
DEMOS = (
    ('widget-factory', 'scarlet-ui-widget-factory', 'scarlet_ui_widget_factory'),
    ('counter', 'scarlet-ui-web-counter', 'scarlet_ui_web_counter'),
)
MARKER = '.scarletui-samples'


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
    """Check routes under any deployment prefix, including GitHub project Pages."""
    directory = directory.resolve()
    for file in directory.rglob('*.html'):
        parser = References()
        parser.feed(file.read_text())
        for url in parser.urls:
            parts = urlsplit(url)
            if parts.scheme or parts.netloc or not parts.path:
                continue
            if parts.path.startswith('/'):
                raise ValueError(f'Root-relative URL breaks project Pages: {file}: {url}')
            destination = (file.parent / unquote(parts.path)).resolve()
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
        if f'./pkg/{module}.js' not in page.read_text():
            raise ValueError(f'Missing module import: {page}')
        if not glue.is_file() or f'{module}_bg.wasm' not in glue.read_text():
            raise ValueError(f'Missing WASM reference: {glue}')
        if not wasm.is_file() or wasm.read_bytes()[:8] != b'\0asm\x01\0\0\0':
            raise ValueError(f'Invalid WASM artifact: {wasm}')
    for filename in ('index.html', 'styles.css', 'capability.js', '.nojekyll', 'font-license.txt'):
        if not (directory / filename).is_file():
            raise ValueError(f'Missing site asset: {filename}')


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
    # Cargo metadata honors CARGO_TARGET_DIR and workspace configuration.
    metadata = json.loads(run('cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'))
    target = Path(metadata['target_directory']) / 'wasm32-unknown-unknown' / ('release' if profile == 'release' else 'debug')
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.scarletui-samples-', dir=output.parent) as temporary:
        stage = Path(temporary) / 'site'
        stage.mkdir()
        for name in ('index.html', 'styles.css', 'capability.js'):
            shutil.copy2(ROOT / 'samples' / name, stage / name)
        (stage / '.nojekyll').touch()
        (stage / MARKER).write_text('Generated by tools/build-web-samples.py\n')
        shutil.copy2(ROOT / 'examples/web-counter/assets/LICENSE', stage / 'font-license.txt')
        for route, _, module in DEMOS:
            demo = stage / route
            demo.mkdir()
            source = 'web-counter' if route == 'counter' else route
            shutil.copy2(ROOT / 'examples' / source / 'index.html', demo / 'index.html')
            subprocess.run(['wasm-bindgen', str(target / f'{module}.wasm'), '--target', 'web', '--no-typescript', '--out-dir', str(demo / 'pkg')], check=True, cwd=ROOT)
        validate(stage)
        manifest = {
            'format': 1,
            'profile': profile,
            'wasm_bindgen': version,
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
