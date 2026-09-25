# Web Editor security boundary V1 — Rar integration status

This page records the active HeisLuka/rar integration state for WEB-SECURITY-01.

## Browser/source rule

After upload, raw PUB bytes are server-side source material. Browser-visible JSON must pass the source-neutral payload fence and may not expose raw bytes, CFB/source paths, parser records, carriers, stream names, source references, or byte ranges.

## Active-content rule

Browser-visible SVG/HTML must pass the deterministic CLOUD-SANITIZE-01 V1 sanitizer before delivery. Script/event/foreign executable content, document-controlled network references, DTD/entity/processing-instruction input, and unsafe HTML transport attributes fail closed.

## Resource access rule

A browser resource read requires both document-level CAP_VIEW authorization and a tenant-bound, expiring artifact grant. Application cache/resource identity remains tenant-scoped even when two tenants have identical content hashes.

## Parse isolation rule

The integration uses the existing Linux per-file worker harness from rar#143. Parse-like workers execute with RLIMIT address-space/CPU/open-file/output bounds, no_new_privs, and libseccomp network syscalls denied. This does not claim filesystem namespace/chroot confinement beyond what that harness actually provides.

## Browser shell headers

Current browser HTTP plumbing receives a restrictive CSP plus nosniff, no-referrer, same-origin resource policy, and a restrictive Permissions-Policy. External document-driven fetch remains default deny.

## Remaining closure work

WEB-SECURITY-01 is not closed by this slice. Production apps/chaptera-server currently exposes only live/ready/version and therefore does not yet provide the production upload/scene/resource route on which the full product acceptance chain can be proven. Production route wiring must consume these policies and the current SourceIngress/blob/authz ports. PNG/JPEG/font delivery must consume the separate decoder/validation receipts rather than inventing browser decoder authority here.
