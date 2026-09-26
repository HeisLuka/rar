# CHAPTERA-VTPE-ROUTES-01 — fail-closed Reader route contract

Reader 0.1 owns a provider-neutral explicit-click route boundary.

- routes are disabled by default;
- only a bare HTTPS Chaptera origin is accepted;
- Support -> /support;
- Share -> copy canonical origin;
- Report -> /report;
- ArchiveHelp -> /archive-help;
- Later / AlreadySupported -> no external effect;
- no Boosty/provider literal exists in Reader;
- no automatic navigation is performed.

The production origin is supplied only after CHAPTERA-SUPPORT-ORIGIN-01 establishes the canonical pre-release Chaptera HTTPS origin. The full site publish task is not required for this pure contract; /download can remain fail-closed until the signed public release exists.
