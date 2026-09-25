# Chaptera Editor local portable-package producer v1

This document defines the public-safe handoff for `EDITOR-LOCAL-PRODUCER-01`.

The real Chaptera desktop binary and real PUB fixture may remain private/local. The public Rar workspace owns the receipt schema, validation, and the small builder that turns a local runtime proof into a source-free receipt.

## Stage the portable ZIP

If the local Chaptera Editor binary already exists, package it from the active Rar checkout instead of hand-building a ZIP:

```powershell
python tools/package_editor_live_trial.py `
  --editor-exe "D:\path\to\Chaptera-Editor.exe" `
  --output-zip "D:\path\to\Chaptera-Editor.zip" `
  --manifest "D:\path\to\Chaptera-Editor.package.json"
```

The packager is deliberately narrow: it writes only the Editor executable and the canonical trial README, uses stable ZIP metadata, rejects non-PE input and unsafe entry names, and produces deterministic artifact hashes. It does not claim that the supplied executable is product-authoritative; the real runtime producer still has to prove the editor loop.

## Build the receipt

Run from the active `HeisLuka/rar` checkout on the authorized Windows machine:

```powershell
python tools/build_editor_live_trial_package_receipt.py `
  --zip "D:\\path\\to\\Chaptera-Editor.zip" `
  --binary-entry "Chaptera-Editor.exe" `
  --readme-entry "TRIAL-README.md" `
  --chaptera-version "0.1.0-local" `
  --fixture-kind real_pub_sanitized `
  --output "D:\\path\\to\\package-receipt.json" `
  -- <local-producer-command>
```

Then validate:

```powershell
python tools/validate_editor_live_trial_package_receipt.py "D:\\path\\to\\package-receipt.json"
```

## What the builder proves itself

The builder does not trust the producer for package identity. It directly:

- hashes the portable ZIP;
- hashes the exact executable entry inside the ZIP;
- requires the executable entry to be a Windows `.exe`;
- requires `TRIAL-README.md` (or the explicitly selected README entry);
- requires the README contract marker `chaptera.editor-live-trial-readme.v1`;
- rejects obvious installer payloads (`.msi`, `.msix`, `.appx`, bundles);
- rejects bundling any `.pub` file inside the portable ZIP;
- emits no local path, source filename, source hash, document text, or customer identity.

## Local producer protocol

The command after `--` receives one JSON object on stdin:

```json
{
  "action": "editor_live_trial_package_smoke",
  "fixture_kind": "real_pub_sanitized",
  "zip_path": "<local absolute path>",
  "binary_entry": "Chaptera-Editor.exe",
  "readme_entry": "TRIAL-README.md"
}
```

`zip_path` is private local execution input. It is never copied into the public receipt.

The producer must emit exactly this JSON shape on stdout:

```json
{
  "fixture_kind": "real_pub_sanitized",
  "source_sha256_before": "<64 lowercase hex>",
  "source_sha256_after": "<same 64 lowercase hex>",
  "reader_only": false,
  "editor_controls_enabled": true,
  "native_save_pub_claimed": false,
  "launch_without_dev_toolchain": true,
  "real_pub_opened": true,
  "supported_story_edited": true,
  "supported_object_dragged": true,
  "undo_redo_verified": true,
  "editor_project_saved": true,
  "close_reopen_reproduced_state": true,
  "unsupported_actions_fail_closed": true
}
```

The builder fails closed if:

- source identity changed;
- the packaged binary is reader-only;
- Editor controls are unavailable;
- native Save PUB is claimed;
- any required runtime-smoke arm is incomplete;
- the ZIP/README boundary is wrong.

The two source hashes are consumed only locally to prove immutability and are discarded before receipt serialization.

## Scope

This builder closes only the **portable package receipt** portion of `EDITOR-LOCAL-PRODUCER-01`.

It does not waive the separate projection-instance admission required before imported-PUB Resize/ReplaceImage evidence can be counted as customer-safe product closure. It also does not waive the later AUTH-WRAP stop gate.

No private Chaptera source, customer PUB, licensed binary, source path, or runtime secret belongs in Git history.
