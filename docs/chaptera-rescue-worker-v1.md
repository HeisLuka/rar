# Chaptera Rescue isolated recovery worker protocol v1

This boundary separates the Chaptera Rescue UI process from malformed/untrusted and potentially long-running recovery execution.

The public Rar worker shell **does not contain a recovery engine**. Until an authorized executor is connected, a valid admitted job ends as `executor_unavailable`, with source identity re-verified and no recovery artifact or success claim.

## Process contract

`chaptera-rescue.exe` / an orchestrator owns user intent and source admission. It spawns a short-lived `chaptera-recovery-worker.exe`, sends exactly one `chaptera.rescue-worker-job.v1` object on stdin, consumes JSONL events on stdout, and receives one terminal `chaptera.rescue-worker-result.v1`.

stderr is human diagnostics only.

The worker never mutates the source. An eventual executor may write only under the admitted job directory. A successful executor must return a producer receipt that can then pass the already-merged `RESCUE-RECEIPT-CONSUMER-01` boundary.

## Limits

Every job carries explicit ceilings for wall time, CPU time, memory, output bytes and artifact count. The V1 public shell validates those limits but intentionally does not claim OS-level CPU/memory enforcement yet because no real executor is connected.

The next implementation slice must put the actual executor under a platform process-limit mechanism (Windows Job Object for Windows product acceptance) and turn limit breach into typed `timed_out` or `resource_limited`, never a Rescue success.

## Progress

Events report named phases, not invented percentages:

`admission → source_verification → executor → result_validation → finished`.

## Privacy / durable evidence

The worker job is local IPC and may contain a local source path. Durable/public receipts must not. The terminal result identifies the immutable source by SHA-256 and, after real execution exists, may identify a producer receipt only by relative job-output path + SHA-256. The public Rescue consumer remains the authority for turning that producer receipt into a customer-facing product outcome.
