import pathlib
import sys
import unittest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from sanitize_active_content import SanitizationError, sanitize_html, sanitize_svg


class ActiveContentSanitizerTests(unittest.TestCase):
    def test_safe_svg_is_deterministic_and_source_free(self):
        source = b'<svg xmlns="http://www.w3.org/2000/svg" width="10" height="20"><rect id="r" x="0" y="0" width="10" height="20" fill="#fff"/></svg>'
        first, receipt1 = sanitize_svg(source)
        second, receipt2 = sanitize_svg(source)
        self.assertEqual(first, second)
        self.assertEqual(receipt1, receipt2)
        self.assertIn(b'xmlns="http://www.w3.org/2000/svg"', first)
        self.assertFalse(receipt1["network_fetch_allowed"])
        self.assertFalse(receipt1["active_content_allowed"])

    def test_svg_script_event_foreign_object_and_external_href_fail_closed(self):
        bad = [
            b'<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>',
            b'<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"></svg>',
            b'<svg xmlns="http://www.w3.org/2000/svg"><foreignObject><div>x</div></foreignObject></svg>',
            b'<svg xmlns="http://www.w3.org/2000/svg"><use href="https://example.com/a.svg#x"/></svg>',
            b'<!DOCTYPE svg [<!ENTITY x SYSTEM "http://127.0.0.1/x">]><svg xmlns="http://www.w3.org/2000/svg">&x;</svg>',
        ]
        for source in bad:
            with self.subTest(source=source[:80]):
                with self.assertRaises(SanitizationError):
                    sanitize_svg(source)

    def test_svg_local_fragment_refs_are_allowed_but_network_urls_are_not(self):
        safe = b'<svg xmlns="http://www.w3.org/2000/svg"><defs><clipPath id="c"><rect x="0" y="0" width="1" height="1"/></clipPath></defs><g clip-path="url(#c)"><use href="#c"/></g></svg>'
        sanitized, _ = sanitize_svg(safe)
        self.assertIn(b'clip-path="url(#c)"', sanitized)
        self.assertIn(b'href="#c"', sanitized)

        with self.assertRaises(SanitizationError):
            sanitize_svg(b'<svg xmlns="http://www.w3.org/2000/svg"><rect fill="url(https://example.com/x)"/></svg>')

    def test_safe_static_html_is_canonicalized(self):
        source = b'<div id="a"><p>Hello &amp; <strong>world</strong></p><br></div>'
        sanitized, receipt = sanitize_html(source)
        self.assertEqual(
            sanitized,
            b'<div id="a"><p>Hello &amp; <strong>world</strong></p><br></div>',
        )
        self.assertEqual(receipt["kind"], "html")

    def test_html_active_or_network_content_fails_closed(self):
        bad = [
            b'<script>alert(1)</script>',
            b'<div onclick="alert(1)">x</div>',
            b'<iframe src="https://example.com"></iframe>',
            b'<img src="https://example.com/a.png">',
            b'<a href="javascript:alert(1)">x</a>',
            b'<!DOCTYPE html><div>x</div>',
        ]
        for source in bad:
            with self.subTest(source=source):
                with self.assertRaises(SanitizationError):
                    sanitize_html(source)

    def test_malformed_html_and_invalid_utf8_fail_closed(self):
        with self.assertRaises(SanitizationError):
            sanitize_html(b"<div><span>x</div>")
        with self.assertRaises(SanitizationError):
            sanitize_html(b"\xff\xfe")


if __name__ == "__main__":
    unittest.main()
