'use client';

import { upload } from '@vercel/blob/client';
import { useEffect, useMemo, useRef, useState } from 'react';

const MAX_BYTES = 64 * 1024 * 1024;

type PublicStatus = {
  status: 'queued' | 'processing' | 'complete' | 'failed';
  emailStatus: 'pending' | 'sent' | 'failed' | 'not_configured';
  result?: {
    compatibility: 'compatible' | 'partial' | 'unsupported' | 'invalid' | 'failed';
    summary: string;
    publisherFamily?: string;
    pages?: number;
    diagnosticsCode?: string;
  };
};

function bytes(value: number) {
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / 1024 / 1024).toFixed(1)} MB`;
}

export default function Home() {
  const inputRef = useRef<HTMLInputElement>(null);
  const [file, setFile] = useState<File | null>(null);
  const [email, setEmail] = useState('');
  const [consent, setConsent] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [stage, setStage] = useState<'idle' | 'uploading' | 'queued'>('idle');
  const [progress, setProgress] = useState(0);
  const [check, setCheck] = useState<{ id: string; token: string } | null>(null);
  const [status, setStatus] = useState<PublicStatus | null>(null);
  const [error, setError] = useState('');

  const validEmail = useMemo(() => /^\S+@\S+\.\S+$/.test(email.trim()), [email]);

  function choose(next: File | null) {
    setError('');
    setStatus(null);
    setCheck(null);
    setProgress(0);
    if (!next) return setFile(null);
    if (!next.name.toLowerCase().endsWith('.pub')) {
      setFile(null);
      return setError('Please choose a Microsoft Publisher .pub file.');
    }
    if (next.size === 0 || next.size > MAX_BYTES) {
      setFile(null);
      return setError('The file must be between 1 byte and 64 MB.');
    }
    setFile(next);
  }

  async function submit() {
    if (!file || !validEmail || !consent || stage !== 'idle') return;
    setError('');
    setStage('uploading');
    try {
      const safeName = file.name.replace(/[^a-zA-Z0-9._-]+/g, '_').slice(-120);
      const blob = await upload(`incoming/${safeName}`, file, {
        access: 'private',
        handleUploadUrl: '/api/upload',
        contentType: file.type || 'application/octet-stream',
        multipart: file.size > 8 * 1024 * 1024,
        onUploadProgress: ({ percentage }) => setProgress(Math.round(percentage)),
      });

      const response = await fetch('/api/checks', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          email: email.trim(),
          blobUrl: blob.url,
          pathname: blob.pathname,
          filename: file.name,
          byteLength: file.size,
        }),
      });
      const body = await response.json();
      if (!response.ok) throw new Error(body.error || 'Could not start the check.');

      setCheck({ id: body.id, token: body.token });
      setStage('queued');
      setStatus({
        status: body.status,
        emailStatus: 'pending',
      });
    } catch (cause) {
      setStage('idle');
      setError(cause instanceof Error ? cause.message : 'Upload failed.');
    }
  }

  useEffect(() => {
    if (!check || status?.status === 'complete' || status?.status === 'failed') return;
    let alive = true;
    const poll = async () => {
      try {
        const response = await fetch(
          `/api/checks/${encodeURIComponent(check.id)}?token=${encodeURIComponent(check.token)}`,
          { cache: 'no-store' },
        );
        const body = await response.json();
        if (alive && response.ok) setStatus(body);
      } catch {
        // Keep polling. A transient status request should not restart the upload.
      }
    };
    void poll();
    const timer = window.setInterval(poll, 3500);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [check, status?.status]);

  const busy = stage !== 'idle';
  const complete = status?.status === 'complete';

  return (
    <main>
      <div className="shell">
        <nav className="nav">
          <div className="brand">Chaptera</div>
          <div className="badge">PUB compatibility check · preview</div>
        </nav>

        <section className="hero">
          <div className="eyebrow">Microsoft Publisher files</div>
          <h1>Will your .PUB file still work?</h1>
          <p>
            Drop in a Publisher file. Chaptera will inspect it in an isolated checker and email
            you a compatibility report with what we can read, what may be incomplete, and what
            needs attention.
          </p>

          <div className="card">
            {!check ? (
              <>
                <div
                  className={`drop ${dragging ? 'active' : ''}`}
                  onClick={() => !busy && inputRef.current?.click()}
                  onDragOver={(event) => {
                    event.preventDefault();
                    if (!busy) setDragging(true);
                  }}
                  onDragLeave={() => setDragging(false)}
                  onDrop={(event) => {
                    event.preventDefault();
                    setDragging(false);
                    if (!busy) choose(event.dataTransfer.files?.[0] ?? null);
                  }}
                >
                  <input
                    ref={inputRef}
                    type="file"
                    accept=".pub,application/vnd.ms-publisher,application/x-mspublisher"
                    onChange={(event) => choose(event.target.files?.[0] ?? null)}
                  />
                  <div>
                    <div className="dropIcon">↥</div>
                    <strong>Drop your .PUB file here</strong>
                    <small>or click to choose a file · up to 64 MB</small>
                  </div>
                </div>

                {file && (
                  <div className="filePill">
                    <div>
                      <b>{file.name}</b>
                      <span>{bytes(file.size)}</span>
                    </div>
                    {!busy && (
                      <button type="button" onClick={() => choose(null)}>
                        Remove
                      </button>
                    )}
                  </div>
                )}

                <div className="formRow">
                  <input
                    className="email"
                    type="email"
                    placeholder="you@company.com"
                    value={email}
                    disabled={busy}
                    onChange={(event) => setEmail(event.target.value)}
                    aria-label="Email for the report"
                  />
                  <button
                    className="cta"
                    type="button"
                    disabled={!file || !validEmail || !consent || busy}
                    onClick={submit}
                  >
                    {busy ? 'Uploading…' : 'Check my file'}
                  </button>
                </div>

                <label className="consent">
                  <input
                    type="checkbox"
                    checked={consent}
                    disabled={busy}
                    onChange={(event) => setConsent(event.target.checked)}
                  />
                  <span>
                    I have the right to upload this file. I understand it will be processed for
                    this compatibility check and automatically deleted after the retention window.
                  </span>
                </label>

                {stage === 'uploading' && (
                  <div className="progressWrap">
                    <div className="progressBar"><div style={{ width: `${progress}%` }} /></div>
                    <div className="progressText"><span>Uploading privately</span><span>{progress}%</span></div>
                  </div>
                )}

                {error && <div className="error">{error}</div>}
              </>
            ) : (
              <div className="status">
                <div className="statusTop">
                  <div>
                    <h3>
                      {complete
                        ? 'Compatibility report ready'
                        : status?.status === 'failed'
                          ? 'We could not complete this check'
                          : status?.status === 'processing'
                            ? 'Checking your Publisher file'
                            : 'Your file is in the checking queue'}
                    </h3>
                    <p>
                      {complete
                        ? status?.result?.summary
                        : 'You can close this page. We will send the result to the email address you provided.'}
                    </p>
                  </div>
                  <div className="statusChip">{status?.status ?? 'queued'}</div>
                </div>

                {complete && status?.result && (
                  <div className="resultGrid">
                    <div className="metric">
                      <span>Compatibility</span>
                      <b>{status.result.compatibility}</b>
                    </div>
                    <div className="metric">
                      <span>Publisher family</span>
                      <b>{status.result.publisherFamily || 'Not identified'}</b>
                    </div>
                    <div className="metric">
                      <span>Email</span>
                      <b>{status.emailStatus === 'sent' ? 'Report sent' : 'Report prepared'}</b>
                    </div>
                  </div>
                )}

                {status?.status === 'failed' && (
                  <div className="error">
                    The checker could not produce a reliable result. We will not label the file
                    compatible when the evidence is incomplete.
                  </div>
                )}
              </div>
            )}
          </div>
        </section>

        <section className="featureGrid">
          <div className="feature">
            <b>Private by default</b>
            <p>The uploaded file is private and is not exposed as a public download.</p>
          </div>
          <div className="feature">
            <b>Evidence, not guesses</b>
            <p>Unknown or unsupported parts are reported as such instead of being silently treated as working.</p>
          </div>
          <div className="feature">
            <b>Results by email</b>
            <p>The final compatibility summary is sent only after the checker returns a completed receipt.</p>
          </div>
        </section>

        <footer className="footer">
          <span>Chaptera · Publisher continuity tools</span>
          <span>Uploaded files are temporary and used only to perform the requested check.</span>
        </footer>
      </div>
    </main>
  );
}
