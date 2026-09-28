package com.chaptera.reader;

import org.json.JSONObject;

final class FailurePresentation {
    final String title;
    final String message;
    final String intakeClass;

    private FailurePresentation(String title, String message, String intakeClass) {
        this.title = title;
        this.message = message;
        this.intakeClass = intakeClass;
    }

    static FailurePresentation fromDiagnosticJson(String diagnosticJson) {
        String intakeClass = "unknown";
        try {
            JSONObject root = new JSONObject(diagnosticJson);
            JSONObject envelope = root.optJSONObject("envelope");
            if (envelope != null) {
                intakeClass = envelope.optString("intake_class", "unknown");
            }
        } catch (Exception ignored) {
            return generic();
        }

        switch (intakeClass) {
            case "not_pub":
                return new FailurePresentation(
                    "This is not a Publisher file",
                    "Chaptera read the local bytes, but they do not look like a Microsoft Publisher document. Choose another file.",
                    intakeClass
                );
            case "suspicious_polyglot":
                return new FailurePresentation(
                    "Conflicting file signatures",
                    "This file contains conflicting format signatures. Chaptera did not open it as a PUB. The source was not modified.",
                    intakeClass
                );
            case "pub_damaged":
                return new FailurePresentation(
                    "Publisher file appears damaged",
                    "This looks like a Publisher document, but its structure appears damaged. Chaptera will not modify or claim to repair the source.",
                    intakeClass
                );
            case "pub_possible":
                return new FailurePresentation(
                    "Publisher file may be unsupported",
                    "This may be a Publisher document, but Chaptera cannot identify a supported structure yet. You can inspect local diagnostics or choose another file.",
                    intakeClass
                );
            case "archive_with_pub":
                return new FailurePresentation(
                    "Publisher file found inside an archive",
                    "This archive appears to contain a Publisher candidate. Extract the .pub file and open that file directly.",
                    intakeClass
                );
            case "pub_high_value":
                return new FailurePresentation(
                    "Publisher file could not be opened",
                    "This strongly resembles a Publisher document, but the current Reader could not open it. The source was left unchanged.",
                    intakeClass
                );
            default:
                return generic();
        }
    }

    static FailurePresentation accessDenied() {
        return new FailurePresentation(
            "File access is no longer available",
            "Android no longer grants Chaptera access to this file. Retry if access was temporary, or choose the file again.",
            "access_denied"
        );
    }

    static FailurePresentation providerUnavailable(String detail) {
        String suffix = detail == null || detail.isEmpty() ? "" : " (" + detail + ")";
        return new FailurePresentation(
            "Could not read this local file",
            "The file provider is unavailable or the file moved. Retry, or choose another local file." + suffix,
            "provider_unavailable"
        );
    }

    static FailurePresentation generic() {
        return new FailurePresentation(
            "Could not open this file locally",
            "Chaptera could not open the file. You can retry, choose another file, or inspect local diagnostics.",
            "unknown"
        );
    }
}
