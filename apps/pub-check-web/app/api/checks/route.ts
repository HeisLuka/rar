import { head } from '@vercel/blob';
import { randomUUID } from 'node:crypto';

import {
  CHECK_RETENTION_MS,
  type CheckRecord,
  makePublicToken,
  writeCheck,
} from '../../../lib/checks';
import { dispatchCheck } from '../../../lib/dispatch';

const MAX_BYTES = 64 * 1024 * 1024;

type CreateBody = {
  email?: string;
  blobUrl?: string;
  pathname?: string;
  filename?: string;
  byteLength?: number;
};

function validEmail(value: string) {
  return value.length <= 254 && /^\S+@\S+\.\S+$/.test(value);
}

export async function POST(request: Request) {
  let body: CreateBody;
  try {
    body = (await request.json()) as CreateBody;
  } catch {
    return Response.json({ error: 'Invalid request.' }, { status: 400 });
  }

  const email = body.email?.trim() ?? '';
  const filename = body.filename?.trim() ?? '';
  const pathname = body.pathname?.trim() ?? '';
  const blobUrl = body.blobUrl?.trim() ?? '';
  const byteLength = body.byteLength ?? 0;

  if (!validEmail(email)) {
    return Response.json({ error: 'Enter a valid email address.' }, { status: 400 });
  }
  if (
    !filename.toLowerCase().endsWith('.pub') ||
    !pathname.startsWith('incoming/') ||
    !pathname.toLowerCase().endsWith('.pub') ||
    !Number.isSafeInteger(byteLength) ||
    byteLength <= 0 ||
    byteLength > MAX_BYTES
  ) {
    return Response.json({ error: 'The uploaded file is not an accepted PUB file.' }, { status: 400 });
  }

  try {
    const metadata = await head(blobUrl);
    if (metadata.pathname !== pathname || metadata.size !== byteLength) {
      return Response.json({ error: 'Uploaded file identity does not match.' }, { status: 409 });
    }
  } catch {
    return Response.json({ error: 'Uploaded file could not be verified.' }, { status: 400 });
  }

  const now = new Date();
  const id = randomUUID();
  const record: CheckRecord = {
    schema: 'chaptera.pub-check.v1',
    id,
    publicToken: makePublicToken(),
    email,
    source: {
      url: blobUrl,
      pathname,
      filename: filename.slice(0, 240),
      byteLength,
    },
    status: 'queued',
    emailStatus: 'pending',
    dispatchStatus: 'pending',
    createdAt: now.toISOString(),
    updatedAt: now.toISOString(),
    deleteAfter: new Date(now.getTime() + CHECK_RETENTION_MS).toISOString(),
  };

  await writeCheck(record);

  const origin = new URL(request.url).origin;
  record.dispatchStatus = await dispatchCheck(id, origin);
  record.updatedAt = new Date().toISOString();
  await writeCheck(record);

  return Response.json({
    id: record.id,
    token: record.publicToken,
    status: record.status,
    dispatched: record.dispatchStatus,
  });
}
