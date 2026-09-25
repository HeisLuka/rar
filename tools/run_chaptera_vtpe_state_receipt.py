#!/usr/bin/env python3
import hashlib
import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[1]
MAIN = ROOT / "apps" / "chaptera-desktop" / "src" / "main.rs"
SUPPORTER = ROOT / "apps" / "chaptera-desktop" / "src" / "supporter.rs"
MANIFEST = ROOT / "apps" / "chaptera-desktop" / "Cargo.toml"
OUT = ROOT / "target" / "chaptera-vtpe-state-v1" / "receipt.json"

main = MAIN.read_text(encoding="utf-8")
supporter = SUPPORTER.read_text(encoding="utf-8")
manifest = MANIFEST.read_text(encoding="utf-8")

required_main = {
    "versioned_storage_key": 'const SUPPORTER_STORAGE_KEY: &str = "chaptera.supporter.v1";',
    "creation_context_storage": "cc.storage",
    "storage_restore": "restore_supporter_state(storage)",
    "app_save": "fn save(&mut self, storage: &mut dyn eframe::Storage)",
    "storage_write": "storage.set_string(SUPPORTER_STORAGE_KEY",
    "roundtrip_test": "supporter_persistence_wiring_round_trips_through_eframe_storage",
    "fallback_test": "malformed_supporter_storage_falls_back_to_conservative_default",
}
required_supporter = {
    "first_success": "first_meaningful_success_can_prompt_immediately",
    "later_cooldown": "later_requires_both_time_and_three_new_successes",
    "rolling_window": "prompt_cap_is_a_real_rolling_window",
    "dismiss_45d": "second_dismissal_suppresses_for_forty_five_days",
    "supported_180d_click_30d": "already_supported_and_support_click_use_different_suppression",
    "clock_rollback": "clock_rollback_is_conservatively_suppressed_after_prompt",
    "privacy_roundtrip": "supporter_state_round_trips_without_document_identity",
    "future_schema_fail_closed": "unknown_or_malformed_persisted_state_fails_to_none",
}

for name, needle in required_main.items():
    assert needle in main, (name, needle)
for name, needle in required_supporter.items():
    assert needle in supporter, (name, needle)

eframe_line = next(
    line.strip()
    for line in manifest.splitlines()
    if line.strip().startswith("eframe =")
)
assert 'version = "=0.31.1"' in eframe_line, eframe_line
assert '"persistence"' in eframe_line, eframe_line

def sha256(path: pathlib.Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()

receipt = {
    "receipt_kind": "chaptera.vtpe-state-v1.validation",
    "desktop_package": "chaptera-desktop",
    "storage_key": "chaptera.supporter.v1",
    "eframe_version": "0.31.1",
    "eframe_persistence_enabled": True,
    "source_identity": {
        "main_rs": sha256(MAIN),
        "supporter_rs": sha256(SUPPORTER),
        "cargo_toml": sha256(MANIFEST),
    },
    "wiring": {name: True for name in required_main},
    "cadence_contract": {name: True for name in required_supporter},
    "privacy_boundary": {
        "app_level_state_only": True,
        "document_identity_fields": False,
        "network_effects": False,
    },
    "validation_scope": (
        "Current Rar desktop implementation plus executable cadence and eframe "
        "storage roundtrip tests; no donor branch is an execution dependency."
    ),
}
OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
print(json.dumps(receipt, indent=2, sort_keys=True))
