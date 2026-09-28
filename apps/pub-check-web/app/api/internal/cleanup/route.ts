import { del } from '@vercel/blob';

import { internalAuthorized } from '../../../../lib/auth';
import {
  ORPHAN_RETENTION_MS,
  deleteCheck,
  listCheckRecords,
  listOrphanIncomingBefore,
} from '../../../../lib/checks';

function cronAuthorized(request: Request) {
  const secret = process.env.CRON_SECRET;
  return !!secret && request.headers.get('authorization') === `Bearer ${secret}`;
}

export async function GET(request: Request) {
  if (!cronAuthorized(request) && !internalAuthorized(request)) {
    return new Response('Unauthorized', { status: 401 });
  }

  const now = Date.now();
  const records = await listCheckRecords();
  const retained = records.filter((record) => new Date(record.deleteAfter).getTime() > now);
  const expired = records.filter((record) => new Date(record.deleteAfter).getTime() <= now);

  for (const record of expired) await deleteCheck(record);

  const referenced = new Set(retained.map((record) => record.source.url));
  const orphanCutoff = new Date(now - ORPHAN_RETENTION_MS);
  const possibleOrphans = await listOrphanIncomingBefore(orphanCutoff);
  const orphans = possibleOrphans.filter((url) => !referenced.has(url));
  await Promise.allSettled(orphans.map((url) => del(url)));

  return Response.json({
    expiredChecksDeleted: expired.length,
    orphanSourcesDeleted: orphans.length,
  });
}
