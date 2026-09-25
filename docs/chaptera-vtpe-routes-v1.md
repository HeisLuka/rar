# CHAPTERA-VTPE-ROUTES-01 — fail-closed desktop route contract V1

This slice materializes the historical Chaptera supporter route law in the active `HeisLuka/rar` desktop source without inventing a production origin.

## Shared action boundary

`SupporterAction` lives in the UI-neutral supporter domain and defines exactly six user actions:

- Support
- Later
- AlreadySupported
- Share
- Report
- ArchiveHelp

This is the action boundary the later nonmodal supporter UI consumes. Route code does not infer actions from labels or duplicate route literals across widgets.

## Injected route configuration

`SupporterRoutes` is disabled by default. `from_https_origin` accepts only a bare HTTPS origin and fails closed for:

- non-HTTPS input;
- paths;
- queries;
- fragments;
- credentials/userinfo;
- backslash path ambiguity;
- whitespace or an empty authority.

A valid injected origin maps exactly:

- Support → `/support`
- Share → copy the canonical public origin
- Report → `/report`
- ArchiveHelp → `/archive-help`
- Later / AlreadySupported → no external effect

No Boosty or other payment-provider URL exists in this route layer.

## Product boundary

This PR does not set `CHAPTERA_SITE_ORIGIN`, call `egui::Context::open_url`, write clipboard text, or claim live navigation. Those actions belong to explicit UI dispatch after the UI producer is materialized.

Production wiring remains fail-closed until `CHAPTERA-SITE-PUBLISH-01` records the canonical Chaptera HTTPS origin. That task is currently blocked on external domain/hosting authority, so guessing a preview URL here would violate the product boundary.

The pure contract is still useful now: it fixes one route authority for the desktop and gives the upcoming UI/dispatch work a typed, tested consumer instead of duplicating literals.
