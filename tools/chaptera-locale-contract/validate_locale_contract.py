#!/usr/bin/env python3
"""Public-safe CHAPTERA-VTPE-LOCALE-01 validation receipt.

This is a source-neutral contract probe. It does not import private Chaptera
implementation code and does not claim private desktop integration. It proves
Windows locale acquisition and the bounded locale->market/preference contract.
"""

from __future__ import annotations

import argparse
import ast
import ctypes
import json
import os
import pathlib
import platform
import re
import sys

SCHEMA = "chaptera.vtpe-locale-validation.v1"
LOCALE_NAME_MAX_LENGTH = 85
FORBIDDEN_IMPORT_ROOTS = {
    "socket",
    "urllib",
    "requests",
    "http",
    "ipaddress",
    "geopy",
    "aiohttp",
}


def normalize_locale(value: str | None) -> str:
    raw = (value or "").strip().replace("_", "-").lower()
    return re.split(r"[.@]", raw, maxsplit=1)[0]


def market_profile(value: str | None) -> str:
    normalized = normalize_locale(value)
    return {
        "en-us": "US",
        "en-gb": "UK",
        "ru-ru": "Russia",
    }.get(normalized, "NeutralEnglish")


def resolve_locale_source(
    override_locale: str | None,
    os_locale: str | None,
    lang: str | None,
) -> str | None:
    for candidate in (override_locale, os_locale, lang):
        if candidate is not None and candidate.strip():
            return candidate.strip()
    return None


def windows_user_locale() -> str:
    if os.name != "nt":
        raise RuntimeError("real Windows locale probe requires os.name == 'nt'")

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    fn = kernel32.GetUserDefaultLocaleName
    fn.argtypes = [ctypes.POINTER(ctypes.c_wchar), ctypes.c_int]
    fn.restype = ctypes.c_int

    buffer = ctypes.create_unicode_buffer(LOCALE_NAME_MAX_LENGTH)
    count = fn(buffer, len(buffer))
    if count <= 1:
        raise OSError(ctypes.get_last_error(), "GetUserDefaultLocaleName failed")

    value = buffer.value
    if not value or "\x00" in value or len(value) >= LOCALE_NAME_MAX_LENGTH:
        raise RuntimeError(f"unexpected Windows locale result: {value!r}")
    return value


def imported_roots() -> list[str]:
    source = pathlib.Path(__file__).read_text(encoding="utf-8")
    tree = ast.parse(source)
    roots: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            roots.update(alias.name.split(".", 1)[0] for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module:
            roots.add(node.module.split(".", 1)[0])
    return sorted(roots)


def check_equal(name: str, actual, expected, checks: list[dict]) -> None:
    passed = actual == expected
    checks.append(
        {
            "name": name,
            "passed": passed,
            "actual": actual,
            "expected": expected,
        }
    )
    if not passed:
        raise AssertionError(f"{name}: expected {expected!r}, got {actual!r}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args()

    checks: list[dict] = []

    market_cases = [
        ("en-US", "US"),
        ("EN_us", "US"),
        ("en_US.UTF-8", "US"),
        ("en-US@custom", "US"),
        ("en-GB", "UK"),
        ("en_GB.UTF-8", "UK"),
        ("ru-RU", "Russia"),
        ("ru_RU.UTF-8", "Russia"),
        ("en-CA", "NeutralEnglish"),
        ("ru-UA", "NeutralEnglish"),
        ("fr-FR", "NeutralEnglish"),
        ("en", "NeutralEnglish"),
        ("ru", "NeutralEnglish"),
        ("en-US-x-private", "NeutralEnglish"),
        ("", "NeutralEnglish"),
        ("   ", "NeutralEnglish"),
    ]
    for raw, expected in market_cases:
        check_equal(
            f"market_profile:{raw!r}",
            market_profile(raw),
            expected,
            checks,
        )

    precedence_cases = [
        {
            "name": "override_wins",
            "override": "ru-RU",
            "os": "en-US",
            "lang": "en_GB.UTF-8",
            "source": "ru-RU",
            "profile": "Russia",
        },
        {
            "name": "os_wins_without_override",
            "override": None,
            "os": "en-GB",
            "lang": "ru_RU.UTF-8",
            "source": "en-GB",
            "profile": "UK",
        },
        {
            "name": "lang_is_fallback",
            "override": None,
            "os": None,
            "lang": "en_US.UTF-8",
            "source": "en_US.UTF-8",
            "profile": "US",
        },
        {
            "name": "non_target_override_still_wins_and_fails_closed",
            "override": "fr-FR",
            "os": "en-US",
            "lang": "ru_RU.UTF-8",
            "source": "fr-FR",
            "profile": "NeutralEnglish",
        },
        {
            "name": "non_target_os_still_wins_over_lang",
            "override": None,
            "os": "de-DE",
            "lang": "ru_RU.UTF-8",
            "source": "de-DE",
            "profile": "NeutralEnglish",
        },
        {
            "name": "empty_sources",
            "override": "  ",
            "os": "",
            "lang": None,
            "source": None,
            "profile": "NeutralEnglish",
        },
    ]
    for case in precedence_cases:
        source = resolve_locale_source(case["override"], case["os"], case["lang"])
        check_equal(
            f"precedence_source:{case['name']}",
            source,
            case["source"],
            checks,
        )
        check_equal(
            f"precedence_profile:{case['name']}",
            market_profile(source),
            case["profile"],
            checks,
        )

    roots = imported_roots()
    forbidden_imports = sorted(FORBIDDEN_IMPORT_ROOTS.intersection(roots))
    check_equal("forbidden_imports", forbidden_imports, [], checks)

    actual_locale = windows_user_locale()
    actual_profile = market_profile(actual_locale)

    receipt = {
        "schema": SCHEMA,
        "claim": "public_windows_locale_contract_validation_not_private_product_integration",
        "result": "pass",
        "platform": {
            "system": platform.system(),
            "release": platform.release(),
            "python": platform.python_version(),
            "os_name": os.name,
        },
        "actual_windows_user_locale": {
            "api": "GetUserDefaultLocaleName",
            "raw": actual_locale,
            "normalized": normalize_locale(actual_locale),
            "market_profile": actual_profile,
        },
        "resolution_order": [
            "CHAPTERA_LOCALE deterministic override",
            "Windows GetUserDefaultLocaleName",
            "LANG fallback",
            "NeutralEnglish market fallback",
        ],
        "market_rule": {
            "en-us": "US",
            "en-gb": "UK",
            "ru-ru": "Russia",
            "other": "NeutralEnglish",
            "normalization": "trim; underscore-to-hyphen; lowercase; strip first .encoding or @modifier",
        },
        "boundary": {
            "private_chaptera_source_imported": False,
            "product_integration_claim": False,
            "network_lookup_used": False,
            "ip_geolocation_used": False,
            "timezone_inference_used": False,
            "keyboard_history_used": False,
            "account_identity_used": False,
        },
        "import_roots": roots,
        "checks": checks,
        "check_count": len(checks),
        "all_checks_passed": all(check["passed"] for check in checks),
    }

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(receipt, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
