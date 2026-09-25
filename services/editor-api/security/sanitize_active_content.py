#!/usr/bin/env python3
"""Strict default-deny sanitizer for browser-visible SVG/HTML artifacts.

This is intentionally a small safe subset, not a general-purpose browser HTML
rewriter. Unknown/active/network-bearing constructs fail closed.
"""

from __future__ import annotations

import argparse
import hashlib
import html
from html.parser import HTMLParser
import json
from pathlib import Path
import re
import sys
import xml.etree.ElementTree as ET

HERE = Path(__file__).resolve().parent
POLICY = json.loads((HERE / "policy-v1.json").read_text(encoding="utf-8"))
LIMITS = POLICY["limits"]

SVG_NS = "http://www.w3.org/2000/svg"
XLINK_NS = "http://www.w3.org/1999/xlink"
LOCAL_REF_RE = re.compile(r"^#[A-Za-z_][A-Za-z0-9_.:-]*$")
LOCAL_URL_RE = re.compile(r"^url\(#[A-Za-z_][A-Za-z0-9_.:-]*\)$")
VOID_HTML = {"br"}


class SanitizationError(ValueError):
    pass


def _sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _decode_input(data: bytes) -> str:
    if len(data) > LIMITS["max_input_bytes"]:
        raise SanitizationError("input exceeds sanitizer byte limit")
    try:
        text = data.decode("utf-8", errors="strict")
    except UnicodeDecodeError as exc:
        raise SanitizationError("input must be valid UTF-8") from exc
    if "\x00" in text:
        raise SanitizationError("NUL is forbidden")
    return text


def _local_name(name: str) -> str:
    if name.startswith("{"):
        return name.split("}", 1)[1]
    return name


def _namespace(name: str) -> str | None:
    if name.startswith("{"):
        return name[1:].split("}", 1)[0]
    return None


def _validate_svg_attribute(name: str, value: str) -> tuple[str, str]:
    ns = _namespace(name)
    local = _local_name(name)

    if ns not in (None, XLINK_NS):
        raise SanitizationError(f"SVG attribute namespace is not allowed: {ns}")
    if local.lower().startswith("on"):
        raise SanitizationError(f"SVG event attribute is forbidden: {local}")
    if local == "style":
        raise SanitizationError("SVG style attribute is forbidden in V1")
    if local not in set(POLICY["svg"]["allowed_attributes"]):
        raise SanitizationError(f"SVG attribute is not allowed: {local}")

    if local == "href":
        if not LOCAL_REF_RE.fullmatch(value):
            raise SanitizationError("SVG href must be a local fragment")
        return "href", value

    if local in {"clip-path", "mask"}:
        if value != "none" and not LOCAL_URL_RE.fullmatch(value):
            raise SanitizationError(f"SVG {local} may reference local fragments only")

    if local in {"fill", "stroke"}:
        lowered = value.strip().lower()
        if "url(" in lowered and not LOCAL_URL_RE.fullmatch(value.strip()):
            raise SanitizationError(f"SVG {local} may reference local fragments only")
        if "javascript:" in lowered or "data:" in lowered or "http:" in lowered or "https:" in lowered:
            raise SanitizationError(f"SVG {local} contains a forbidden URL scheme")

    return local, value


def _serialize_svg_element(element: ET.Element, state: dict, depth: int = 1) -> str:
    if depth > LIMITS["max_depth"]:
        raise SanitizationError("SVG exceeds sanitizer depth limit")

    ns = _namespace(element.tag)
    tag = _local_name(element.tag)
    if ns not in (None, SVG_NS):
        raise SanitizationError(f"SVG element namespace is not allowed: {ns}")
    if tag not in set(POLICY["svg"]["allowed_tags"]):
        raise SanitizationError(f"SVG element is not allowed: {tag}")

    state["nodes"] += 1
    if state["nodes"] > LIMITS["max_nodes"]:
        raise SanitizationError("SVG exceeds sanitizer node limit")

    attrs = {}
    for raw_name, raw_value in element.attrib.items():
        name, value = _validate_svg_attribute(raw_name, raw_value)
        if name in attrs:
            raise SanitizationError(f"duplicate SVG attribute after normalization: {name}")
        attrs[name] = value

    if tag == "svg" and depth == 1:
        attrs["xmlns"] = SVG_NS

    attr_text = "".join(
        f' {name}="{html.escape(value, quote=True)}"'
        for name, value in sorted(attrs.items())
    )

    text = element.text or ""
    state["text_chars"] += len(text)
    if state["text_chars"] > LIMITS["max_text_chars"]:
        raise SanitizationError("SVG exceeds sanitizer text limit")

    children = list(element)
    if not children and not text:
        body = f"<{tag}{attr_text}/>"
    else:
        inner = html.escape(text, quote=False)
        for child in children:
            inner += _serialize_svg_element(child, state, depth + 1)
            tail = child.tail or ""
            state["text_chars"] += len(tail)
            if state["text_chars"] > LIMITS["max_text_chars"]:
                raise SanitizationError("SVG exceeds sanitizer text limit")
            inner += html.escape(tail, quote=False)
        body = f"<{tag}{attr_text}>{inner}</{tag}>"
    return body


def sanitize_svg(data: bytes) -> tuple[bytes, dict]:
    text = _decode_input(data)
    probe = text.lstrip()
    if probe.startswith("<?xml"):
        end = probe.find("?>")
        if end < 0:
            raise SanitizationError("malformed XML declaration")
        probe = probe[end + 2 :].lstrip()
    if "<!" in probe or "<?" in probe:
        raise SanitizationError("DTD/entity/processing-instruction syntax is forbidden")

    try:
        root = ET.fromstring(text)
    except ET.ParseError as exc:
        raise SanitizationError("malformed SVG") from exc
    if _local_name(root.tag) != "svg":
        raise SanitizationError("SVG root element is required")

    state = {"nodes": 0, "text_chars": 0}
    sanitized = _serialize_svg_element(root, state).encode("utf-8")
    receipt = _receipt("svg", data, sanitized, state)
    return sanitized, receipt


class StrictHTMLSanitizer(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.allowed_tags = set(POLICY["html"]["allowed_tags"])
        self.global_attrs = set(POLICY["html"]["global_attributes"])
        self.cell_attrs = set(POLICY["html"]["table_cell_attributes"])
        self.stack: list[str] = []
        self.parts: list[str] = []
        self.nodes = 0
        self.text_chars = 0

    def _validate_attrs(self, tag: str, attrs):
        normalized = {}
        allowed = set(self.global_attrs)
        if tag in {"td", "th"}:
            allowed |= self.cell_attrs
        for name, value in attrs:
            name = name.lower()
            value = "" if value is None else value
            if name.startswith("on"):
                raise SanitizationError(f"HTML event attribute is forbidden: {name}")
            if name in {
                "style", "href", "src", "srcset", "action", "formaction",
                "poster", "data", "background", "ping"
            }:
                raise SanitizationError(f"HTML active/network attribute is forbidden: {name}")
            if name not in allowed and not name.startswith("aria-"):
                raise SanitizationError(f"HTML attribute is not allowed: {name}")
            if name in normalized:
                raise SanitizationError(f"duplicate HTML attribute: {name}")
            normalized[name] = value
        return normalized

    def _open(self, tag: str, attrs, self_closing: bool) -> None:
        tag = tag.lower()
        if tag not in self.allowed_tags:
            raise SanitizationError(f"HTML element is not allowed: {tag}")
        self.nodes += 1
        if self.nodes > LIMITS["max_nodes"]:
            raise SanitizationError("HTML exceeds sanitizer node limit")
        normalized = self._validate_attrs(tag, attrs)
        attr_text = "".join(
            f' {name}="{html.escape(value, quote=True)}"'
            for name, value in sorted(normalized.items())
        )
        if tag in VOID_HTML or self_closing:
            self.parts.append(f"<{tag}{attr_text}>")
            return
        if len(self.stack) + 1 > LIMITS["max_depth"]:
            raise SanitizationError("HTML exceeds sanitizer depth limit")
        self.parts.append(f"<{tag}{attr_text}>")
        self.stack.append(tag)

    def handle_starttag(self, tag, attrs):
        self._open(tag, attrs, False)

    def handle_startendtag(self, tag, attrs):
        self._open(tag, attrs, True)

    def handle_endtag(self, tag):
        tag = tag.lower()
        if tag in VOID_HTML:
            raise SanitizationError(f"void HTML element must not have an end tag: {tag}")
        if not self.stack or self.stack[-1] != tag:
            raise SanitizationError(f"malformed HTML close tag: {tag}")
        self.stack.pop()
        self.parts.append(f"</{tag}>")

    def handle_data(self, data):
        self.text_chars += len(data)
        if self.text_chars > LIMITS["max_text_chars"]:
            raise SanitizationError("HTML exceeds sanitizer text limit")
        self.parts.append(html.escape(data, quote=False))

    def handle_comment(self, data):
        # Comments are dropped deterministically.
        return

    def handle_decl(self, decl):
        raise SanitizationError("HTML declarations/DOCTYPE are forbidden")

    def handle_pi(self, data):
        raise SanitizationError("HTML processing instructions are forbidden")

    def unknown_decl(self, data):
        raise SanitizationError("unknown HTML declaration is forbidden")

    def finish(self) -> str:
        if self.stack:
            raise SanitizationError("malformed HTML: unclosed element")
        return "".join(self.parts)


def sanitize_html(data: bytes) -> tuple[bytes, dict]:
    text = _decode_input(data)
    parser = StrictHTMLSanitizer()
    try:
        parser.feed(text)
        parser.close()
        sanitized_text = parser.finish()
    except SanitizationError:
        raise
    except Exception as exc:
        raise SanitizationError("malformed HTML") from exc
    sanitized = sanitized_text.encode("utf-8")
    receipt = _receipt(
        "html",
        data,
        sanitized,
        {"nodes": parser.nodes, "text_chars": parser.text_chars},
    )
    return sanitized, receipt


def _receipt(kind: str, original: bytes, sanitized: bytes, state: dict) -> dict:
    return {
        "policy_version": POLICY["policy_version"],
        "kind": kind,
        "input_sha256": _sha256(original),
        "output_sha256": _sha256(sanitized),
        "input_bytes": len(original),
        "output_bytes": len(sanitized),
        "nodes": state["nodes"],
        "text_chars": state["text_chars"],
        "network_fetch_allowed": False,
        "active_content_allowed": False,
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--kind", choices=("svg", "html"), required=True)
    parser.add_argument("input")
    parser.add_argument("output")
    parser.add_argument("--receipt", required=True)
    args = parser.parse_args(argv)

    source = Path(args.input).read_bytes()
    sanitized, receipt = (
        sanitize_svg(source) if args.kind == "svg" else sanitize_html(source)
    )
    Path(args.output).write_bytes(sanitized)
    Path(args.receipt).write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except SanitizationError as exc:
        print(f"sanitize failed closed: {exc}", file=sys.stderr)
        raise SystemExit(2)
