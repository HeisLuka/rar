async function dispatchGitHub(checkId: string, origin: string) {
  const token = process.env.GITHUB_CHECKER_TOKEN;
  const repository = process.env.GITHUB_CHECKER_REPOSITORY || 'HeisLuka/rar';
  const ref = process.env.GITHUB_CHECKER_REF || 'main';
  if (!token) return 'not_configured' as const;

  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) {
    return 'failed' as const;
  }

  const response = await fetch(
    `https://api.github.com/repos/${repository}/actions/workflows/pub-check-worker.yml/dispatches`,
    {
      method: 'POST',
      headers: {
        authorization: `Bearer ${token}`,
        accept: 'application/vnd.github+json',
        'content-type': 'application/json',
        'x-github-api-version': '2022-11-28',
      },
      body: JSON.stringify({
        ref,
        inputs: {
          check_id: checkId,
        },
      }),
    },
  );

  return response.status === 204 ? ('sent' as const) : ('failed' as const);
}

async function dispatchWebhook(checkId: string, origin: string) {
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

export async function dispatchCheck(checkId: string, origin: string) {
  if (process.env.GITHUB_CHECKER_TOKEN) {
    return dispatchGitHub(checkId, origin);
  }
  return dispatchWebhook(checkId, origin);
}
