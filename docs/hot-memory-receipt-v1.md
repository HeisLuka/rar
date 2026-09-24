# CLOUD-HOT-MEMORY-01 — source-neutral residency receipt

This contract prevents different measurements from being collapsed into one vague “MB/document” number.

The canonical server memory model is reported as independent layers:

- **L1** — canonical hot core: AuthoringModel / EditorSession and current semantic truth required to accept the next operation;
- **L2** — semantic acceleration: indexes, dependency maps, replay/dirty metadata;
- **L3** — layout/shaping caches;
- **L4** — Scene/render caches;
- **L5** — decoded images/fonts/thumbnails and other opportunistic resources;
- **L6** — raw parser/import state, which should not stay resident merely because an editor is open.

Every mode reports COLD baseline RSS, WARM ready RSS, HOT steady RSS and COLD→WARM peak, plus an explicit staged eviction witness in the required order **L5 → L4 → L3 → L2**. It also records activation scenarios at different snapshot lags with tail bytes, bytes read, replay CPU, activation wall time and peak RSS.

Two workload modes are mandatory: `semantic_only_server` and `server_layout`. This lets later capacity work measure the incremental cost of server-derived layout instead of assuming it.

Layer metrics record their measurement method because process RSS deltas, allocator attribution and object-graph accounting are not interchangeable.

The checked-in fixture is deliberately synthetic. It proves only schema and invariants and has `capacity_decision_allowed=false`. A real capacity/sharding decision requires a sanitized `real_pub_source_free` receipt from the authorized canonical runtime.
