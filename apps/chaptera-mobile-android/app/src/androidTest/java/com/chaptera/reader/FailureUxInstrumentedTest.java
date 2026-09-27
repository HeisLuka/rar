package com.chaptera.reader;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Intent;
import android.net.Uri;
import android.provider.MediaStore;
import android.view.View;
import android.widget.Button;
import android.widget.TextView;
import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.OutputStream;
import org.junit.Test;
import org.junit.runner.RunWith;

@RunWith(AndroidJUnit4.class)
public final class FailureUxInstrumentedTest {
    @Test
    public void nonPubContentOffersBoundedRecoveryActions() throws Exception {
        byte[] bytes = "plain text is not a Publisher document".getBytes(java.nio.charset.StandardCharsets.UTF_8);
        Uri uri = insertDownload("not-publisher.txt", "text/plain", bytes);
        try {
            Intent intent = new Intent(Intent.ACTION_VIEW)
                .setClass(InstrumentationRegistry.getInstrumentation().getTargetContext(), MainActivity.class)
                .setDataAndType(uri, "text/plain")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);

            try (ActivityScenario<MainActivity> scenario = ActivityScenario.launch(intent)) {
                scenario.onActivity(activity -> {
                    TextView status = activity.findViewById(R.id.reader_status);
                    Button retry = activity.findViewById(R.id.reader_retry);
                    Button chooseAnother = activity.findViewById(R.id.reader_choose_another);
                    Button diagnostics = activity.findViewById(R.id.reader_diagnostics);

                    assertTrue(status.getText().toString().startsWith("This is not a Publisher file."));
                    assertEquals(View.VISIBLE, retry.getVisibility());
                    assertEquals(View.VISIBLE, chooseAnother.getVisibility());
                    assertEquals(View.VISIBLE, diagnostics.getVisibility());
                });
            }
        } finally {
            InstrumentationRegistry.getInstrumentation().getTargetContext()
                .getContentResolver().delete(uri, null, null);
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
