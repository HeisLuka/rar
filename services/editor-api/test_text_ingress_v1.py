#!/usr/bin/env python3
import unittest

from text_ingress_v1 import (
    TextIngressError,
    canonical_story_text_from_json_v1,
    canonical_story_text_json_v1,
    normalize_external_text_v1,
    validate_canonical_fragment_text_v1,
)


class TextIngressV1Tests(unittest.TestCase):
    def test_lf_cr_crlf_converge_to_one_canonical_boundary(self):
        outputs = [
            normalize_external_text_v1("a\nb"),
            normalize_external_text_v1("a\rb"),
            normalize_external_text_v1("a\r\nb"),
        ]
        self.assertEqual({"a\rb"}, {value.text for value in outputs})
        self.assertEqual({3}, {value.scalar_len for value in outputs})
        self.assertEqual({1}, {value.paragraph_boundary_count for value in outputs})

    def test_mixed_newline_spellings_single_pass(self):
        value = normalize_external_text_v1("a\r\nb\nc\rd")
        self.assertEqual("a\rb\rc\rd", value.text)
        self.assertNotIn("\n", value.text)
        self.assertEqual(3, value.paragraph_boundary_count)

    def test_trailing_external_newline_adds_only_supplied_boundary(self):
        self.assertEqual("a\r", normalize_external_text_v1("a\n").text)
        self.assertEqual("a\r", normalize_external_text_v1("a\r\n").text)
        self.assertEqual("a\r", normalize_external_text_v1("a\r").text)

    def test_empty_input_stays_empty_without_terminal_sentinel(self):
        value = normalize_external_text_v1("")
        self.assertEqual("", value.text)
        self.assertEqual(0, value.scalar_len)
        self.assertEqual(0, value.paragraph_boundary_count)

    def test_plain_text_without_newline_does_not_gain_terminal_cr(self):
        value = normalize_external_text_v1("plain")
        self.assertEqual("plain", value.text)
        self.assertFalse(value.text.endswith("\r"))
        self.assertEqual(5, value.scalar_len)

    def test_unicode_is_preserved_without_normalization(self):
        precomposed = normalize_external_text_v1("é")
        decomposed = normalize_external_text_v1("e\u0301")
        self.assertEqual("é", precomposed.text)
        self.assertEqual("e\u0301", decomposed.text)
        self.assertNotEqual(precomposed.text, decomposed.text)
        self.assertEqual(1, precomposed.scalar_len)
        self.assertEqual(2, decomposed.scalar_len)

    def test_emoji_zwj_combining_and_zero_width_are_preserved(self):
        text = "👩\u200d💻 e\u0301 \u200b"
        value = normalize_external_text_v1(text)
        self.assertEqual(text, value.text)
        self.assertEqual(len(text), value.scalar_len)

    def test_desktop_browser_newline_spellings_are_byte_identical_after_ingress(self):
        desktop = normalize_external_text_v1("one\r\ntwo")
        browser = normalize_external_text_v1("one\ntwo")
        self.assertEqual(desktop.text.encode("utf-8"), browser.text.encode("utf-8"))
        self.assertEqual(desktop.text_sha256, browser.text_sha256)

    def test_canonical_fragment_is_validation_only_not_double_normalized(self):
        canonical = "one\rtwo"
        value = validate_canonical_fragment_text_v1(canonical)
        self.assertEqual(canonical, value.text)
        with self.assertRaisesRegex(TextIngressError, "must not contain LF"):
            validate_canonical_fragment_text_v1("one\ntwo")

    def test_replacement_scalar_length_is_after_newline_normalization(self):
        value = normalize_external_text_v1("A\r\n😀")
        self.assertEqual("A\r😀", value.text)
        self.assertEqual(3, value.scalar_len)

    def test_serialization_replay_is_deterministic(self):
        value = normalize_external_text_v1("A\r\n😀e\u0301")
        payload = canonical_story_text_json_v1(value)
        replayed = canonical_story_text_from_json_v1(payload)
        self.assertEqual(value, replayed)

    def test_surrogate_is_rejected_as_non_scalar(self):
        with self.assertRaisesRegex(TextIngressError, "surrogate"):
            normalize_external_text_v1("A\ud800B")


if __name__ == "__main__":
    unittest.main()
