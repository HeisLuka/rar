# CLOUD-PROJECTIONS-01 — Search, Recent and thumbnail projections V1

This contract materializes the product semantics already selected by the
`rar#37` modelcheck. Projection state is disposable/read-optimized state;
current lifecycle, authorization and canonical revision identity remain
authoritative.

## Search

The search index only selects candidates. Every returned hit is joined to the
current document directory state and filtered by current lifecycle + current
authorization. Display name/workspace comes from current metadata, not from the
possibly stale index copy. A stale indexed revision is surfaced as `stale`;
ordinary result opening targets the current document revision.

This intentionally does **not** pretend recall is fresh: a renamed document may
still match the old term and fail to match the new term until reindex.

## Recent

Recent is per-principal activity state. Open/view/edit activity can reorder
documents without creating document semantic revisions. The read path filters
every row against current access/lifecycle.

Recent is not rebuildable from RevisionStream alone; retained principal activity
events/state are an explicit additional source.

## Thumbnails

Artifacts are immutable and keyed by:

- DocumentId
- RevisionId
- LayoutEnvironmentId
- Scene/projection protocol version
- renderer/thumbnailer version

There is no mutable "last completion wins" pointer. The selector first asks for
the exact current revision/environment/version key. If absent, a caller may
provide canonically ordered older revision candidates; any selected fallback is
labeled `stale`. Otherwise the result is `missing`.

## Evidence boundary

This is a public executable service-reference contract. It does not choose a
search backend, tokenizer, privacy/redaction policy, freshness SLO, thumbnail
format/tiers, activity retention policy or deployed reconciliation/DR design.
