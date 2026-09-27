package com.chaptera.reader;

import android.app.Activity;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.provider.OpenableColumns;
import android.view.Gravity;
import android.view.View;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.TextView;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.security.MessageDigest;
import org.json.JSONObject;

public final class MainActivity extends Activity {
    private static final int OPEN_DOCUMENT = 1001;

    private TextView status;
    private PubCanvasView canvas;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);

        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setPadding(16, 16, 16, 16);

        Button open = new Button(this);
        open.setText("Open PUB");
        open.setOnClickListener(v -> chooseDocument());
        root.addView(open, new LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT,
            LinearLayout.LayoutParams.WRAP_CONTENT
        ));

        status = new TextView(this);
        status.setId(com.chaptera.reader.R.id.reader_status);
        status.setGravity(Gravity.CENTER_VERTICAL);
        status.setText("Open a local .pub file. No account or network is required.");
        root.addView(status, new LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT,
            LinearLayout.LayoutParams.WRAP_CONTENT
        ));

        canvas = new PubCanvasView(this);
        canvas.setId(com.chaptera.reader.R.id.reader_canvas);
        root.addView(canvas, new LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT,
            0,
            1f
        ));
        setContentView(root);

        Uri handedOff = getIntent() == null ? null : getIntent().getData();
        if (handedOff != null) openUri(handedOff);
    }

    private void chooseDocument() {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        startActivityForResult(intent, OPEN_DOCUMENT);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == OPEN_DOCUMENT && resultCode == RESULT_OK && data != null && data.getData() != null) {
            Uri uri = data.getData();
            int flags = data.getFlags() & Intent.FLAG_GRANT_READ_URI_PERMISSION;
            if (flags != 0) {
                try {
                    getContentResolver().takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION);
                } catch (SecurityException ignored) {
                    // Some providers grant one-shot access only; opening still proceeds.
                }
            }
            openUri(uri);
        }
    }

    private void openUri(Uri uri) {
        try {
            byte[] bytes = readBounded(uri, 128 * 1024 * 1024);
            String before = sha256(bytes);
            String wire = NativeReader.openLocalPubJson(bytes);
            String after = sha256(bytes);
            if (!before.equals(after)) {
                throw new IllegalStateException("Reader core mutated the supplied source bytes");
            }
            if (wire.startsWith("ERR:")) {
                String diagnostic = NativeReader.failureDiagnosticJson(bytes);
                status.setText("Could not open this file locally. " + compactError(wire, diagnostic));
                canvas.setPage(null);
                return;
            }

            JSONObject receipt = new JSONObject(wire);
            int pages = receipt.getInt("page_count");
            String fidelity = receipt.optString("fidelity", "unknown");
            JSONObject firstPage = receipt.getJSONObject("first_page");
            status.setText(displayName(uri) + " · " + pages + " page(s) · " + fidelity + " · offline local open");
            canvas.setPage(firstPage);
        } catch (SecurityException denied) {
            status.setText("Chaptera no longer has permission to read this file. Select it again.");
            canvas.setPage(null);
        } catch (Exception error) {
            status.setText("Could not read this local file: " + error.getMessage());
            canvas.setPage(null);
        }
    }

    private byte[] readBounded(Uri uri, int maxBytes) throws Exception {
        try (InputStream input = getContentResolver().openInputStream(uri)) {
            if (input == null) throw new IllegalStateException("content provider returned no stream");
            ByteArrayOutputStream output = new ByteArrayOutputStream();
            byte[] buffer = new byte[64 * 1024];
            int total = 0;
            for (;;) {
                int read = input.read(buffer);
                if (read < 0) break;
                total += read;
                if (total > maxBytes) throw new IllegalArgumentException("file exceeds local Reader size limit");
                output.write(buffer, 0, read);
            }
            return output.toByteArray();
        }
    }

    private String displayName(Uri uri) {
        try (android.database.Cursor cursor = getContentResolver().query(
            uri, new String[]{OpenableColumns.DISPLAY_NAME}, null, null, null
        )) {
            if (cursor != null && cursor.moveToFirst()) {
                int index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (index >= 0) return cursor.getString(index);
            }
        } catch (Exception ignored) {}
        return "Local PUB";
    }

    private static String compactError(String wire, String diagnostic) {
        String base = wire.length() > 180 ? wire.substring(0, 180) : wire;
        if (diagnostic == null || diagnostic.startsWith("ERR:")) return base;
        return base;
    }

    private static String sha256(byte[] bytes) throws Exception {
        byte[] digest = MessageDigest.getInstance("SHA-256").digest(bytes);
        StringBuilder out = new StringBuilder(digest.length * 2);
        for (byte value : digest) out.append(String.format("%02x", value & 0xff));
        return out.toString();
    }
}
