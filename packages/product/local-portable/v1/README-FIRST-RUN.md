# Chaptera Local — Portable V0

This package is intended to run without a repository checkout, Rust/Cargo, a
system Python installation, pip, or Visual Studio Build Tools.

Start:

```text
Start-Chaptera-Local.cmd
```

The package keeps durable local state outside the install directory by default:

```text
%LOCALAPPDATA%\Chaptera\Local
```

The current bounded V0 browser editor still downloads one pinned public
SampleNewsletter.pub fixture on first use and verifies its exact SHA-256 and
byte length before use. Native Save PUB is not claimed.

For diagnostics, open the Chaptera Local browser dashboard. Server, worker and
editor bootstrap logs are stored below the local state directory.

## Package integrity

The release ZIP is accompanied by `Chaptera-Local.package.json`, and the extracted package contains `SHA256SUMS`.
Chaptera CI verifies the ZIP hash/size against the outer manifest and verifies every extracted runtime file against the internal checksum ledger before the clean-package smoke is accepted.

The hosted Windows clean-package smoke proves checkout/toolchain independence and package immutability, but it is not a claim that GitHub's runner is a virgin Windows installation. Native PE/DLL dependency closure for the shipped Chaptera Rust executables is checked separately during the package build.

