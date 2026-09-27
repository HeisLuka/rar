package com.chaptera.reader;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.RectF;
import android.view.MotionEvent;
import android.view.ScaleGestureDetector;
import android.view.View;
import org.json.JSONArray;
import org.json.JSONObject;

final class PubCanvasView extends View {
    private JSONObject page;
    private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final ScaleGestureDetector scaleDetector;
    private float zoom = 1f;
    private float panX = 0f;
    private float panY = 0f;
    private float lastX;
    private float lastY;

    PubCanvasView(Context context) {
        super(context);
        paint.setTypeface(android.graphics.Typeface.create("sans", android.graphics.Typeface.NORMAL));
        scaleDetector = new ScaleGestureDetector(context, new ScaleGestureDetector.SimpleOnScaleGestureListener() {
            @Override
            public boolean onScale(ScaleGestureDetector detector) {
                applyScaleFactor(detector.getScaleFactor());
                return true;
            }
        });
    }

    void setPage(JSONObject page) {
        this.page = page;
        resetViewport();
        invalidate();
    }

    void restoreViewport(float zoom, float panX, float panY) {
        this.zoom = clamp(zoom, 1f, 8f);
        this.panX = panX;
        this.panY = panY;
        invalidate();
    }

    float getZoom() {
        return zoom;
    }

    float getPanXOffset() {
        return panX;
    }

    float getPanYOffset() {
        return panY;
    }

    void applyScaleFactor(float factor) {
        zoom = clamp(zoom * factor, 1f, 8f);
        invalidate();
    }

    private void resetViewport() {
        zoom = 1f;
        panX = 0f;
        panY = 0f;
    }

    @Override
    public boolean onTouchEvent(MotionEvent event) {
        scaleDetector.onTouchEvent(event);
        if (event.getPointerCount() == 1 && !scaleDetector.isInProgress()) {
            switch (event.getActionMasked()) {
                case MotionEvent.ACTION_DOWN:
                    lastX = event.getX();
                    lastY = event.getY();
                    return true;
                case MotionEvent.ACTION_MOVE:
                    float x = event.getX();
                    float y = event.getY();
                    panX += x - lastX;
                    panY += y - lastY;
                    lastX = x;
                    lastY = y;
                    invalidate();
                    return true;
                default:
                    break;
            }
        }
        return true;
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
        float scale = fitScale * zoom;
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

    private static float clamp(float value, float min, float max) {
        return Math.max(min, Math.min(max, value));
    }
}
