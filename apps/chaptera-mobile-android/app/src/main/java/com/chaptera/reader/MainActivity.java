package com.chaptera.reader;

import android.app.Activity;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.provider.OpenableColumns;
import android.text.InputType;
import android.view.Gravity;
import android.widget.Button;
import android.widget.EditText;
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
    private Button previous;
    private Button next;
    private EditText pageJump;

    private long sessionId = -1L;
    private int pageCount = 0;
    private int currentPage = 0;
    private String documentName = "Local PUB";
    private String fidelity = "unknown";

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

        LinearLayout navigation = new LinearLayout(this);
        navigation.setOrientation(LinearLayout.HORIZONTAL);

        previous = new Button(this);
        previous.setId(R.id.reader_previous);
        previous.setText("Previous");
        previous.setEnabled(false);
        previous.setOnClickListener(v -> showPage(currentPage - 1, true));
        navigation.addView(previous, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));

        pageJump = new EditText(this);
        pageJump.setId(R.id.reader_page_jump);
        pageJump.setSingleLine(true);
        pageJump.setGravity(Gravity.CENTER);
        pageJump.setInputType(InputType.TYPE_CLASS_NUMBER);
        pageJump.setHint("Page");
        navigation.addView(pageJump, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));

        Button go = new Button(this);
        go.setId(R.id.reader_page_go);
        go.setText("Go");
        go.setOnClickListener(v -> {
            try {
                int requested = Integer.parseInt(pageJump.getText().toString().trim()) - 1;
                showPage(requested, true);
            } catch (NumberFormatException ignored) {
                status.setText("Enter a page number between 1 and " + Math.max(1, pageCount) + ".");
            }
        });
        navigation.addView(go, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 0.6f));

        next = new Button(this);
        next.setId(R.id.reader_next);
        next.setText("Next");
        next.setEnabled(false);
        next.setOnClickListener(v -> showPage(currentPage + 1, true));
        navigation.addView(next, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));

        root.addView(navigation, new LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT,
            LinearLayout.LayoutParams.WRAP_CONTENT
        ));

        status = new TextView(this);
        status.setId(R.id.reader_status);
        status.setGravity(Gravity.CENTER_VERTICAL);
        status.setText("Open a local .pub file. No account or network is required.");
        root.addView(status, new LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT,
            LinearLayout.LayoutParams.WRAP_CONTENT
        ));

        canvas = new PubCanvasView(this);
        canvas.setId(R.id.reader_canvas);
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
                closeCurrentSession();
                canvas.setPage(null);
                updateNavigation();
                return;
            }

            JSONObject receipt = new JSONObject(wire);
            closeCurrentSession();
            sessionId = receipt.getLong("session_id");
            pageCount = receipt.getInt("page_count");
            currentPage = 0;
            fidelity = receipt.optString("fidelity", "unknown");
            documentName = displayName(uri);
            canvas.setPage(receipt.getJSONObject("first_page"));
            pageJump.setText("1");
            updateStatus();
            updateNavigation();
        } catch (SecurityException denied) {
            status.setText("Chaptera no longer has permission to read this file. Select it again.");
            closeCurrentSession();
            canvas.setPage(null);
            updateNavigation();
        } catch (Exception error) {
            status.setText("Could not read this local file: " + error.getMessage());
            closeCurrentSession();
            canvas.setPage(null);
            updateNavigation();
        }
    }

    private void showPage(int pageIndex, boolean resetViewport) {
        if (sessionId < 0 || pageIndex < 0 || pageIndex >= pageCount) {
            status.setText("Page must be between 1 and " + Math.max(1, pageCount) + ".");
            return;
        }
        String wire = NativeReader.renderPageJson(sessionId, pageIndex);
        if (wire.startsWith("ERR:")) {
            status.setText("Could not render page " + (pageIndex + 1) + ": " + wire);
            return;
        }
        try {
            float zoom = canvas.getZoom();
            float panX = canvas.getPanXOffset();
            float panY = canvas.getPanYOffset();
            canvas.setPage(new JSONObject(wire));
            if (!resetViewport) canvas.restoreViewport(zoom, panX, panY);
            currentPage = pageIndex;
            pageJump.setText(Integer.toString(currentPage + 1));
            updateStatus();
            updateNavigation();
        } catch (Exception error) {
            status.setText("Could not display page " + (pageIndex + 1) + ": " + error.getMessage());
        }
    }

    private void updateStatus() {
        status.setText(
            documentName + " · page " + (currentPage + 1) + "/" + pageCount +
            " · " + fidelity + " · offline local open"
        );
    }

    private void updateNavigation() {
        previous.setEnabled(sessionId >= 0 && currentPage > 0);
        next.setEnabled(sessionId >= 0 && currentPage + 1 < pageCount);
        pageJump.setEnabled(sessionId >= 0);
    }

    private void closeCurrentSession() {
        if (sessionId >= 0) {
            NativeReader.closeSession(sessionId);
        }
        sessionId = -1L;
        pageCount = 0;
        currentPage = 0;
    }

    @Override
    protected void onDestroy() {
        if (!isChangingConfigurations()) {
            closeCurrentSession();
        }
        super.onDestroy();
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
