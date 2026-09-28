package com.chaptera.reader;

final class NativeReader {
    static {
        System.loadLibrary("chaptera_mobile_reader_jni");
    }

    private NativeReader() {}

    static native String openLocalPubJson(byte[] bytes);
    static native String openSessionJson(byte[] bytes);
    static native String pageRenderPlanJson(long sessionId, long pageIndex);
    static native byte[] imageResourceArgb8(long sessionId, String resourceKey);
    static native void closeSession(long sessionId);
    static native String failureDiagnosticJson(byte[] bytes);
}
