import { internalAuthorized } from '../../../../../lib/auth';
import {
  type CheckResult,
  type Compatibility,
  readCheck,
  writeCheck,
} from '../../../../../lib/checks';
import { sendResultEmail } from '../../../../../lib/email';

const COMPATIBILITY: Compatibility[] = [
  'compatible',
  'partial',
  'unsupported',
  'invalid',
  'failed',
];

function validResult(value: unknown): value is CheckResult {
  if (!value || typeof value !== 'object') return false;
  const result = value as CheckResult;
  return (
    COMPATIBILITY.includes(result.compatibility) &&
    typeof result.summary === 'string' &&
    result.summary.trim().length > 0 &&
    result.summary.length <= 4000 &&
    (result.pages === undefined ||
      (Number.isSafeInteger(result.pages) && result.pages >= 0 && result.pages <= 100000)) &&
    (result.limitations === undefined ||
      (Array.isArray(result.limitations) &&
        result.limitations.length <= 100 &&
        result.limitations.every((item) => typeof item === 'string' && item.length <= 1000)))
  );
}

export async function POST(
  request: Request,
  context: { params: Promise<{ id: string }> },
) {
  if (!internalAuthorized(request)) {
    return new Response('Unauthorized', { status: 401 });
  }

  const { id } = await context.params;
  const record = await readCheck(id);
  if (!record) return Response.json({ error: 'Check not found.' }, { status: 404 });

  let result: unknown;
  try {
    result = await request.json();
  } catch {
    return Response.json({ error: 'Invalid JSON result.' }, { status: 400 });
  }
  if (!validResult(result)) {
    return Response.json({ error: 'Result does not match the bounded report schema.' }, { status: 400 });
  }

  record.result = {
    ...result,
    summary: result.summary.trim(),
    limitations: result.limitations?.map((item) => item.trim()).filter(Boolean),
  };
  record.status = result.compatibility === 'failed' ? 'failed' : 'complete';
  record.updatedAt = new Date().toISOString();
  await writeCheck(record);

  record.emailStatus = await sendResultEmail(record);
  record.updatedAt = new Date().toISOString();
  await writeCheck(record);

  return Response.json({
    ok: true,
    status: record.status,
    emailStatus: record.emailStatus,
  });
}
