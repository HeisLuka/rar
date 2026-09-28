package com.chaptera.reader;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Intent;
import android.content.pm.PackageInfo;
import android.net.Uri;
import android.provider.MediaStore;
import android.widget.TextView;
import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.Arrays;
import org.junit.Test;
import org.junit.runner.RunWith;

@RunWith(AndroidJUnit4.class)
public final class LocalOpenInstrumentedTest {
    @Test
    public void realPubArrivesThroughContentUriAndReachesFirstUsefulPage() throws Exception {
        byte[] fixture = readAsset("SampleNewsletter.pub");
        Uri uri = insertDownload("SampleNewsletter.pub", "application/x-mspublisher", fixture);
        try {
            Intent intent = new Intent(Intent.ACTION_VIEW)
                .setClass(InstrumentationRegistry.getInstrumentation().getTargetContext(), MainActivity.class)
                .setDataAndType(uri, "application/x-mspublisher")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);

            try (ActivityScenario<MainActivity> scenario = ActivityScenario.launch(intent)) {
                scenario.onActivity(activity -> {
                    TextView status = activity.findViewById(R.id.reader_status);
                    String value = status.getText().toString();
                    assertTrue("expected first rendered page status, got: " + value, value.contains("page 1/"));
                    assertTrue("expected offline locality marker, got: " + value, value.contains("offline local open"));
                    assertFalse("real PUB must not fall into local open error", value.startsWith("Could not"));
                    assertTrue(activity.findViewById(R.id.reader_canvas).isShown());

                    android.widget.Button next = activity.findViewById(R.id.reader_next_page);
                    if (next.isEnabled()) {
                        assertTrue("next-page click must be handled", next.performClick());
                        String pageTwo = ((TextView) activity.findViewById(R.id.reader_status)).getText().toString();
                        assertTrue("expected page 2 after navigation, got: " + pageTwo, pageTwo.contains("page 2/"));
                    } else {
                        assertTrue("disabled Next is valid only for a single-page fixture: " + value, value.contains("page 1/1"));
                    }
                });
            }
        } finally {
            InstrumentationRegistry.getInstrumentation().getTargetContext()
                .getContentResolver().delete(uri, null, null);
        }
    }

    @Test
    public void sharedSessionExposesExactEmbeddedImageBytes() throws Exception {
        byte[] fixture = readAsset("SampleNewsletter.pub");
        String receiptWire = NativeReader.openSessionJson(fixture);
        assertFalse("session open failed: " + receiptWire, receiptWire.startsWith("ERR:"));

        org.json.JSONObject receipt = new org.json.JSONObject(receiptWire);
        long sessionId = receipt.getLong("session_id");
        try {
            String planWire = NativeReader.pageRenderPlanJson(sessionId, 0);
            assertFalse("page render plan failed: " + planWire, planWire.startsWith("ERR:"));
            org.json.JSONObject page = new org.json.JSONObject(planWire);
            org.json.JSONArray nodes = page.getJSONArray("nodes");

            boolean foundImage = false;
            for (int i = 0; i < nodes.length(); i++) {
                org.json.JSONObject image = nodes.getJSONObject(i).optJSONObject("image");
                if (image == null) continue;
                String resourceId = image.getString("resource_id");
                byte[] encoded = NativeReader.imageResourceBytes(sessionId, resourceId);
                assertTrue("embedded image bytes must be non-empty", encoded != null && encoded.length > 0);
                assertTrue(
                    "embedded image bytes must decode on Android",
                    android.graphics.BitmapFactory.decodeByteArray(encoded, 0, encoded.length) != null
                );
                foundImage = true;
                break;
            }
            assertTrue("real PUB fixture must expose at least one render-plan image", foundImage);
        } finally {
            NativeReader.closeSession(sessionId);
        }
    }

    @Test
    public void nonPubContentUriReturnsBoundedLocalFailure() throws Exception {
        byte[] bytes = "not a publisher document".getBytes(java.nio.charset.StandardCharsets.UTF_8);
        Uri uri = insertDownload("not-pub.bin", "application/octet-stream", bytes);
        try {
            Intent intent = new Intent(Intent.ACTION_VIEW)
                .setClass(InstrumentationRegistry.getInstrumentation().getTargetContext(), MainActivity.class)
                .setDataAndType(uri, "application/octet-stream")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);

            try (ActivityScenario<MainActivity> scenario = ActivityScenario.launch(intent)) {
                scenario.onActivity(activity -> {
                    TextView status = activity.findViewById(R.id.reader_status);
                    String value = status.getText().toString();
                    assertTrue("non-PUB must produce bounded local error: " + value,
                        value.startsWith("Could not open this file locally."));
                });
            }
        } finally {
            InstrumentationRegistry.getInstrumentation().getTargetContext()
                .getContentResolver().delete(uri, null, null);
        }
    }

    @Test
    public void applicationDoesNotRequestInternetPermission() throws Exception {
        android.content.Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        PackageInfo info = context.getPackageManager().getPackageInfo(
            context.getPackageName(),
            android.content.pm.PackageManager.GET_PERMISSIONS
        );
        String[] permissions = info.requestedPermissions == null ? new String[0] : info.requestedPermissions;
        assertFalse(Arrays.asList(permissions).contains("android.permission.INTERNET"));
    }

    private byte[] readAsset(String name) throws Exception {
        try (InputStream input = InstrumentationRegistry.getInstrumentation().getContext().getAssets().open(name)) {
            ByteArrayOutputStream output = new ByteArrayOutputStream();
            byte[] buffer = new byte[64 * 1024];
            for (;;) {
                int read = input.read(buffer);
                if (read < 0) break;
                output.write(buffer, 0, read);
            }
            return output.toByteArray();
        }
    }

    private Uri insertDownload(String name, String mime, byte[] bytes) throws Exception {
        ContentResolver resolver = InstrumentationRegistry.getInstrumentation().getTargetContext().getContentResolver();
        ContentValues values = new ContentValues();
        values.put(MediaStore.Downloads.DISPLAY_NAME, name);
        values.put(MediaStore.Downloads.MIME_TYPE, mime);
        values.put(MediaStore.Downloads.IS_PENDING, 1);
        Uri uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values);
        if (uri == null) throw new IllegalStateException("failed to create MediaStore content URI");
        try (OutputStream output = resolver.openOutputStream(uri)) {
            if (output == null) throw new IllegalStateException("failed to open MediaStore output");
            output.write(bytes);
        }
        ContentValues ready = new ContentValues();
        ready.put(MediaStore.Downloads.IS_PENDING, 0);
        resolver.update(uri, ready, null, null);
        return uri;
    }
}
