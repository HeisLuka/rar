# PUB corpus acquisition baseline

This directory is the minimal public-safe acquisition substrate for the active Rar GitHub Actions laboratory.

It is intentionally a bounded port of historically validated corpus tooling from `HeisLuka/pub-rs` and `HeisLuka/yab`. Those repositories remain provenance/reference implementations; new hosted execution happens here.

## Contract

The pipeline accepts public unauthenticated locator rows and treats downloaded documents as inert bytes only.

It preserves:

- source and resolved locator provenance;
- exact byte size and SHA-256;
- conservative CFB / Publisher classification;
- exact-SHA duplicate relationships;
- separate natural, negative, quarantine, source-container and truncated-partial states;
- archive parent/member provenance;
- Common Crawl completeness evidence where applicable.

Downloaded Publisher documents are never executed or opened in Microsoft Office by this workflow.

## Smoke fixture

`smoke_seed.csv` pins Apache POI's public `Sample.pub` at commit
`74700b5692fd0449031ac6a3c78aec1155d144d5`.

Expected identity:

- size: `72192`
- SHA-256: `6fefdef46b87c767150878dc384549cb2d2ec2ac54de25f8ddb3a5628301107e`

The smoke workflow fails closed if the fetched bytes drift or no longer classify as Publisher CFB.

## Public Actions boundary

Everything in this repository, Actions logs, and uploaded artifacts must be safe for public disclosure. Do not add private/customer PUB files, proprietary binaries, credentials, private URLs, or replayable signed URLs.
