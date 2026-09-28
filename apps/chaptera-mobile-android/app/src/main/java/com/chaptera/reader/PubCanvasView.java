package com.chaptera.reader;

import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.RectF;
import android.view.MotionEvent;
import android.view.ScaleGestureDetector;
import android.view.View;
import java.util.HashMap;
import java.util.Map;
import org.json.JSONArray;
import org.json.JSONObject;

final class PubCanvasView extends View {
    private JSONObject page;
    private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Map<String, Bitmap> imageBitmaps = new HashMap<>();
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
                        byte[] encoded = NativeReader.imageResourceBytes(sessionId, resourceId);
                        if (encoded == null || encoded.length == 0) continue;
                        Bitmap bitmap = BitmapFactory.decodeByteArray(encoded, 0, encoded.length);
                        if (bitmap != null) imageBitmaps.put(resourceId, bitmap);
                    } catch (RuntimeException ignored) {
                        // Viewer diagnostics remain the authority for missing/partial image fidelity.
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
                Bitmap bitmap = imageBitmaps.get(image.optString("resource_id", ""));
                if (bitmap != null) {
                    paint.setStyle(Paint.Style.FILL);
                    canvas.drawBitmap(bitmap, null, rect, paint);
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

    void restoreViewport(float zoom, float panX, float panY) {
        this.userScale = clamp(zoom, 0.5f, 8f);
        this.panX = panX;
        this.panY = panY;
        invalidate();
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
