#!/usr/bin/env python3
"""Derive social cards and metadata from built pages, including standalone samples."""
import argparse
import html
from html.parser import HTMLParser
import json
import re
from pathlib import Path
import subprocess

ORIGIN = 'https://ui.scarlet-os.org'
SCRIPT = Path(__file__).resolve().parent


class Page(HTMLParser):
    def __init__(self, source):
        super().__init__(convert_charrefs=True)
        self.source = source
        self.lines = source.splitlines(keepends=True)
        self.title = ''
        self.in_title = False
        self.in_heading = False
        self.heading = ''
        self.content = False
        self.paragraph = None
        self.paragraphs = []
        self.meta = {}
        self.canonical = None
        self.removals = []
        self.insertion = None
        self.feed(source)

    def position(self):
        line, column = self.getpos()
        return sum(map(len, self.lines[:line - 1])) + column

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if 'sl-markdown-content' in attrs.get('class', '').split():
            self.content = True
        if tag == 'h1':
            self.in_heading = True
        if tag == 'p' and self.content:
            self.paragraph = ''
        if tag == 'title':
            self.in_title = True
            if self.insertion is None:
                self.insertion = self.position()
        if tag == 'meta':
            key = attrs.get('property', attrs.get('name', '')).lower()
            self.meta[key] = attrs.get('content', '')
            if key.startswith(('og:', 'twitter:')) or key == 'description':
                self.removals.append((self.position(), self.position() + len(self.get_starttag_text())))
        if tag == 'link' and attrs.get('rel') == 'canonical':
            self.canonical = attrs.get('href')
            self.removals.append((self.position(), self.position() + len(self.get_starttag_text())))

    def handle_endtag(self, tag):
        if tag == 'h1':
            self.in_heading = False
        if tag == 'p' and self.paragraph is not None:
            value = ' '.join(self.paragraph.split())
            if value and not value.startswith(('This page is generated from', 'Status:')):
                self.paragraphs.append(value)
            self.paragraph = None
        if tag == 'title':
            self.in_title = False
        if tag == 'head':
            self.insertion = self.position()

    def handle_data(self, data):
        if self.in_title:
            self.title += data
        if self.in_heading:
            self.heading += data
        if self.paragraph is not None:
            self.paragraph += data


def excerpt(text, limit=200):
    """A bounded verbatim source excerpt, not an invented page summary."""
    first = re.split(r'(?<=[.!?])\s+(?=[A-Z])', ' '.join(text.split()), maxsplit=1)[0]
    if len(first) <= limit:
        return first
    return first[:limit - 1].rsplit(' ', 1)[0] + '…'


def process(root):
    root = Path(root).resolve()
    cards = []
    for file in sorted(root.rglob('*.html')):
        source = file.read_text()
        page = Page(source)
        relative = file.relative_to(root)
        if file.name == '404.html' or 'noindex' in page.meta.get('robots', '').lower():
            continue
        if not page.title or page.insertion is None:
            raise ValueError(f'Page has no title: {relative}')
        route = relative.as_posix()
        route = route[:-10] if route.endswith('index.html') else route
        canonical = page.canonical or f'{ORIGIN}/{route}'
        title = page.title.strip()
        description = page.meta.get('description')
        if not description or description == 'Cross-platform, declarative UI for Rust.':
            description = excerpt(page.paragraphs[0]) if page.paragraphs else title
        image_relative = 'assets/social-card.png' if route == '' else f'assets/social/{relative.with_suffix(".png").as_posix()}'
        image_url = f'{ORIGIN}/{image_relative}'
        alt = page.meta.get('og:image:alt') if route == '' else None
        alt = alt or title
        values = {
            'description': description, 'og:title': title, 'og:description': description,
            'og:type': page.meta.get('og:type', 'website'), 'og:url': canonical, 'og:image': image_url,
            'og:image:width': '1200', 'og:image:height': '630', 'og:image:type': 'image/png',
            'og:image:alt': alt, 'twitter:card': 'summary_large_image',
            'twitter:title': title, 'twitter:description': description,
            'twitter:image': image_url, 'twitter:image:alt': alt,
        }
        for key in ('og:site_name', 'og:locale'):
            if key in page.meta:
                values[key] = page.meta[key]
        tags = ''.join(f'<meta {"property" if key.startswith("og:") else "name"}="{key}" content="{html.escape(value, quote=True)}">' for key, value in values.items())
        tags += f'<link rel="canonical" href="{html.escape(canonical, quote=True)}">'
        edits = [(start, end, '') for start, end in page.removals]
        edits.append((page.insertion, page.insertion, tags))
        for start, end, replacement in sorted(edits, reverse=True):
            source = source[:start] + replacement + source[end:]
        file.write_text(source)
        # Keep toolkit sharing artwork typographic rather than an icon-library advertisement.
        image_path = None
        card_title = page.heading.strip() or title.removesuffix(' | ScarletUI')
        subtitle = 'Cross-platform, declarative\nUI for Rust.' if route == '' else excerpt(description)
        cards.append({'homepage': route == '', 'title': card_title, 'subtitle': subtitle, 'imagePath': image_path,
                      'output': str(root / image_relative)})
    return cards


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    args = parser.parse_args()
    cards = process(args.directory)
    subprocess.run(['node', str(SCRIPT / 'render-social-manifest.mjs')],
                   input=json.dumps(cards), text=True, check=True)
    print(f'Generated social metadata/cards for {len(cards)} pages')


if __name__ == '__main__':
    main()
