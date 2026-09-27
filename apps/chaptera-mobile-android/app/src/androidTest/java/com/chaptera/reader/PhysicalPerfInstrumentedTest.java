package com.chaptera.reader;

import static org.junit.Assert.assertTrue;

import android.app.ActivityManager;
import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.os.Process;
import android.os.SystemClock;
import android.provider.MediaStore;
import android.widget.Button;
import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.security.MessageDigest;
import org.json.JSONArray;
import org.json.JSONObject;
import org.junit.Test;
import org.junit.runner.RunWith;

@RunWith(AndroidJUnit4.class)
public final class PhysicalPerfInstrumentedTest {
    @Test
    public void measureRepresentativePhysicalDeviceEnvelope() throws Exception {
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        String[] fixtures = {"Simple.pub", "SampleBrochure.pub", "SampleNewsletter.pub"};
        JSONArray rows = new JSONArray();

        for (String fixtureName : fixtures) {
            byte[] bytes = readAsset(fixtureName);
            Uri uri = insertDownload("Perf-" + fixtureName, bytes);
            try {
                JSONObject row = measureFixture(context, uri, fixtureName, bytes);
                rows.put(row);
            } finally {
                context.getContentResolver().delete(uri, null, null);
            }
        }

        JSONObject receipt = new JSONObject();
        receipt.put("schema", "chaptera.mobile-reader-perf.v1");
        receipt.put("device_model", android.os.Build.MODEL);
        receipt.put("device_manufacturer", android.os.Build.MANUFACTURER);
        receipt.put("device_api", android.os.Build.VERSION.SDK_INT);
        receipt.put("device_abi", android.os.Build.SUPPORTED_ABIS.length == 0 ? "" : android.os.Build.SUPPORTED_ABIS[0]);
        receipt.put("apk_bytes", new File(context.getApplicationInfo().sourceDir).length());
        receipt.put("native_library_bytes", nativeLibraryBytes(context));
        receipt.put("fixtures", rows);
        receipt.put("contains_document_bytes", false);
        receipt.put("contains_recovered_document_text", false);

        File out = new File(context.getFilesDir(), "chaptera-mobile-perf.json");
        try (FileOutputStream stream = new FileOutputStream(out)) {
            stream.write((receipt.toString(2) + "\n").getBytes(java.nio.charset.StandardCharsets.UTF_8));
        }
    }

    private JSONObject measureFixture(Context context, Uri uri, String name, byte[] bytes) throws Exception {
        Intent intent = new Intent(Intent.ACTION_VIEW)
            .setClass(context, MainActivity.class)
            .setDataAndType(uri, "application/x-mspublisher")
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);

        long handoffStart = SystemClock.elapsedRealtimeNanos();
        ActivityScenario<MainActivity> scenario = ActivityScenario.launch(intent);
        final long[] firstUsefulNs = new long[1];
        final long[] pageSwitchNs = new long[1];
        final long[] interactionNs = new long[1];
        final int[] maxPssKb = new int[1];

        try {
            scenario.onActivity(activity -> {
                String status = ((android.widget.TextView) activity.findViewById(R.id.reader_status))
                    .getText().toString();
                assertTrue("fixture must reach a useful page: " + name + " / " + status, status.contains("page 1/"));
                firstUsefulNs[0] = SystemClock.elapsedRealtimeNanos() - handoffStart;
                maxPssKb[0] = Math.max(maxPssKb[0], currentPssKb(activity));

                Button next = activity.findViewById(R.id.reader_next);
                if (next.isEnabled()) {
                    long switchStart = SystemClock.elapsedRealtimeNanos();
                    next.performClick();
                    pageSwitchNs[0] = SystemClock.elapsedRealtimeNanos() - switchStart;
                    maxPssKb[0] = Math.max(maxPssKb[0], currentPssKb(activity));
                }

                PubCanvasView canvas = activity.findViewById(R.id.reader_canvas);
                long interactionStart = SystemClock.elapsedRealtimeNanos();
                canvas.applyScaleFactor(1.5f);
                dispatchPan(canvas);
                interactionNs[0] = SystemClock.elapsedRealtimeNanos() - interactionStart;
                maxPssKb[0] = Math.max(maxPssKb[0], currentPssKb(activity));
            });
        } finally {
            scenario.close();
        }

        JSONObject row = new JSONObject();
        row.put("fixture", name);
        row.put("fixture_bytes", bytes.length);
        row.put("fixture_sha256", hex(MessageDigest.getInstance("SHA-256").digest(bytes)));
        row.put("activity_handoff_to_first_useful_page_ms", nsToMs(firstUsefulNs[0]));
        row.put("page_switch_ms", pageSwitchNs[0] == 0 ? JSONObject.NULL : nsToMs(pageSwitchNs[0]));
        row.put("zoom_pan_dispatch_ms", nsToMs(interactionNs[0]));
        row.put("sampled_max_pss_kb", maxPssKb[0]);
        return row;
    }

    private static int currentPssKb(Context context) {
        ActivityManager manager = (ActivityManager) context.getSystemService(Context.ACTIVITY_SERVICE);
        android.os.Debug.MemoryInfo[] memory = manager.getProcessMemoryInfo(new int[]{Process.myPid()});
        return memory.length == 0 ? 0 : memory[0].getTotalPss();
    }

    private static long nativeLibraryBytes(Context context) {
        File dir = new File(context.getApplicationInfo().nativeLibraryDir);
        File[] files = dir.listFiles();
        if (files == null) return 0L;
        long total = 0L;
        for (File file : files) {
            if (file.isFile()) total += file.length();
        }
        return total;
    }

    private static void dispatchPan(PubCanvasView canvas) {
        long now = SystemClock.uptimeMillis();
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

    private static double nsToMs(long value) {
        return value / 1_000_000.0;
    }

    private static String hex(byte[] bytes) {
        StringBuilder out = new StringBuilder(bytes.length * 2);
        for (byte value : bytes) out.append(String.format("%02x", value & 0xff));
        return out.toString();
    }
}
