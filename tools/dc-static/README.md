# DC-STATIC-01 hosted static pass

This is the `rar` execution reroute for PUB-T-164 / DC-STATIC-01.

The official Microsoft package is unchanged and hash-pinned:

- package: `publisher2010-kb3114395-fullfile-x86-glb.exe`
- package SHA-256: `9bf8c3771b133781b29ffe24a913d62fd9a009a8c5da46a80e192805759063d8`
- target: x86 `MSPUB.EXE 14.0.7162.5000`, 9,675,944 bytes
- target SHA-256: `27c00f7f06957f24d392f9c61fbd3b40282f15dc595caf66785b561f559d7b97`

The earlier yab run proved package integrity but stopped at acquisition because the
MSP-extracted cabinet stream is named `PATCH_CAB` with no `.cab` extension.
That run only expanded extension-matched containers. The rar revision recognizes
both `PATCH_CAB` and the CAB magic `MSCF`, then expands those streams before
the exact target hash gate.

No Microsoft executable, MSP, MSI or CAB payload is uploaded as an Actions
artifact. Only derived JSON/Markdown receipts are eligible.

If the exact target is recovered, the existing bounded PE analyzer maps the
pre-registered Design Checker anchor vocabulary to resources and code-reference
candidates. Static proximity is not promoted to a dispatcher/check-table claim
without a later runtime join.
