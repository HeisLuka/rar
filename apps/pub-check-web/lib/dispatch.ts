export async function dispatchCheck(checkId: string, origin: string) {
  const webhook = process.env.CHECKER_WEBHOOK_URL;
  if (!webhook) return 'not_configured' as const;

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 8000);
  try {
    const response = await fetch(webhook, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        ...(process.env.CHECKER_WEBHOOK_TOKEN
          ? { authorization: `Bearer ${process.env.CHECKER_WEBHOOK_TOKEN}` }
          : {}),
      },
      body: JSON.stringify({
        schema: 'chaptera.pub-check-dispatch.v1',
        checkId,
        sourceUrl: `${origin}/api/internal/source/${encodeURIComponent(checkId)}`,
        resultUrl: `${origin}/api/internal/result/${encodeURIComponent(checkId)}`,
      }),
      signal: controller.signal,
    });
    return response.ok ? ('sent' as const) : ('failed' as const);
  } catch {
    return 'failed' as const;
  } finally {
    clearTimeout(timer);
  }
}
