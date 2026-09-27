package com.chaptera.reader;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Intent;
import android.content.pm.ActivityInfo;
import android.net.Uri;
import android.provider.MediaStore;
import android.view.View;
import android.widget.Button;
import android.widget.TextView;
import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.security.MessageDigest;
import org.junit.Test;
import org.junit.runner.RunWith;

@RunWith(AndroidJUnit4.class)
public final class V0UserCycleInstrumentedTest {
    @Test
    public void completeOfflineReaderCycleIsRepeatable() throws Exception {
        android.content.Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        context.getSharedPreferences("chaptera_reader_resume_v1", android.content.Context.MODE_PRIVATE)
            .edit().clear().commit();

        byte[] fixture = readAsset("SampleNewsletter.pub");
        Uri firstPub = insertDownload("CycleOne.pub", "application/x-mspublisher", fixture);
        Uri bad = insertDownload(
            "not-publisher.txt",
            "text/plain",
            "plain text is not Publisher".getBytes(java.nio.charset.StandardCharsets.UTF_8)
        );
        Uri secondPub = insertDownload("CycleTwo.pub", "application/x-mspublisher", fixture);

        byte[] firstHashBefore = sha256(readUri(firstPub));
        byte[] secondHashBefore = sha256(readUri(secondPub));

        try {
            Intent first = new Intent(Intent.ACTION_VIEW)
                .setClass(context, MainActivity.class)
                .setDataAndType(firstPub, "application/x-mspublisher")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);

            ActivityScenario<MainActivity> scenario = ActivityScenario.launch(first);
            try {
                scenario.onActivity(activity -> {
                    TextView status = activity.findViewById(R.id.reader_status);
                    Button next = activity.findViewById(R.id.reader_next);
                    PubCanvasView canvas = activity.findViewById(R.id.reader_canvas);

                    assertTrue(status.getText().toString().contains("CycleOne.pub"));
                    assertTrue(status.getText().toString().contains("page 1/"));
                    assertTrue(status.getText().toString().contains("offline local open"));
                    assertTrue(next.isEnabled());

                    next.performClick();
                    assertTrue(status.getText().toString().contains("page 2/"));

                    canvas.applyScaleFactor(2f);
                    dispatchPan(canvas);

                    activity.setRequestedOrientation(ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE);
                });

                InstrumentationRegistry.getInstrumentation().waitForIdleSync();

                scenario.onActivity(activity -> {
                    TextView status = activity.findViewById(R.id.reader_status);
                    PubCanvasView canvas = activity.findViewById(R.id.reader_canvas);
                    assertTrue(status.getText().toString().contains("page 2/"));
                    assertTrue(canvas.getZoom() > 1f);
                    assertTrue(Math.abs(canvas.getPanXOffset()) > 0f || Math.abs(canvas.getPanYOffset()) > 0f);
                });
            } finally {
                scenario.close();
            }

            ActivityScenario<MainActivity> resumed = ActivityScenario.launch(new Intent(context, MainActivity.class));
            try {
                resumed.onActivity(activity -> {
                    TextView status = activity.findViewById(R.id.reader_status);
                    PubCanvasView canvas = activity.findViewById(R.id.reader_canvas);
                    assertTrue("resume must reopen original local URI", status.getText().toString().contains("CycleOne.pub"));
                    assertTrue(status.getText().toString().contains("page 2/"));
                    assertTrue(canvas.getZoom() > 1f);
                });
            } finally {
                resumed.close();
            }

            Intent failureIntent = new Intent(Intent.ACTION_VIEW)
                .setClass(context, MainActivity.class)
                .setDataAndType(bad, "text/plain")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            ActivityScenario<MainActivity> failed = ActivityScenario.launch(failureIntent);
            try {
                failed.onActivity(activity -> {
                    TextView status = activity.findViewById(R.id.reader_status);
                    Button retry = activity.findViewById(R.id.reader_retry);
                    Button chooseAnother = activity.findViewById(R.id.reader_choose_another);
                    Button diagnostics = activity.findViewById(R.id.reader_diagnostics);
                    assertTrue(status.getText().toString().startsWith("This is not a Publisher file."));
                    assertEquals(View.VISIBLE, retry.getVisibility());
                    assertEquals(View.VISIBLE, chooseAnother.getVisibility());
                    assertEquals(View.VISIBLE, diagnostics.getVisibility());
                });
            } finally {
                failed.close();
            }

            Intent second = new Intent(Intent.ACTION_VIEW)
                .setClass(context, MainActivity.class)
                .setDataAndType(secondPub, "application/x-mspublisher")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            ActivityScenario<MainActivity> secondScenario = ActivityScenario.launch(second);
            try {
                secondScenario.onActivity(activity -> {
                    TextView status = activity.findViewById(R.id.reader_status);
                    assertTrue(status.getText().toString().contains("CycleTwo.pub"));
                    assertTrue(status.getText().toString().contains("page 1/"));
                    assertTrue(status.getText().toString().contains("offline local open"));
                });
            } finally {
                secondScenario.close();
            }

            assertArrayEquals(firstHashBefore, sha256(readUri(firstPub)));
            assertArrayEquals(secondHashBefore, sha256(readUri(secondPub)));
        } finally {
            ContentResolver resolver = context.getContentResolver();
            resolver.delete(firstPub, null, null);
            resolver.delete(bad, null, null);
            resolver.delete(secondPub, null, null);
            context.getSharedPreferences("chaptera_reader_resume_v1", android.content.Context.MODE_PRIVATE)
                .edit().clear().commit();
        }
    }

    private static void dispatchPan(PubCanvasView canvas) {
        long now = android.os.SystemClock.uptimeMillis();
        android.view.MotionEvent down = android.view.MotionEvent.obtain(now, now, android.view.MotionEvent.ACTION_DOWN, 100f, 100f, 0);
        android.view.MotionEvent move = android.view.MotionEvent.obtain(now, now + 20, android.view.MotionEvent.ACTION_MOVE, 150f, 130f, 0);
        android.view.MotionEvent up = android.view.MotionEvent.obtain(now, now + 40, android.view.MotionEvent.ACTION_UP, 150f, 130f, 0);
        canvas.dispatchTouchEvent(down);
        canvas.dispatchTouchEvent(move);
        canvas.dispatchTouchEvent(up);
        down.recycle();
        move.recycle();
        up.recycle();
    }

    private byte[] readAsset(String name) throws Exception {
        try (InputStream input = InstrumentationRegistry.getInstrumentation().getContext().getAssets().open(name)) {
            return readAll(input);
        }
    }

    private byte[] readUri(Uri uri) throws Exception {
        try (InputStream input = InstrumentationRegistry.getInstrumentation().getTargetContext()
            .getContentResolver().openInputStream(uri)) {
            if (input == null) throw new IllegalStateException("content provider returned no stream");
            return readAll(input);
        }
    }

    private static byte[] readAll(InputStream input) throws Exception {
        ByteArrayOutputStream output = new ByteArrayOutputStream();
        byte[] buffer = new byte[64 * 1024];
        for (;;) {
            int read = input.read(buffer);
            if (read < 0) break;
            output.write(buffer, 0, read);
        }
        return output.toByteArray();
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

    private static byte[] sha256(byte[] bytes) throws Exception {
        return MessageDigest.getInstance("SHA-256").digest(bytes);
    }
}
