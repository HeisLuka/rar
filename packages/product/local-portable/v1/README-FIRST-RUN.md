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
