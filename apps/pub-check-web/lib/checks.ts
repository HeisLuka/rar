import { del, get, list, put } from '@vercel/blob';
import { randomBytes, timingSafeEqual } from 'node:crypto';

export const CHECK_RETENTION_MS = 7 * 24 * 60 * 60 * 1000;
export const ORPHAN_RETENTION_MS = 24 * 60 * 60 * 1000;

export type CheckStatus = 'queued' | 'processing' | 'complete' | 'failed';
export type EmailStatus = 'pending' | 'sent' | 'failed' | 'not_configured';
export type Compatibility =
  | 'compatible'
  | 'partial'
  | 'unsupported'
  | 'invalid'
  | 'failed';

export type CheckResult = {
  compatibility: Compatibility;
  summary: string;
  publisherFamily?: string;
  pages?: number;
  diagnosticsCode?: string;
  limitations?: string[];
  checkerVersion?: string;
};

export type CheckRecord = {
  schema: 'chaptera.pub-check.v1';
  id: string;
  publicToken: string;
  email: string;
  source: {
    url: string;
    pathname: string;
    filename: string;
    byteLength: number;
  };
  status: CheckStatus;
  emailStatus: EmailStatus;
  dispatchStatus: 'pending' | 'sent' | 'not_configured' | 'failed';
  result?: CheckResult;
  createdAt: string;
  updatedAt: string;
  deleteAfter: string;
};

const recordPath = (id: string) => `checks/${id}.json`;

async function streamText(stream: ReadableStream<Uint8Array>) {
  return new Response(stream).text();
}

export function makePublicToken() {
  return randomBytes(32).toString('base64url');
}

export function publicTokenMatches(expected: string, actual: string) {
  const a = Buffer.from(expected);
  const b = Buffer.from(actual);
  return a.length === b.length && timingSafeEqual(a, b);
}

export async function writeCheck(record: CheckRecord) {
  await put(recordPath(record.id), JSON.stringify(record), {
    access: 'private',
    allowOverwrite: true,
    contentType: 'application/json; charset=utf-8',
    cacheControlMaxAge: 0,
  });
}

export async function readCheck(id: string): Promise<CheckRecord | null> {
  const result = await get(recordPath(id), {
    access: 'private',
    useCache: false,
  });
  if (!result || result.statusCode !== 200) return null;
  const parsed = JSON.parse(await streamText(result.stream)) as CheckRecord;
  if (parsed.schema !== 'chaptera.pub-check.v1' || parsed.id !== id) return null;
  return parsed;
}

export async function deleteCheck(record: CheckRecord) {
  await Promise.allSettled([
    del(record.source.url),
    del(recordPath(record.id)),
  ]);
}

export async function listCheckRecords() {
  const found: CheckRecord[] = [];
  let cursor: string | undefined;
  do {
    const page = await list({ prefix: 'checks/', cursor, limit: 100 });
    for (const blob of page.blobs) {
      const result = await get(blob.pathname, { access: 'private', useCache: false });
      if (!result || result.statusCode !== 200) continue;
      try {
        const parsed = JSON.parse(await streamText(result.stream)) as CheckRecord;
        if (parsed.schema === 'chaptera.pub-check.v1') found.push(parsed);
      } catch {
        // A malformed record is not trusted as deletion authority.
      }
    }
    cursor = page.cursor || undefined;
  } while (cursor);
  return found;
}

export async function listOrphanIncomingBefore(cutoff: Date) {
  const urls: string[] = [];
  let cursor: string | undefined;
  do {
    const page = await list({ prefix: 'incoming/', cursor, limit: 100 });
    for (const blob of page.blobs) {
      if (new Date(blob.uploadedAt).getTime() < cutoff.getTime()) urls.push(blob.url);
    }
    cursor = page.cursor || undefined;
  } while (cursor);
  return urls;
}
