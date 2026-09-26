#!/usr/bin/env python3
from dataclasses import asdict
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "services" / "editor-api"))

from resolved_text_caret_map_v1 import (
    ResolvedClusterV1,
    ResolvedLineFragmentV1,
    ResolvedTextCaretMapError,
    build_resolved_text_caret_map_v1,
    caret_map_to_dict_v1,
)
from story_edit_domain_v1 import (
    StoryEditDomainError,
    derive_story_edit_domain_v1,
)
from text_ingress_v1 import normalize_external_text_v1
from text_keyboard_policy_v1 import (
    TextKeyboardPolicyError,
    apply_text_keyboard_policy_v1,
    grapheme_boundaries_v1,
)
from text_selection_state_v1 import build_text_selection_state_v1

MANIFEST = ROOT / "crates" / "chaptera-text-input-adapter" / "Cargo.toml"
REVISION = "revision:golden"


def rust(request):
    proc = subprocess.run(
        [
            "cargo", "run", "--quiet",
            "--manifest-path", str(MANIFEST),
            "--example", "conformance_probe",
        ],
        cwd=ROOT,
        input=json.dumps(request, ensure_ascii=False),
        text=True,
        capture_output=True,
        check=False,
    )
    if proc.returncode != 0:
        print("Rust conformance probe failed", file=sys.stderr)
        print(proc.stderr, file=sys.stderr)
        print(json.dumps(request, ensure_ascii=False, indent=2), file=sys.stderr)
        raise SystemExit(proc.returncode)
    return json.loads(proc.stdout)


def check(name, expected, request):
    actual = rust(request)
    if actual != expected:
        print(name, "mismatch", file=sys.stderr)
        print(json.dumps({"python": expected, "rust": actual}, indent=2, ensure_ascii=False), file=sys.stderr)
        raise SystemExit(1)
    print(name, "PASS")


def domain_case(name, story_id, text, provenance):
    try:
        value = derive_story_edit_domain_v1(
            story_id=story_id,
            story_text=text,
            provenance=provenance,
        )
        expected = {"ok": value.to_dict()}
    except StoryEditDomainError as error:
        expected = {"error_code": error.code}
    check(name, expected, {
        "op": "domain",
        "story_id": story_id,
        "story_text": text,
        "provenance": provenance,
    })


def ingress_case(name, text):
    value = normalize_external_text_v1(text)
    check(name, {"ok": value.to_dict()}, {"op": "ingress", "input_text": text})


def grapheme_case(name, text):
    check(
        name,
        {"ok": {"boundaries": list(grapheme_boundaries_v1(text))}},
        {"op": "graphemes", "text": text},
    )


def line(story_id, clusters):
    return ResolvedLineFragmentV1(
        story_id=story_id,
        page_id="page:1",
        frame_id="frame:1",
        line_id="line:1",
        flow_ordinal=0,
        previous_line_id=None,
        next_line_id=None,
        page_y_top_emu=0,
        page_y_bottom_emu=20,
        frame_y_top_emu=0,
        frame_y_bottom_emu=20,
        clusters=tuple(clusters),
    )


def scalar_clusters(count):
    return [
        ResolvedClusterV1(i, i + 1, i * 10, (i + 1) * 10, i * 10, (i + 1) * 10)
        for i in range(count)
    ]


def keyboard_case(name, command, story_text, provenance, anchor, focus, clusters):
    story_id = "story:keyboard"
    domain = derive_story_edit_domain_v1(
        story_id=story_id,
        story_text=story_text,
        provenance=provenance,
    )
    selection = build_text_selection_state_v1(
        domain=domain,
        revision_id=REVISION,
        anchor_scalar=anchor,
        focus_scalar=focus,
    )
    caret_map = build_resolved_text_caret_map_v1(
        layout_revision_id="layout:golden",
        story_id=story_id,
        story_scalar_len=len(story_text),
        lines=(line(story_id, clusters),),
    )
    try:
        value = apply_text_keyboard_policy_v1(
            command=command,
            story_text=story_text,
            domain=domain,
            selection=selection,
            caret_map=caret_map,
            expected_revision_id=REVISION,
        )
        expected = {"ok": asdict(value)}
    except (TextKeyboardPolicyError, ResolvedTextCaretMapError) as error:
        expected = {"error_code": error.code}
    check(name, expected, {
        "op": "keyboard",
        "command": command,
        "story_text": story_text,
        "domain": domain.to_dict(),
        "selection": asdict(selection),
        "caret_map": caret_map_to_dict_v1(caret_map),
        "expected_revision_id": REVISION,
    })


def main():
    domain_case("domain_chaptera", "story:a", "A\r", "chaptera_created")
    domain_case("domain_mature", "story:q", "ABC\r", "imported_mature_quill_terminal_cr")
    domain_case("domain_unknown", "story:u", "ABC\r", "imported_unknown")

    ingress_case("ingress_crlf_lf_cr", "A\r\nB\nC\rD")
    ingress_case("ingress_no_unicode_normalization", "e\u0301")

    grapheme_case("grapheme_ascii", "abc")
    grapheme_case("grapheme_supplementary", "A😀B")
    grapheme_case("grapheme_combining", "a\u0301b")
    grapheme_case("grapheme_zwj_family", "👨‍👩‍👧‍👦")
    grapheme_case("grapheme_regional_pairs", "🇺🇸🇨🇦")

    keyboard_case(
        "keyboard_nonempty_delete",
        "delete_backward",
        "abc",
        "chaptera_created",
        0,
        2,
        scalar_clusters(3),
    )
    keyboard_case(
        "keyboard_protected_end_noop",
        "delete_forward",
        "ABC\r",
        "imported_mature_quill_terminal_cr",
        3,
        3,
        [ResolvedClusterV1(0, 3, 0, 30, 0, 30)],
    )
    keyboard_case(
        "keyboard_fi_internal_caret_unsupported",
        "move_next",
        "fi",
        "chaptera_created",
        0,
        0,
        [ResolvedClusterV1(0, 2, 0, 20, 0, 20)],
    )


if __name__ == "__main__":
    main()
