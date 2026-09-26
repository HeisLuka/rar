# TUF acceptance fixtures

The signed JSON fixtures in this directory are copied from
`awslabs/tough` commit `98d8eb8b2ce63515d9b4981c938ef6453c5b5771`:

- `tough/tests/data/rotated-root`
- `tough/tests/data/expired-repository/metadata`

They are used only as deterministic interoperability/security fixtures for
Chaptera's pinned `tough 0.24.0` client boundary.

Upstream project: https://github.com/awslabs/tough
License: MIT OR Apache-2.0.

The fixtures contain public metadata/signatures only. No production Chaptera
private key material is stored here.
