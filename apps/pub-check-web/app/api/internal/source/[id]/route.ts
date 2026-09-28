import { get } from '@vercel/blob';

import { internalAuthorized } from '../../../../../lib/auth';
import { readCheck, writeCheck } from '../../../../../lib/checks';

export const dynamic = 'force-dynamic';

export async function GET(
  request: Request,
  context: { params: Promise<{ id: string }> },
) {
  if (!internalAuthorized(request)) {
    return new Response('Unauthorized', { status: 401 });
  }

  const { id } = await context.params;
  const record = await readCheck(id);
  if (!record) return new Response('Not found', { status: 404 });

  const blob = await get(record.source.url, { access: 'private', useCache: false });
  if (!blob || blob.statusCode !== 200) return new Response('Source unavailable', { status: 404 });

  if (record.status === 'queued') {
    record.status = 'processing';
    record.updatedAt = new Date().toISOString();
    await writeCheck(record);
  }

  return new Response(blob.stream, {
    headers: {
      'content-type': 'application/octet-stream',
      'content-length': String(record.source.byteLength),
      'content-disposition': 'attachment; filename="source.pub"',
      'cache-control': 'no-store',
      'x-content-type-options': 'nosniff',
      'x-chaptera-check-id': record.id,
    },
  });
}
