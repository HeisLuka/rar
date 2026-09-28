import type { CheckRecord } from './checks';

function html(value: string) {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#039;');
}

export async function sendResultEmail(record: CheckRecord) {
  const apiKey = process.env.RESEND_API_KEY;
  const from = process.env.REPORT_FROM_EMAIL;
  if (!apiKey || !from) return 'not_configured' as const;
  if (!record.result) return 'failed' as const;

  const result = record.result;
  const limitations =
    result.limitations?.length
      ? `<ul>${result.limitations
          .slice(0, 12)
          .map((item) => `<li>${html(item)}</li>`)
          .join('')}</ul>`
      : '<p>No additional limitations were included in this receipt.</p>';

  const payload = JSON.stringify({
      from,
      to: [record.email],
      subject: 'Your Chaptera PUB compatibility report',
      text: [
        'Your Chaptera PUB compatibility check is ready.',
        '',
        `Compatibility: ${result.compatibility}`,
        `Summary: ${result.summary}`,
        result.publisherFamily ? `Publisher family: ${result.publisherFamily}` : '',
        typeof result.pages === 'number' ? `Pages observed: ${result.pages}` : '',
        result.limitations?.length
          ? `Limitations: ${result.limitations.join('; ')}`
          : '',
        '',
        'This report describes only what the current checker could prove. Unknown or unsupported content is not treated as compatible.',
        'The uploaded file is temporary and is deleted after the configured retention window.',
      ].filter(Boolean).join('\n'),
      html: `
        <div style="font-family:Inter,Arial,sans-serif;color:#172018;max-width:640px;margin:auto">
          <p style="font-size:13px;color:#617064">Chaptera PUB compatibility check</p>
          <h1 style="font-size:28px;line-height:1.15;margin:12px 0 18px">Your report is ready</h1>
          <div style="border:1px solid #dfe7df;border-radius:16px;padding:18px">
            <p style="margin:0 0 8px;color:#617064;font-size:12px">Compatibility</p>
            <p style="margin:0 0 18px;font-size:20px;font-weight:700">${html(result.compatibility)}</p>
            <p style="margin:0 0 8px;color:#617064;font-size:12px">Summary</p>
            <p style="margin:0;line-height:1.55">${html(result.summary)}</p>
          </div>
          ${result.publisherFamily ? `<p><b>Publisher family:</b> ${html(result.publisherFamily)}</p>` : ''}
          ${typeof result.pages === 'number' ? `<p><b>Pages observed:</b> ${result.pages}</p>` : ''}
          <h2 style="font-size:17px;margin-top:24px">Known limitations</h2>
          ${limitations}
          <p style="font-size:12px;color:#617064;margin-top:26px;line-height:1.5">
            This is a bounded compatibility report, not a promise of perfect Publisher fidelity.
            Unknown or unsupported content remains explicitly unknown or unsupported.
          </p>
        </div>
      `,
    });

  for (let attempt = 0; attempt < 2; attempt += 1) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 10000);
    try {
      const response = await fetch('https://api.resend.com/emails', {
        method: 'POST',
        headers: {
          authorization: `Bearer ${apiKey}`,
          'content-type': 'application/json',
          'idempotency-key': `pub-check-result/${record.id}`,
        },
        body: payload,
        signal: controller.signal,
      });
      if (response.ok) return 'sent' as const;
      if (attempt === 0 && (response.status === 429 || response.status >= 500)) continue;
      return 'failed' as const;
    } catch {
      if (attempt === 1) return 'failed' as const;
    } finally {
      clearTimeout(timer);
    }
  }

  return 'failed' as const;
}
