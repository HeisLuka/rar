# OPT-REGRESSION-SPINE-01 — optimization evidence contract

The optimization spine standardizes evidence produced by existing benchmark tasks. It does not create a second performance queue and it does not replace task-specific product thresholds.

## Two layers

A **measurement snapshot** binds one producer receipt to an exact build identity, workload identity, runtime/hardware identity, correctness signals, evidence authority and a multidimensional metric vector.

An **optimization receipt** compares compatible baseline/candidate snapshots. Workload, runtime and producer schema must match exactly in v1; mismatches fail closed rather than being normalized away.

## Metric rules

Metrics are independent and carry units plus direction. Regression budgets are per metric. There is no aggregate performance score.

An unavailable metric is `unknown` with a reason. It is never represented as zero. A budget depending on an unknown metric is unevaluable and forces `needs_real_corpus_validation`.

Correctness/fidelity failure overrides every performance gain.

## Evidence authority

Public synthetic/product-grounded measurements may justify keeping or reverting a bounded implementation optimization, but they cannot authorize a product technology decision. Product-technology scope requires measurements whose producer marks real product corpus authority.

## First producer

`chaptera.render-bench.v1` is the first integrated producer. The adapter captures CPU compile/patch/frame/preview latency, compiled bytes and Python allocation metrics while retaining GPU latency, GPU residency and cost as explicit unknowns. Because the current Render benchmark still has `real_pub_scene_present=false`, its measurement snapshot has `technology_decision_allowed=false`.


## Copy/materialization producer

`chaptera.copy-ledger.v1` is a second integrated producer. Its adapter exposes total materialized bytes, avoidable duplicate bytes, observed allocation/event-peak counters and per-payload-class materialization/amplification metrics through the same measurement schema. Synthetic copy-ledger fixtures remain unable to authorize product technology decisions; only sanitized `real_pub_source_free` evidence can do so.
