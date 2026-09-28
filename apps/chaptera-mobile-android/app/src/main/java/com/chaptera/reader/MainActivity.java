package com.chaptera.reader;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.provider.OpenableColumns;
import android.text.InputType;
import android.view.Gravity;
import android.view.View;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.TextView;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.security.MessageDigest;
import org.json.JSONArray;
import org.json.JSONObject;

public final class MainActivity extends Activity {
    private static final int OPEN_DOCUMENT = 1001;
    private static final String RESUME_PREFS = "chaptera_reader_resume_v1";
    private static final String PREF_URI = "uri";
    private static final String PREF_PAGE = "page";
    private static final String PREF_ZOOM = "zoom";
    private static final String PREF_PAN_X = "pan_x";
    private static final String PREF_PAN_Y = "pan_y";

    private TextView status;
    private TextView diagnosticsView;
    private PubCanvasView canvas;
    private Button previousPage;
    private Button nextPage;
    private EditText pageJump;
    private Button retry;
    private Button chooseAnother;
    private Button failureDiagnostics;
    private long sessionId;
    private int currentPage;
    private int pageCount;
    private String documentName = "Local PUB";
    private String fidelity = "unknown";
    private Uri lastUri;
    private Uri currentUri;
    private String lastDiagnosticJson;

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
        previousPage = new Button(this);
        previousPage.setId(com.chaptera.reader.R.id.reader_previous_page);
        previousPage.setText("Previous");
        previousPage.setEnabled(false);
        previousPage.setOnClickListener(v -> renderPage(currentPage - 1));
        navigation.addView(previousPage, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));

        pageJump = new EditText(this);
        pageJump.setId(com.chaptera.reader.R.id.reader_page_jump);
        pageJump.setSingleLine(true);
        pageJump.setGravity(Gravity.CENTER);
        pageJump.setInputType(InputType.TYPE_CLASS_NUMBER);
        pageJump.setHint("Page");
        pageJump.setEnabled(false);
        navigation.addView(pageJump, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));

        Button go = new Button(this);
        go.setId(com.chaptera.reader.R.id.reader_page_go);
        go.setText("Go");
        go.setOnClickListener(v -> {
            try {
                int requested = Integer.parseInt(pageJump.getText().toString().trim()) - 1;
                renderPage(requested);
            } catch (NumberFormatException ignored) {
                status.setText("Enter a page number between 1 and " + Math.max(1, pageCount) + ".");
            }
        });
        navigation.addView(go, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 0.6f));

        nextPage = new Button(this);
        nextPage.setId(com.chaptera.reader.R.id.reader_next_page);
        nextPage.setText("Next");
        nextPage.setEnabled(false);
        nextPage.setOnClickListener(v -> renderPage(currentPage + 1));
        navigation.addView(nextPage, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
        root.addView(navigation, new LinearLayout.LayoutParams(
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

        LinearLayout recovery = new LinearLayout(this);
        recovery.setOrientation(LinearLayout.HORIZONTAL);

        retry = new Button(this);
        retry.setId(com.chaptera.reader.R.id.reader_retry);
        retry.setText("Retry");
        retry.setVisibility(View.GONE);
        retry.setOnClickListener(v -> {
            if (lastUri != null) openUri(lastUri);
        });
        recovery.addView(retry, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));

        chooseAnother = new Button(this);
        chooseAnother.setId(com.chaptera.reader.R.id.reader_choose_another);
        chooseAnother.setText("Choose another file");
        chooseAnother.setVisibility(View.GONE);
        chooseAnother.setOnClickListener(v -> chooseDocument());
        recovery.addView(chooseAnother, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));

        failureDiagnostics = new Button(this);
        failureDiagnostics.setId(com.chaptera.reader.R.id.reader_failure_diagnostics);
        failureDiagnostics.setText("Diagnostics");
        failureDiagnostics.setVisibility(View.GONE);
        failureDiagnostics.setOnClickListener(v -> showFailureDiagnostics());
        recovery.addView(failureDiagnostics, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));

        root.addView(recovery, new LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT,
            LinearLayout.LayoutParams.WRAP_CONTENT
        ));

        diagnosticsView = new TextView(this);
        diagnosticsView.setId(com.chaptera.reader.R.id.reader_viewer_diagnostics);
        diagnosticsView.setVisibility(View.GONE);
        root.addView(diagnosticsView, new LinearLayout.LayoutParams(
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
        if (handedOff != null) {
            openUri(handedOff, null);
        } else {
            resumeLastDocument();
        }
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
        openUri(uri, null);
    }

    private void openUri(Uri uri, ResumeState resume) {
        lastUri = uri;
        clearFailureUi();
        try {
            int maxInputBytes = Math.toIntExact(NativeReader.maxInputBytes());
            Long declaredBytes = declaredSize(uri);
            if (declaredBytes != null && declaredBytes > maxInputBytes) {
                throw new IllegalArgumentException(
                    "file exceeds local Reader size limit before read (" + declaredBytes + " bytes)"
                );
            }
            byte[] bytes = readBounded(uri, maxInputBytes);
            String before = sha256(bytes);
            String wire = NativeReader.openSessionJson(bytes);
            String after = sha256(bytes);
            if (!before.equals(after)) {
                throw new IllegalStateException("Reader core mutated the supplied source bytes");
            }
            if (wire.startsWith("ERR:")) {
                lastDiagnosticJson = NativeReader.failureDiagnosticJson(bytes);
                FailurePresentation failure = FailurePresentation.fromDiagnosticJson(lastDiagnosticJson);
                showFailure(failure);
                if (resume != null) clearResumeState();
                closeCurrentSession();
                currentUri = null;
                canvas.setPage(null);
                return;
            }

            JSONObject receipt = new JSONObject(wire);
            long openedSession = receipt.getLong("session_id");
            int pages = receipt.getInt("page_count");
            if (pages <= 0) {
                NativeReader.closeSession(openedSession);
                throw new IllegalStateException("PUB opened without readable pages");
            }

            closeCurrentSession();
            sessionId = openedSession;
            pageCount = pages;
            currentPage = 0;
            documentName = displayName(uri);
            fidelity = receipt.optString("fidelity", "unknown");
            currentUri = uri;
            showDiagnostics(receipt.optJSONArray("diagnostics"));
            renderPage(0);

            if (resume != null) {
                int restoredPage = Math.max(0, Math.min(resume.pageIndex, pageCount - 1));
                if (restoredPage != 0) {
                    renderPage(restoredPage);
                }
                canvas.restoreViewport(resume.zoom, resume.panX, resume.panY);
                persistResumeState();
            }
        } catch (SecurityException denied) {
            showFailure(FailurePresentation.accessDenied());
            clearResumeState();
            closeCurrentSession();
            currentUri = null;
            canvas.setPage(null);
        } catch (Exception error) {
            showFailure(FailurePresentation.providerUnavailable(error.getMessage()));
            if (resume != null) clearResumeState();
            closeCurrentSession();
            currentUri = null;
            canvas.setPage(null);
        }
    }

    private void renderPage(int pageIndex) {
        if (sessionId <= 0L) return;
        if (pageIndex < 0 || pageIndex >= pageCount) {
            status.setText("Page must be between 1 and " + Math.max(1, pageCount) + ".");
            return;
        }
        String wire = NativeReader.pageRenderPlanJson(sessionId, pageIndex);
        if (wire.startsWith("ERR:")) {
            status.setText("Could not render page " + (pageIndex + 1) + ": " + wire);
            return;
        }
        try {
            JSONObject page = new JSONObject(wire);
            currentPage = pageIndex;
            pageJump.setText(Integer.toString(currentPage + 1));
            pageJump.setEnabled(true);
            canvas.setPage(page, sessionId);
            status.setText(
                documentName + " · page " + (currentPage + 1) + "/" + pageCount
                    + " · " + fidelity + " · offline local open"
            );
            previousPage.setEnabled(currentPage > 0);
            nextPage.setEnabled(currentPage + 1 < pageCount);
            persistResumeState();
        } catch (Exception error) {
            status.setText("Could not decode render plan: " + error.getMessage());
        }
    }

    private void showFailure(FailurePresentation failure) {
        status.setText(failure.title + ". " + failure.message);
        retry.setVisibility(View.VISIBLE);
        chooseAnother.setVisibility(View.VISIBLE);
        failureDiagnostics.setVisibility(lastDiagnosticJson == null ? View.GONE : View.VISIBLE);
    }

    private void clearFailureUi() {
        lastDiagnosticJson = null;
        if (retry != null) retry.setVisibility(View.GONE);
        if (chooseAnother != null) chooseAnother.setVisibility(View.GONE);
        if (failureDiagnostics != null) failureDiagnostics.setVisibility(View.GONE);
    }

    private void showFailureDiagnostics() {
        if (lastDiagnosticJson == null) return;
        String bounded = lastDiagnosticJson.length() > 4000
            ? lastDiagnosticJson.substring(0, 4000) + "\n…"
            : lastDiagnosticJson;
        new AlertDialog.Builder(this)
            .setTitle("Local diagnostics")
            .setMessage(bounded)
            .setPositiveButton("Close", null)
            .show();
    }

    private void showDiagnostics(JSONArray diagnostics) {
        if (diagnosticsView == null) return;
        if (diagnostics == null || diagnostics.length() == 0) {
            diagnosticsView.setText("");
            diagnosticsView.setVisibility(View.GONE);
            return;
        }

        StringBuilder text = new StringBuilder("Viewer diagnostics:");
        int shown = Math.min(diagnostics.length(), 4);
        for (int i = 0; i < shown; i++) {
            JSONObject diagnostic = diagnostics.optJSONObject(i);
            if (diagnostic == null) continue;
            text.append("\n• ")
                .append(diagnostic.optString("code", "viewer.warning"));
            String message = diagnostic.optString("message", "");
            if (!message.isEmpty()) text.append(": ").append(message);
        }
        if (diagnostics.length() > shown) {
            text.append("\n• +").append(diagnostics.length() - shown).append(" more");
        }
        diagnosticsView.setText(text.toString());
        diagnosticsView.setVisibility(View.VISIBLE);
    }

    private void closeCurrentSession() {
        if (sessionId > 0L) {
            NativeReader.closeSession(sessionId);
            sessionId = 0L;
        }
        pageCount = 0;
        currentPage = 0;
        if (diagnosticsView != null) {
            diagnosticsView.setText("");
            diagnosticsView.setVisibility(View.GONE);
        }
        if (previousPage != null) previousPage.setEnabled(false);
        if (nextPage != null) nextPage.setEnabled(false);
        if (pageJump != null) {
            pageJump.setText("");
            pageJump.setEnabled(false);
        }
    }

    @Override
    protected void onStop() {
        persistResumeState();
        super.onStop();
    }

    private void resumeLastDocument() {
        android.content.SharedPreferences prefs = getSharedPreferences(RESUME_PREFS, MODE_PRIVATE);
        String rawUri = prefs.getString(PREF_URI, null);
        if (rawUri == null || rawUri.isEmpty()) return;

        ResumeState state = new ResumeState(
            prefs.getInt(PREF_PAGE, 0),
            prefs.getFloat(PREF_ZOOM, 1f),
            prefs.getFloat(PREF_PAN_X, 0f),
            prefs.getFloat(PREF_PAN_Y, 0f)
        );
        openUri(Uri.parse(rawUri), state);
    }

    private void persistResumeState() {
        if (sessionId <= 0L || currentUri == null || canvas == null) return;
        getSharedPreferences(RESUME_PREFS, MODE_PRIVATE)
            .edit()
            .putString(PREF_URI, currentUri.toString())
            .putInt(PREF_PAGE, currentPage)
            .putFloat(PREF_ZOOM, canvas.getZoom())
            .putFloat(PREF_PAN_X, canvas.getPanXOffset())
            .putFloat(PREF_PAN_Y, canvas.getPanYOffset())
            .apply();
    }

    private void clearResumeState() {
        getSharedPreferences(RESUME_PREFS, MODE_PRIVATE).edit().clear().apply();
    }

    private static final class ResumeState {
        final int pageIndex;
        final float zoom;
        final float panX;
        final float panY;

        ResumeState(int pageIndex, float zoom, float panX, float panY) {
            this.pageIndex = pageIndex;
            this.zoom = zoom;
            this.panX = panX;
            this.panY = panY;
        }
    }

    @Override
    protected void onDestroy() {
        closeCurrentSession();
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

    private Long declaredSize(Uri uri) {
        try (android.database.Cursor cursor = getContentResolver().query(
            uri, new String[]{OpenableColumns.SIZE}, null, null, null
        )) {
            if (cursor != null && cursor.moveToFirst()) {
                int index = cursor.getColumnIndex(OpenableColumns.SIZE);
                if (index >= 0 && !cursor.isNull(index)) {
                    long size = cursor.getLong(index);
                    if (size >= 0L) return size;
                }
            }
        } catch (Exception ignored) {
            // Unknown provider metadata falls back to the streaming read bound below.
        }
        return null;
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
