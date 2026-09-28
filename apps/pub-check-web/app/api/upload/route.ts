import { handleUpload, type HandleUploadBody } from '@vercel/blob/client';
import { checkBotId } from 'botid/server';

const MAX_BYTES = 64 * 1024 * 1024;

export async function POST(request: Request) {
  const body = (await request.json()) as HandleUploadBody;

  if (body.type === 'blob.generate-client-token') {
    const verification = await checkBotId();
    if (verification.isBot) {
      return Response.json({ error: 'Automated upload rejected.' }, { status: 403 });
    }
  }

  try {
    const response = await handleUpload({
      body,
      request,
      onBeforeGenerateToken: async (pathname) => {
        const normalized = pathname.toLowerCase();
        if (!normalized.startsWith('incoming/') || !normalized.endsWith('.pub')) {
          throw new Error('Only .pub files are accepted.');
        }

        return {
          allowedContentTypes: [
            'application/octet-stream',
            'application/vnd.ms-publisher',
            'application/x-mspublisher',
          ],
          maximumSizeInBytes: MAX_BYTES,
          addRandomSuffix: true,
          allowOverwrite: false,
          validUntil: Date.now() + 10 * 60 * 1000,
        };
      },
      onUploadCompleted: async () => {
        // The client separately creates a check record after the private blob is complete.
        // Orphan blobs are removed by the cleanup job.
      },
    });

    return Response.json(response);
  } catch (cause) {
    return Response.json(
      { error: cause instanceof Error ? cause.message : 'Upload could not be authorized.' },
      { status: 400 },
    );
  }
}
