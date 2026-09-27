package com.chaptera.reader;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Intent;
import android.net.Uri;
import android.provider.MediaStore;
import android.view.MotionEvent;
import android.widget.Button;
import android.widget.TextView;
import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import org.junit.Test;
import org.junit.runner.RunWith;

@RunWith(AndroidJUnit4.class)
public final class ResumeInstrumentedTest {
    @Test
    public void relaunchWithoutIntentReopensAuthoritativeUriAndRestoresPosition() throws Exception {
        byte[] fixture = readAsset("SampleNewsletter.pub");
        Uri uri = insertDownload("SampleNewsletter.pub", fixture);
        try {
            android.content.Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
            context.getSharedPreferences("chaptera_reader_resume_v1", android.content.Context.MODE_PRIVATE)
                .edit().clear().commit();

            Intent first = new Intent(Intent.ACTION_VIEW)
                .setClass(context, MainActivity.class)
                .setDataAndType(uri, "application/x-mspublisher")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);

            try (ActivityScenario<MainActivity> scenario = ActivityScenario.launch(first)) {
                scenario.onActivity(activity -> {
                    Button next = activity.findViewById(R.id.reader_next);
                    assertTrue(next.isEnabled());
                    next.performClick();

                    PubCanvasView canvas = activity.findViewById(R.id.reader_canvas);
                    dispatchPan(canvas);

                    activity.getSharedPreferences("chaptera_reader_resume_v1", android.content.Context.MODE_PRIVATE)
                        .edit()
                        .putFloat("zoom", 2.0f)
                        .putFloat("pan_x", canvas.getPanXOffset())
                        .putFloat("pan_y", canvas.getPanYOffset())
                        .commit();
                });
            }

            Intent relaunch = new Intent(context, MainActivity.class);
            try (ActivityScenario<MainActivity> scenario = ActivityScenario.launch(relaunch)) {
                scenario.onActivity(activity -> {
                    TextView status = activity.findViewById(R.id.reader_status);
                    PubCanvasView canvas = activity.findViewById(R.id.reader_canvas);
                    assertTrue("resume should reopen the persisted source URI", status.getText().toString().contains("page 2/"));
                    assertEquals(2.0f, canvas.getZoom(), 0.001f);
                    assertTrue(Math.abs(canvas.getPanXOffset()) > 0f || Math.abs(canvas.getPanYOffset()) > 0f);
                });
            }
        } finally {
            InstrumentationRegistry.getInstrumentation().getTargetContext()
                .getContentResolver().delete(uri, null, null);
        }
    }

    @Test
    public void missingPersistedSourceIsClearedInsteadOfServingCachedDocumentBytes() throws Exception {
        byte[] fixture = readAsset("SampleNewsletter.pub");
        Uri uri = insertDownload("removed-newsletter.pub", fixture);
        android.content.Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        context.getSharedPreferences("chaptera_reader_resume_v1", android.content.Context.MODE_PRIVATE)
            .edit()
            .putString("uri", uri.toString())
            .putInt("page", 3)
            .putFloat("zoom", 2f)
            .commit();
        context.getContentResolver().delete(uri, null, null);

        Intent relaunch = new Intent(context, MainActivity.class);
        try (ActivityScenario<MainActivity> scenario = ActivityScenario.launch(relaunch)) {
            scenario.onActivity(activity -> {
                TextView status = activity.findViewById(R.id.reader_status);
                assertTrue(
                    "deleted source must produce an honest local read failure",
                    status.getText().toString().startsWith("Could not read this local file:")
                        || status.getText().toString().contains("permission")
                );
                String persisted = activity
                    .getSharedPreferences("chaptera_reader_resume_v1", android.content.Context.MODE_PRIVATE)
                    .getString("uri", null);
                assertEquals(null, persisted);
            });
        }
    }

    private static void dispatchPan(PubCanvasView canvas) {
        long now = android.os.SystemClock.uptimeMillis();
        MotionEvent down = MotionEvent.obtain(now, now, MotionEvent.ACTION_DOWN, 100f, 100f, 0);
        MotionEvent move = MotionEvent.obtain(now, now + 20, MotionEvent.ACTION_MOVE, 145f, 125f, 0);
        MotionEvent up = MotionEvent.obtain(now, now + 40, MotionEvent.ACTION_UP, 145f, 125f, 0);
        canvas.dispatchTouchEvent(down);
        canvas.dispatchTouchEvent(move);
        canvas.dispatchTouchEvent(up);
        down.recycle();
        move.recycle();
        up.recycle();
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

    private Uri insertDownload(String name, byte[] bytes) throws Exception {
        ContentResolver resolver = InstrumentationRegistry.getInstrumentation().getTargetContext().getContentResolver();
        ContentValues values = new ContentValues();
        values.put(MediaStore.Downloads.DISPLAY_NAME, name);
        values.put(MediaStore.Downloads.MIME_TYPE, "application/x-mspublisher");
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
