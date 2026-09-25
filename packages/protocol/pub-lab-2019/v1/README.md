# PUB-LAB-2019 VMware reset backend

This directory defines public-safe evidence contracts for the one authoritative
Publisher 2019 VMware lab. It does **not** introduce a second generic reset
protocol.

## Boundary

VMware-specific evidence is private/backend-facing:

1. `chaptera.pub-lab-2019-restore-challenge.v1`
2. `chaptera.publisher2019-environment-manifest.v1`
3. `chaptera.pub-lab-2019-vmware-evidence.v1`

The runner-facing receipt remains the existing provider-neutral contract:

`pub-research-reset-receipt.v1`

`tools/build_pub_research_reset_receipt.py` maps verified VMware evidence into
that existing contract. `tools/research-runner/verify_reset_receipt.py` is the
ported donor verifier from the established reset seam.

## Existing provider signature

`tools/pub_lab_2019_vmware_reset_provider.ps1` accepts the same arguments as the
existing reset-provider seam:

- `-BaselineId`
- `-SnapshotId`
- `-ExperimentId`
- `-PacketSha256`
- `-ReceiptPath`

It is pinned to:

- baseline `publisher-2019-build12527-golden-v1`
- snapshot `MODERN-2019-12527-GOLDEN-v1`

Local configuration is supplied only through machine-local environment
variables:

- `PUB_LAB_2019_VMX_PATH`
- `PUB_LAB_2019_EXPECTED_ENVIRONMENT_FINGERPRINT`
- `PUB_LAB_2019_ENV_CAPTURE_PROVIDER`

The environment-capture provider is local/private and must accept
`-VmxPath`, `-ChallengeFile`, and `-OutputManifest`. It must capture a
fresh post-boot EnvironmentManifest bound to the challenge nonce. No
credential, product key, activation token, licensed-media path, VM path, or
restore nonce is serialized into the provider-neutral receipt.

## Authority

Hosted CI validates contracts, negative cases, PowerShell syntax, and the
projection into `pub-research-reset-receipt.v1`. Hosted CI never claims that a
VMware snapshot was restored.

Native authority still requires the real `PUB-LAB-2019` VM, exact Publisher
`16.0.12527.22145`, the golden snapshot, and two independent cold restores.
A local ISO/image and lawful Publisher installation entitlement are sufficient;
physical media is not required.
