# Chaptera PUB Check landing

A small public surface for validating real Microsoft Publisher files before the
desktop Reader is ready for broad distribution.

## User flow

1. User drops a `.pub` file and provides an email address.
2. Browser uploads directly to a **private Vercel Blob** object.
3. The app creates a private check record and dispatches a worker job.
4. The checker downloads the exact private file through an authenticated
   internal endpoint.
5. The checker returns a bounded JSON compatibility receipt.
6. Only after that receipt is accepted, the app sends the report by email.
7. File + record expire after seven days. Uploads that never become a check are
   removed after one day.

The landing must never claim compatibility merely because upload succeeded.

## Environment

```
BLOB_READ_WRITE_TOKEN=...
CHECKER_ADMIN_TOKEN=<long random shared secret used by checker for source/result endpoints>
GITHUB_CHECKER_TOKEN=<fine-grained GitHub token with Actions write access to the checker repo>
GITHUB_CHECKER_REPOSITORY=HeisLuka/rar
GITHUB_CHECKER_REF=main
CHECKER_WEBHOOK_URL=<optional fallback checker webhook>
CHECKER_WEBHOOK_TOKEN=<optional auth from landing to fallback checker>
RESEND_API_KEY=...
REPORT_FROM_EMAIL=Chaptera <reports@your-domain.example>
CRON_SECRET=<long random Vercel cron secret>
```

Attach a **private** Vercel Blob store to the deployed project. Client uploads
are capped at 64 MB.

## Checker dispatch contract

The preferred zero-new-server path is GitHub Actions. When `GITHUB_CHECKER_TOKEN`
is configured, the landing dispatches `.github/workflows/pub-check-worker.yml`
with only the opaque `check_id`. The worker builds the source/result URLs from
the **GitHub Secret** `PUB_CHECK_ORIGIN`, so a manual workflow dispatch cannot
redirect `CHECKER_ADMIN_TOKEN` to an arbitrary host.

Configure these GitHub repository secrets after merge:

```
PUB_CHECK_ORIGIN=https://<canonical-landing-host>
CHECKER_ADMIN_TOKEN=<same high-entropy value configured on the landing>
```

The fine-grained token stored on Vercel should have the minimum repository
permission required to dispatch Actions workflows.

If `GITHUB_CHECKER_TOKEN` is absent, an optional generic checker can be used.
When `CHECKER_WEBHOOK_URL` is configured, the landing sends:

```json
{
  "schema": "chaptera.pub-check-dispatch.v1",
  "checkId": "<uuid>",
  "sourceUrl": "https://landing.example/api/internal/source/<uuid>",
  "resultUrl": "https://landing.example/api/internal/result/<uuid>"
}
```

The generic checker is configured separately with `CHECKER_ADMIN_TOKEN` and uses:

```
Authorization: Bearer <CHECKER_ADMIN_TOKEN>
```

to fetch `sourceUrl` and POST `resultUrl`.

## Result schema

```json
{
  "compatibility": "compatible | partial | unsupported | invalid | failed",
  "summary": "Human-readable bounded conclusion",
  "publisherFamily": "optional family/version label",
  "pages": 12,
  "diagnosticsCode": "optional stable sanitized code",
  "limitations": [
    "Optional user-visible limitation"
  ],
  "checkerVersion": "optional exact checker/build identity"
}
```

Do not send raw local paths, stack traces, private parser state or source bytes
in the result.

## Retention

`GET /api/internal/cleanup` is configured as a daily Vercel Cron endpoint and
requires `Authorization: Bearer $CRON_SECRET`. It removes expired checks and
old orphan uploads.

## Development

```bash
cd apps/pub-check-web
npm install
npm run typecheck
npm run build
npm run dev
```

The frontend can be previewed without a checker, but checks remain queued until
GitHub Actions or the fallback webhook checker is connected. That is deliberate; the UI does not fake a
successful compatibility result.
