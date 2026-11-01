"""Exercise the exact inline deploy guard without network or GitHub permissions."""
import contextlib
import io
import os
from pathlib import Path
import tempfile
import textwrap
import unittest
from unittest.mock import patch

WORKFLOW = Path(__file__).resolve().parents[1] / '.github/workflows/samples.yml'
GUARD = textwrap.dedent(WORKFLOW.read_text().split(
    '# BEGIN TESTED DESTINATION GUARD\n', 1)[1].split(
    '          # END TESTED DESTINATION GUARD', 1)[0])


class PagesDestination(unittest.TestCase):
    def check(self, origin, base='', url=None):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / 'output'
            with patch.dict(os.environ, {
                'PAGE_ORIGIN': origin, 'PAGE_BASE': base,
                'PAGE_URL': origin if url is None else url,
                'GITHUB_OUTPUT': str(output),
            }, clear=True), contextlib.redirect_stdout(io.StringIO()):
                exec(compile(GUARD, str(WORKFLOW), 'exec'), {})
            return output.read_text()

    def test_exact_custom_root_always_outputs_https(self):
        for origin in ('https://ui.scarlet-os.org', 'http://ui.scarlet-os.org'):
            for base in ('', '/'):
                for suffix in ('', '/'):
                    with self.subTest(origin=origin, base=base, suffix=suffix):
                        self.assertEqual(self.check(origin, base, origin + suffix),
                                         'page_url=https://ui.scarlet-os.org/\n')

    def test_wrong_origin_is_rejected(self):
        for origin in ('https://ichigo.dev', 'https://petitstrawberry.github.io',
                       'https://ui.scarlet-os.org.evil.example',
                       'https://ui.scarlet-os.org:443', 'https://user@ui.scarlet-os.org',
                       'ftp://ui.scarlet-os.org', 'https://UI.scarlet-os.org',
                       'https://ui.scarlet-os.org/', 'https://ui.scarlet-os.org\n'):
            with self.subTest(origin=origin), self.assertRaises(ValueError):
                self.check(origin)

    def test_project_or_malformed_base_is_rejected(self):
        for base in ('/scarlet-ui', '//', '/counter/', '/%2f', '/..', '\n'):
            with self.subTest(base=base), self.assertRaises(ValueError):
                self.check('https://ui.scarlet-os.org', base)

    def test_mismatched_or_decorated_url_is_rejected(self):
        origin = 'https://ui.scarlet-os.org'
        for url in ('http://ui.scarlet-os.org', origin + '//', origin + '/counter/',
                    origin + '/?next=x', origin + '/#fragment', origin + '/%2e',
                    'https://ui.scarlet-os.org@evil.example/', origin + '\n'):
            with self.subTest(url=url), self.assertRaises(ValueError):
                self.check(origin, url=url)


if __name__ == '__main__':
    unittest.main()
