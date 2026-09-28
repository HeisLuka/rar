import { publicTokenMatches, readCheck } from '../../../../lib/checks';

export const dynamic = 'force-dynamic';

export async function GET(
  request: Request,
  context: { params: Promise<{ id: string }> },
) {
  const { id } = await context.params;
  const token = new URL(request.url).searchParams.get('token') ?? '';
  const record = await readCheck(id);

  if (!record || !token || !publicTokenMatches(record.publicToken, token)) {
    return Response.json({ error: 'Check not found.' }, { status: 404 });
  }

  return Response.json(
    {
      status: record.status,
      emailStatus: record.emailStatus,
      result: record.result
        ? {
            compatibility: record.result.compatibility,
            summary: record.result.summary,
            publisherFamily: record.result.publisherFamily,
            pages: record.result.pages,
            diagnosticsCode: record.result.diagnosticsCode,
          }
        : undefined,
    },
    {
      headers: {
        'cache-control': 'no-store',
      },
    },
  );
}
