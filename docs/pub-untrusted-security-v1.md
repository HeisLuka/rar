# PUB-UNTRUSTED-SEC-01 — shared untrusted PUB boundary V1

This slice binds the already-merged Rar worker isolation from `rar#143` to a concrete untrusted-PUB batch consumer. It deliberately does **not** create a second process sandbox.

## Execution boundary

The parent path is `tools/untrusted_pub_batch_v1.py`.

1. Recursively enumerate only regular `.pub` files with symlink following disabled.
2. Reject over-limit inputs from file metadata before worker start.
3. Launch one file at a time through the existing `tools/migration_pdf_worker_isolation.py::run_isolated_worker`.
4. The shared parent supplies process-group isolation, wall timeout, RLIMIT_AS/CPU/NOFILE/FSIZE, `no_new_privs`, syscall-level network default-deny, disposable staging and fail-closed publication.
5. `chaptera-untrusted-pub-worker` opens the authorized input and its result file, reads the PUB bytes once, then installs a second seccomp filter.
6. After that point the worker cannot open/enumerate/mutate filesystem paths, spawn/exec processes, ptrace or use process_vm. CFB inspection operates only on the in-memory bytes.
7. A malformed CFB is a typed per-file `parse_failed` result; it is not a batch crash.

The network deny list in the shared worker harness also includes socket metadata/options and `io_uring_*` bypass syscalls.

## Structural limits

The V1 worker uses the maintained workspace `cfb = 0.14` reader over `Cursor<&[u8]>`. It applies:

- input bytes: 256 MiB default;
- CFB entries: 8,192 default;
- sum of declared stream lengths: 512 MiB default;
- parent wall/CPU/address-space/open-file/output ceilings.

These are alpha safety limits, not a permanent product ABI.

## Deterministic hostile corpus

CI reuses the pinned real `help.pub` fixture and SHA from the earlier Rescue security provenance. It mixes that control with truncations, mandatory-header corruption, invalid directory/DIFAT references, deterministic random bytes, a sparse over-limit PUB, a `.pub` symlink and a `.pub` FIFO.

The whole batch runs twice and the machine-readable receipts must match exactly. The pinned PUB must be accepted as CFB; malformed regular files must be `parse_failed`; the sparse input must be rejected before read; symlink/FIFO entries must never be opened.

Separate runs prove CFB-entry and declared-stream limits, plus a forced wall timeout. The post-read probe requires read, write and process-spawn escape attempts to fail specifically with `EPERM`.

## Scope boundary

This task owns the shared untrusted-file/process/resource foundation. It does not claim that a known image/font MIME is safe and does not duplicate decoder policy. Hostile PNG/JPEG/font/vector decoding remains at the owning image/font/render surfaces.

The Windows `chaptera-recovery-worker.exe` protocol from `RESCUE-WORKER-ISOLATION-01` remains the product recovery process contract. This Linux V1 slice supplies reusable parser/batch security evidence; it does not substitute for the Windows Job Object acceptance still required by that task.
