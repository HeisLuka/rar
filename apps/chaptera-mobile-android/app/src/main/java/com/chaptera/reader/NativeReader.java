package com.chaptera.reader;

final class NativeReader {
    static {
        System.loadLibrary("chaptera_mobile_reader_jni");
    }

    private NativeReader() {}

    static native String openLocalPubJson(byte[] bytes);
    static native String failureDiagnosticJson(byte[] bytes);
}
