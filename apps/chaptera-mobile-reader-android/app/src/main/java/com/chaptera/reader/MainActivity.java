package com.chaptera.reader;

import android.app.Activity;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.provider.OpenableColumns;
import android.database.Cursor;
import android.view.View;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;

public final class MainActivity extends Activity {
    private static final int OPEN_DOCUMENT_REQUEST = 1001;
    private static final int MAX_SOURCE_BYTES = 128 * 1024 * 1024;

    private TextView status;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);

        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setPadding(24, 24, 24, 24);

        Button open = new Button(this);
        open.setText("Open .pub");
        open.setOnClickListener(v -> chooseDocument());

        status = new TextView(this);
        status.setText("Choose a local Microsoft Publisher file.");

        ScrollView scroll = new ScrollView(this);
        scroll.addView(status);

        root.addView(open, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT));
        root.addView(scroll, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                0,
                1.0f));

        setContentView(root);

        Intent launch = getIntent();
        if (Intent.ACTION_VIEW.equals(launch.getAction()) && launch.getData() != null) {
            openUri(launch.getData());
        }
    }

    private void chooseDocument() {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        startActivityForResult(intent, OPEN_DOCUMENT_REQUEST);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == OPEN_DOCUMENT_REQUEST
                && resultCode == RESULT_OK
                && data != null
                && data.getData() != null) {
            Uri uri = data.getData();
            try {
                getContentResolver().takePersistableUriPermission(
                        uri,
                        data.getFlags() & Intent.FLAG_GRANT_READ_URI_PERMISSION);
            } catch (SecurityException ignored) {
                // Some providers grant only one-shot read permission.
            }
            openUri(uri);
        }
    }

    private void openUri(Uri uri) {
        status.setText("Opening " + displayName(uri) + "...");
        new Thread(() -> {
            String result;
            try {
                byte[] bytes = readBounded(uri);
                result = NativeReader.openLocalPubJson(bytes);
                if (result.startsWith("ERR:OPEN:")) {
                    String diagnostic = NativeReader.failureDiagnosticJson(bytes);
                    result = result + "\n\nDiagnostic:\n" + diagnostic;
                }
            } catch (Throwable error) {
                result = "ERR:ANDROID_INGRESS:" + error.getClass().getSimpleName()
                        + ":" + error.getMessage();
            }
            String finalResult = result;
            runOnUiThread(() -> status.setText(finalResult));
        }, "chaptera-local-open").start();
    }

    private byte[] readBounded(Uri uri) throws IOException {
        try (InputStream input = getContentResolver().openInputStream(uri)) {
            if (input == null) {
                throw new IOException("ContentResolver returned no stream");
            }
            ByteArrayOutputStream output = new ByteArrayOutputStream();
            byte[] buffer = new byte[64 * 1024];
            int total = 0;
            while (true) {
                int read = input.read(buffer);
                if (read < 0) {
                    break;
                }
                total += read;
                if (total > MAX_SOURCE_BYTES) {
                    throw new IOException("Source exceeds 128 MiB V0 ingress limit");
                }
                output.write(buffer, 0, read);
            }
            return output.toByteArray();
        }
    }

    private String displayName(Uri uri) {
        if ("content".equals(uri.getScheme())) {
            try (Cursor cursor = getContentResolver().query(
                    uri, new String[]{OpenableColumns.DISPLAY_NAME}, null, null, null)) {
                if (cursor != null && cursor.moveToFirst()) {
                    int index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                    if (index >= 0) {
                        return cursor.getString(index);
                    }
                }
            } catch (RuntimeException ignored) {
                // Display name is only a UI hint, never content authority.
            }
        }
        return uri.getLastPathSegment() == null ? "local document" : uri.getLastPathSegment();
    }
}
