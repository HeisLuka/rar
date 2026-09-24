# CLOUD-QUOTA-01 — atomic reservation and protected semantic headroom V1

Quota is enforced at admission, not after accepted work. Every bounded resource uses an idempotent reservation identity.

Work classes are separated into interactive semantic work, user export work and rebuildable background work. Export/background may consume shared capacity only. Interactive work may consume remaining shared capacity plus a protected semantic-write headroom.

Once a semantic mutation is durably accepted, quota enforcement must not silently drop or roll it back. When protected headroom is exhausted, the next semantic reservation fails before mutation with a typed reason.

Reservations have lease generations. Exact retry is idempotent, changed amount/class conflicts, release is exact-once, and stale expiry cannot release a renewed lease.

This is a public reference contract. Numeric tier limits, distributed atomic counters, regional ownership and billing/provider cost policy remain downstream.
