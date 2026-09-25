# Chaptera Rescue isolated recovery worker protocol v1

This boundary separates the Chaptera Rescue UI process from malformed/untrusted and potentially long-running recovery execution.

The public Rar worker shell **does not contain a recovery engine**. Without privileged launch configuration, a valid admitted job still ends as `executor_unavailable`, with source identity re-verified and no recovery artifact or success claim. When an authorized external executor is configured at process launch, the worker can invoke it only inside the proven Windows Job Object boundary.

## Process contract

`chaptera-rescue.exe` / an orchestrator owns user intent and source admission. It spawns a short-lived `chaptera-recovery-worker.exe`, sends exactly one `chaptera.rescue-worker-job.v1` object on stdin, consumes JSONL events on stdout, and receives one terminal `chaptera.rescue-worker-result.v1`.

stderr is human diagnostics only.

The worker never mutates the source. An eventual executor may write only under the admitted job directory. A successful executor must return a producer receipt that can then pass the already-merged `RESCUE-RECEIPT-CONSUMER-01` boundary.

## Limits

Every job carries explicit ceilings for wall time, CPU time, memory, output bytes and artifact count. The Windows launcher creates the already-proven Job Object fence and assigns `chaptera-recovery-worker.exe` to it before the worker is allowed to start an external executor. Windows child-process inheritance keeps the executor in the same job tree because the launcher does not enable breakaway. The parent enforces wall time; the Job Object enforces per-process CPU time and memory; the worker independently rejects post-run output byte/artifact-count overflow.

An external executor is fail-closed unless both conditions hold: the worker is actually running in a Windows Job Object and the privileged launcher set `CHAPTERA_RECOVERY_FENCE_MODE=windows_job_object_v1`. The executor path/arguments are launch configuration, not fields in the untrusted recovery job packet.

## Progress

Events report named phases, not invented percentages:

`admission → source_verification → executor → result_validation → finished`.

## Privacy / durable evidence

The worker job is local IPC and may contain a local source path. Durable/public receipts must not. The terminal result identifies the immutable source by SHA-256 and, after real execution exists, may identify a producer receipt only by relative job-output path + SHA-256. The public Rescue consumer remains the authority for turning that producer receipt into a customer-facing product outcome.


## Windows Job Object acceptance slice

`RESCUE-WORKER-WIN-FENCE-01` proves the Windows product-runtime enforcement mechanism separately from the private recovery executor.

The acceptance probe creates a Job Object with:
- `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`;
- per-process memory ceiling;
- per-process CPU-time ceiling;
- parent-owned wall timeout;
- explicit cancel via `TerminateJobObject`;
- post-run output byte/artifact-count admission before any result can be treated as publishable.

Synthetic child modes prove the existing V1 status vocabulary:
`succeeded`, `timed_out`, `cancelled`, `resource_limited`, and `failed`.

This does **not** claim the real recovery executor is connected. It proves the Windows fence that the authorized executor must run under; final `RESCUE-WORKER-ISOLATION-01` closure still requires the real executor to emit the existing producer receipt contract.

## Authorized external executor seam

The public worker now has a narrow integration seam for the already-proven local/private recovery producer:

- launch-only `--executor PATH` plus optional repeated `--executor-arg VALUE`; neither is accepted from `chaptera.rescue-worker-job.v1`;
- the executable and file-valued arguments are hash-bound into a source-free `executor.id`;
- the executor receives local source/job paths only through process environment variables;
- the admitted output directory must be empty and must not contain the source;
- symbolic links in executor output are rejected;
- the fixed receipt path is `producer-receipt.json` directly under the admitted job directory;
- the worker re-hashes the source after execution and rejects any source mutation;
- output byte/artifact ceilings are checked before any success result;
- worker-side receipt admission checks exact source identity, source immutability, zero fabricated bytes, zero silent drops and the source-free privacy boundary;
- the existing `validate_rescue_recovery_receipt.py` consumer remains the authority for full producer schema/route/product outcome validation.

A worker result `status=succeeded` therefore means **execution + transport admission succeeded**, not “the document was recovered.” Recovery/product outcome comes only from the producer receipt consumer.

### Public seam acceptance without fake recovery

`chaptera-recovery-fixture-executor.exe` is a CI-only non-recovery fixture. It accepts only the public Apache POI healthy control `51318.pub` pinned to SHA-256 `3ab75a6a9196e0a51fc9b0aa759459501c71d313030c06652aabffbae0a2ab09`, verifies that it is itself inside a Windows Job Object, and emits only a `diagnostic_only` producer receipt with zero artifacts. The canonical Rescue consumer must map that receipt to `unsupported/no_safe_recovery`.

This proves the executor/process/receipt seam without inventing damaged-file recovery. Final closure of `RESCUE-WORKER-ISOLATION-01` still requires the authorized local producer from `RESCUE-LOCAL-PRODUCER-01` to run through this same fenced seam on the pinned natural damaged witness and have its existing source-free receipt pass the canonical consumer.
