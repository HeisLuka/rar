package com.chaptera.reader;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Intent;
import android.content.pm.ActivityInfo;
import android.net.Uri;
import android.os.SystemClock;
import android.provider.MediaStore;
import android.view.MotionEvent;
import android.widget.Button;
import android.widget.EditText;
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
public final class ReadShellInstrumentedTest {
    @Test
    public void navigation_jump_pan_zoom_and_rotation_keep_one_reader_session() throws Exception {
        byte[] fixture = readAsset("SampleNewsletter.pub");
        Uri uri = insertDownload("SampleNewsletter.pub", fixture);
        ActivityScenario<MainActivity> scenario = null;
        try {
            Intent intent = new Intent(Intent.ACTION_VIEW)
                .setClass(InstrumentationRegistry.getInstrumentation().getTargetContext(), MainActivity.class)
                .setDataAndType(uri, "application/x-mspublisher")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);

            scenario = ActivityScenario.launch(intent);
            final String[] statusBefore = new String[1];
            final float[] viewportBefore = new float[3];

            scenario.onActivity(activity -> {
                TextView status = activity.findViewById(R.id.reader_status);
                Button next = activity.findViewById(R.id.reader_next_page);
                EditText jump = activity.findViewById(R.id.reader_page_jump);
                Button go = activity.findViewById(R.id.reader_page_go);
                PubCanvasView canvas = activity.findViewById(R.id.reader_canvas);

                assertTrue(status.getText().toString().contains("page 1/"));
                assertTrue("fixture should exercise multipage navigation", next.isEnabled());

                next.performClick();
                assertTrue(status.getText().toString().contains("page 2/"));

                jump.setText("1");
                go.performClick();
                assertTrue(status.getText().toString().contains("page 1/"));

                dispatchPan(canvas);
                assertTrue(Math.abs(canvas.getPanXOffset()) > 0f || Math.abs(canvas.getPanYOffset()) > 0f);

                canvas.applyScaleFactor(2f);
                assertTrue("bounded scale operation should increase zoom", canvas.getZoom() > 1f);

                statusBefore[0] = status.getText().toString();
                viewportBefore[0] = canvas.getZoom();
                viewportBefore[1] = canvas.getPanXOffset();
                viewportBefore[2] = canvas.getPanYOffset();

                activity.setRequestedOrientation(ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE);
            });

            InstrumentationRegistry.getInstrumentation().waitForIdleSync();

            scenario.onActivity(activity -> {
                TextView status = activity.findViewById(R.id.reader_status);
                PubCanvasView canvas = activity.findViewById(R.id.reader_canvas);
                assertEquals(statusBefore[0], status.getText().toString());
                assertEquals(viewportBefore[0], canvas.getZoom(), 0.001f);
                assertEquals(viewportBefore[1], canvas.getPanXOffset(), 0.001f);
                assertEquals(viewportBefore[2], canvas.getPanYOffset(), 0.001f);
            });
        } finally {
            if (scenario != null) scenario.close();
            InstrumentationRegistry.getInstrumentation().getTargetContext()
                .getContentResolver().delete(uri, null, null);
        }
    }

    private static void dispatchPan(PubCanvasView canvas) {
        long downTime = SystemClock.uptimeMillis();
        MotionEvent down = MotionEvent.obtain(downTime, downTime, MotionEvent.ACTION_DOWN, 100f, 100f, 0);
        MotionEvent move = MotionEvent.obtain(downTime, downTime + 20, MotionEvent.ACTION_MOVE, 150f, 130f, 0);
        MotionEvent up = MotionEvent.obtain(downTime, downTime + 40, MotionEvent.ACTION_UP, 150f, 130f, 0);
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
