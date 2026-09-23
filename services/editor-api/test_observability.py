import unittest

from observability import (
    ObservabilityContractError,
    TRACE_PROTOCOL_VERSION,
    TraceRecorder,
    normalize_trace_context,
)


class ObservabilityTests(unittest.TestCase):
    def context(self):
        return {
            "protocol_version": TRACE_PROTOCOL_VERSION,
            "trace_id": "trace:12345678",
            "interaction_id": "interaction:12345678",
            "session_incarnation": "session:12345678",
            "operation_class": "commit",
            "browser_family": "chromium",
            "client_operation_id": "browser-op-12345678",
        }

    def test_trace_context_is_bounded_and_preserves_semantic_operation_id(self):
        value = normalize_trace_context(self.context())
        self.assertEqual(value["operation_class"], "commit")
        self.assertEqual(value["client_operation_id"], "browser-op-12345678")

    def test_trace_id_is_not_required_to_equal_client_operation_id(self):
        value = normalize_trace_context(self.context())
        self.assertNotEqual(value["trace_id"], value["client_operation_id"])

    def test_metric_labels_reject_high_cardinality_identity(self):
        with self.assertRaises(ObservabilityContractError):
            TraceRecorder.validate_metric_labels(
                {
                    "stage": "gateway.commit",
                    "operation_class": "commit",
                    "outcome": "success",
                    "region": "local",
                    "protocol_major": "v1",
                    "browser_family": "chromium",
                    "document_id": "doc:secret",
                }
            )

    def test_span_records_trace_and_bounded_metric_without_payload(self):
        recorder = TraceRecorder()
        with recorder.span("gateway.commit", self.context()):
            pass

        trace = recorder.trace_summary("trace:12345678")
        self.assertEqual(trace["span_count"], 1)
        span = trace["spans"][0]
        self.assertEqual(span["client_operation_id"], "browser-op-12345678")
        self.assertNotIn("document_id", span)
        self.assertNotIn("source_hash", span)

        metrics = recorder.metrics_snapshot()
        self.assertEqual(len(metrics), 1)
        labels = metrics[0]["labels"]
        self.assertEqual(labels["stage"], "gateway.commit")
        self.assertNotIn("trace_id", labels)
        self.assertNotIn("client_operation_id", labels)

    def test_recent_buffer_is_bounded(self):
        recorder = TraceRecorder(max_spans=2)
        for i in range(3):
            context = self.context()
            context["trace_id"] = f"trace:1234567{i}"
            with recorder.span("gateway.commit", context):
                pass
        self.assertEqual(sum(t["span_count"] for t in recorder.recent_traces()), 2)


if __name__ == "__main__":
    unittest.main()
