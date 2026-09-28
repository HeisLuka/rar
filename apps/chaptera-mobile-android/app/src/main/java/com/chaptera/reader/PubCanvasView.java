package com.chaptera.reader;

import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.RectF;
import android.view.MotionEvent;
import android.view.ScaleGestureDetector;
import android.view.View;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.HashMap;
import java.util.Map;
import org.json.JSONArray;
import org.json.JSONObject;

final class PubCanvasView extends View {
    private JSONObject page;
    private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private static final byte[] ARGB_ENVELOPE_MAGIC = new byte[]{'C', 'H', 'A', 'R', 'G', 'B', '1', 0};
    private final Map<String, Bitmap> imageBitmaps = new HashMap<>();
    private final Map<String, String> imageDiagnostics = new HashMap<>();
    private final ScaleGestureDetector scaleDetector;
    private float userScale = 1f;
    private float panX;
    private float panY;
    private float lastTouchX;
    private float lastTouchY;

    PubCanvasView(Context context) {
        super(context);
        paint.setTypeface(android.graphics.Typeface.create("sans", android.graphics.Typeface.NORMAL));
        scaleDetector = new ScaleGestureDetector(context, new ScaleGestureDetector.SimpleOnScaleGestureListener() {
            @Override
            public boolean onScale(ScaleGestureDetector detector) {
                userScale = clamp(userScale * detector.getScaleFactor(), 0.5f, 8f);
                invalidate();
                return true;
            }
        });
    }

    void setPage(JSONObject page) {
        setPage(page, 0L);
    }

    void setPage(JSONObject page, long sessionId) {
        this.page = page;
        imageBitmaps.clear();
        imageDiagnostics.clear();
        userScale = 1f;
        panX = 0f;
        panY = 0f;

        if (page != null && sessionId > 0L) {
            JSONArray nodes = page.optJSONArray("nodes");
            if (nodes != null) {
                for (int i = 0; i < nodes.length(); i++) {
                    JSONObject node = nodes.optJSONObject(i);
                    JSONObject image = node == null ? null : node.optJSONObject("image");
                    if (image == null) continue;
                    String resourceId = image.optString("resource_id", "");
                    if (resourceId.isEmpty() || imageBitmaps.containsKey(resourceId)) continue;
                    try {
                        byte[] admitted = NativeReader.imageResourceArgb8(sessionId, resourceId);
                        Bitmap bitmap = bitmapFromAdmittedArgb8(admitted);
                        imageBitmaps.put(resourceId, bitmap);
                    } catch (RuntimeException error) {
                        imageDiagnostics.put(resourceId, boundedImageDiagnosticCode(error));
                    }
                }
            }
        }
        invalidate();
    }

    @Override
    protected void onDraw(Canvas canvas) {
        super.onDraw(canvas);
        canvas.drawColor(Color.rgb(238, 238, 238));
        if (page == null) return;

        JSONObject size = page.optJSONObject("page_size");
        if (size == null) return;
        float pageWidth = (float) size.optLong("width", 1);
        float pageHeight = (float) size.optLong("height", 1);
        if (pageWidth <= 0 || pageHeight <= 0) return;

        float margin = 24f;
        float fitScale = Math.min(
            Math.max(1f, getWidth() - margin * 2f) / pageWidth,
            Math.max(1f, getHeight() - margin * 2f) / pageHeight
        );
        float scale = fitScale * userScale;
        float drawWidth = pageWidth * scale;
        float drawHeight = pageHeight * scale;
        float ox = (getWidth() - drawWidth) / 2f + panX;
        float oy = margin + panY;

        paint.setStyle(Paint.Style.FILL);
        paint.setColor(Color.WHITE);
        canvas.drawRect(ox, oy, ox + drawWidth, oy + drawHeight, paint);
        paint.setStyle(Paint.Style.STROKE);
        paint.setStrokeWidth(1f);
        paint.setColor(Color.DKGRAY);
        canvas.drawRect(ox, oy, ox + drawWidth, oy + drawHeight, paint);

        JSONArray nodes = page.optJSONArray("nodes");
        if (nodes == null) return;
        for (int i = 0; i < nodes.length(); i++) {
            JSONObject node = nodes.optJSONObject(i);
            if (node == null) continue;
            JSONObject bounds = node.optJSONObject("bounds");
            if (bounds == null) continue;

            float x = ox + (float) bounds.optLong("x") * scale;
            float y = oy + (float) bounds.optLong("y") * scale;
            float w = (float) bounds.optLong("width") * scale;
            float h = (float) bounds.optLong("height") * scale;
            if (w <= 0 || h <= 0) continue;
            RectF rect = new RectF(x, y, x + w, y + h);

            JSONArray fill = node.optJSONArray("solid_fill_rgb");
            if (fill != null && fill.length() == 3) {
                paint.setStyle(Paint.Style.FILL);
                paint.setColor(Color.rgb(fill.optInt(0), fill.optInt(1), fill.optInt(2)));
                canvas.drawRect(rect, paint);
            }

            JSONObject image = node.optJSONObject("image");
            if (image != null) {
                String resourceId = image.optString("resource_id", "");
                Bitmap bitmap = imageBitmaps.get(resourceId);
                if (bitmap != null) {
                    paint.setStyle(Paint.Style.FILL);
                    canvas.drawBitmap(bitmap, null, rect, paint);
                } else {
                    String diagnostic = imageDiagnostics.get(resourceId);
                    if (diagnostic != null) {
                        paint.setStyle(Paint.Style.STROKE);
                        paint.setStrokeWidth(1f);
                        paint.setColor(Color.DKGRAY);
                        canvas.drawRect(rect, paint);
                        paint.setStyle(Paint.Style.FILL);
                        paint.setTextSize(Math.max(10f, Math.min(18f, 12f * userScale)));
                        canvas.drawText(
                            "Image unavailable (" + diagnostic + ")",
                            rect.left + 2f,
                            rect.top + Math.max(12f, paint.getTextSize()),
                            paint
                        );
                    }
                }
            }

            JSONObject line = node.optJSONObject("solid_line");
            if (line != null) {
                JSONArray rgb = line.optJSONArray("rgb");
                if (rgb != null && rgb.length() == 3) {
                    paint.setColor(Color.rgb(rgb.optInt(0), rgb.optInt(1), rgb.optInt(2)));
                    paint.setStyle(Paint.Style.STROKE);
                    paint.setStrokeWidth(Math.max(1f, (float) line.optLong("width_emu") * scale));
                    canvas.drawRect(rect, paint);
                }
            }

            JSONObject text = node.optJSONObject("text");
            if (text != null) {
                String value = text.optString("text", "");
                if (!value.isEmpty()) {
                    paint.setStyle(Paint.Style.FILL);
                    paint.setColor(Color.BLACK);
                    paint.setTextSize(Math.max(10f, 114300f * scale));
                    canvas.save();
                    canvas.clipRect(rect);
                    float baseline = rect.top + Math.max(paint.getTextSize(), 12f);
                    canvas.drawText(value.replace('\n', ' '), rect.left + 2f, baseline, paint);
                    canvas.restore();
                }
            }
        }
    }

    static Bitmap bitmapFromAdmittedArgb8(byte[] envelope) {
        if (envelope == null || envelope.length < 16) {
            throw new IllegalArgumentException("admitted image envelope is truncated");
        }
        for (int i = 0; i < ARGB_ENVELOPE_MAGIC.length; i++) {
            if (envelope[i] != ARGB_ENVELOPE_MAGIC[i]) {
                throw new IllegalArgumentException("admitted image envelope magic mismatch");
            }
        }

        ByteBuffer buffer = ByteBuffer.wrap(envelope).order(ByteOrder.BIG_ENDIAN);
        buffer.position(8);
        int width = buffer.getInt();
        int height = buffer.getInt();
        if (width <= 0 || height <= 0) {
            throw new IllegalArgumentException("admitted image dimensions must be positive");
        }

        long pixelCountLong = (long) width * (long) height;
        if (pixelCountLong > Integer.MAX_VALUE) {
            throw new IllegalArgumentException("admitted image pixel count exceeds Android array limit");
        }
        long expectedLength = 16L + pixelCountLong * 4L;
        if (expectedLength != envelope.length) {
            throw new IllegalArgumentException("admitted image envelope length mismatch");
        }

        int[] colors = new int[(int) pixelCountLong];
        for (int i = 0; i < colors.length; i++) {
            colors[i] = buffer.getInt();
        }
        return Bitmap.createBitmap(colors, width, height, Bitmap.Config.ARGB_8888);
    }

    String imageDiagnosticForResource(String resourceId) {
        return imageDiagnostics.get(resourceId);
    }

    private static String boundedImageDiagnosticCode(RuntimeException error) {
        String message = error.getMessage();
        if (message == null || message.isEmpty()) return "image_backend_error";
        int marker = message.indexOf("IMAGE_DECODE:");
        if (marker < 0) return "image_backend_error";
        String rest = message.substring(marker + "IMAGE_DECODE:".length());
        int separator = rest.indexOf(':');
        String code = separator < 0 ? rest : rest.substring(0, separator);
        if (code.isEmpty() || code.length() > 80) return "image_decode_rejected";
        return code;
    }

    @Override
    public boolean onTouchEvent(MotionEvent event) {
        scaleDetector.onTouchEvent(event);

        switch (event.getActionMasked()) {
            case MotionEvent.ACTION_DOWN:
                lastTouchX = event.getX();
                lastTouchY = event.getY();
                return true;
            case MotionEvent.ACTION_MOVE:
                if (!scaleDetector.isInProgress() && event.getPointerCount() == 1) {
                    float x = event.getX();
                    float y = event.getY();
                    panX += x - lastTouchX;
                    panY += y - lastTouchY;
                    lastTouchX = x;
                    lastTouchY = y;
                    invalidate();
                }
                return true;
            case MotionEvent.ACTION_POINTER_UP:
            case MotionEvent.ACTION_UP:
            case MotionEvent.ACTION_CANCEL:
                if (event.getPointerCount() > 0) {
                    lastTouchX = event.getX(0);
                    lastTouchY = event.getY(0);
                }
                return true;
            default:
                return true;
        }
    }

    float getZoom() {
        return userScale;
    }

    float getPanXOffset() {
        return panX;
    }

    float getPanYOffset() {
        return panY;
    }

    void applyScaleFactor(float factor) {
        userScale = clamp(userScale * factor, 0.5f, 8f);
        invalidate();
    }

    private static float clamp(float value, float min, float max) {
        return Math.max(min, Math.min(max, value));
    }

}
