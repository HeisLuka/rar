-- CLOUD-BLOB-GC-01
-- Durable physical delete fence shared by GC and blob binding creation.
--
-- A fence means a GC transaction has won the final SQLite reachability race
-- for this exact physical generation. New bindings must not attach while the
-- fence is present. Provider deletion/unknown-outcome reconciliation remains a
-- later authority step; deleted=1 is reserved for durable physical deletion.

ALTER TABLE physical_blobs
    ADD COLUMN gc_delete_fence BLOB;

ALTER TABLE physical_blobs
    ADD COLUMN gc_fenced_at_ms INTEGER
        CHECK (gc_fenced_at_ms IS NULL OR gc_fenced_at_ms >= 0);

CREATE INDEX IF NOT EXISTS physical_blobs_gc_fence_idx
    ON physical_blobs(deleted, gc_delete_fence, delete_eligible_at_ms);
