# CI-EFFICIENCY-BASELINE-01 — Rar Actions cost census

This gate measures real public GitHub Actions metadata before changing the CI graph.

The receipt covers a bounded set of recent completed Rar runs and records:
- queue delay and run elapsed time;
- summed job duration / runner-minutes;
- an approximate longest-job critical-path proxy;
- runner labels;
- step durations grouped into checkout, runtime setup, dependency install, build, test/benchmark, artifact I/O and other work;
- artifact counts and first-page bytes;
- workflow + head-SHA rerun grouping;
- repeated normalized step names across workflows.

The census deliberately does **not** infer dollar spend, billing-minute rounding, cache hits or true dependency-graph critical path when GitHub's normal run/job metadata does not prove them. Those values remain explicit unknowns.

Repeated setup/action work is only a candidate optimization. Independent public-boundary, packet, provenance and correctness guards must not be merged or removed without a separate semantic-equivalence review.
