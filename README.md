# rar — disposable public execution workspace

> **Agents and automation:** read [`AGENTS.md`](AGENTS.md) before creating branches, pushing commits, opening PRs, or changing workflows. It is the mandatory execution/CI contract for this repository.

`HeisLuka/rar` is the active public-safe GitHub execution/validation workspace for the PUB / Chaptera program from 2026-09-24 onward.

## Boundary

- New public-safe GitHub Actions workflows, bounded experiments, CI validation, schemas and sanitized receipts run here.
- `HeisLuka/miy` is frozen provenance: existing commits, PRs, Actions runs and receipts remain valid evidence, but no new task execution should be started there.
- Historical `yab` / `pub-rs` repositories are provenance only unless a task explicitly names a local/private checkout.
- Private Chaptera source, customer documents, licensed/proprietary binaries, secrets and private runtime state must not be copied here.
- Local/private producers may emit only source-free, schema-approved receipts into this repository.
- Port only the minimal dependency closure required by an active task. Do not mirror the accumulated `miy` repository wholesale.

## Migration rule

For an active task that previously referenced `miy`:
1. keep old `miy` links/hashes as provenance;
2. create new execution branches/runs in `rar`;
3. copy only the minimal public-safe validator/tool/fixture slice needed by that task;
4. record the `rar` branch/run/receipt back in Notion.

