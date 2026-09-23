"""Privacy-bounded observability primitives for Chaptera Cloud Editor V1.

Observability follows semantic identities for correlation but never becomes
semantic authority. High-cardinality identifiers are allowed only in trace
records. Aggregated metric labels are restricted to a small bounded vocabulary.
"""

from __future__ import annotations

import copy
import re
import time
from contextlib import contextmanager
from typing import Any, Dict, Iterator, Optional

TRACE_PROTOCOL_VERSION = "chaptera.trace-context.v1"
_ID_RE = re.compile(r"^[A-Za-z0-9._:-]{8,160}$")
_STAGE_RE = re.compile(r"^[a-z][a-z0-9_.-]{1,63}$")

ALLOWED_METRIC_LABELS = frozenset(
    {
        "stage",
        "operation_class",
        "outcome",
        "region",
        "protocol_major",
        "browser_family",
    }
)
FORBIDDEN_METRIC_LABELS = frozenset(
    {
        "document_id",
        "principal_id",
        "user_id",
        "story_id",
        "revision_id",
        "event_id",
        "client_operation_id",
        "interaction_id",
        "trace_id",
        "source_hash",
        "content_hash",
        "file_name",
        "filename",
    }
)
ALLOWED_OUTCOMES = frozenset({"success", "error", "rejected", "unknown"})
ALLOWED_OPERATION_CLASSES = frozenset(
    {"commit", "scene_read", "open", "reconnect", "export", "asset", "other"}
)
ALLOWED_BROWSER_FAMILIES = frozenset(
    {"chromium", "firefox", "webkit", "other", "unknown"}
)


class ObservabilityContractError(ValueError):
    pass


def _identifier(label: str, value: Any) -> str:
    if not isinstance(value, str) or not _ID_RE.fullmatch(value):
        raise ObservabilityContractError(f"{label} must be a bounded opaque identifier")
    return value


def normalize_trace_context(value: dict) -> dict:
    if not isinstance(value, dict):
        raise ObservabilityContractError("trace context must be an object")
    if value.get("protocol_version") != TRACE_PROTOCOL_VERSION:
        raise ObservabilityContractError("unsupported trace context version")
    operation_class = value.get("operation_class")
    if operation_class not in ALLOWED_OPERATION_CLASSES:
        raise ObservabilityContractError("unsupported operation_class")
    browser_family = value.get("browser_family", "unknown")
    if browser_family not in ALLOWED_BROWSER_FAMILIES:
        raise ObservabilityContractError("unsupported browser_family")

    normalized = {
        "protocol_version": TRACE_PROTOCOL_VERSION,
        "trace_id": _identifier("trace_id", value.get("trace_id")),
        "interaction_id": _identifier("interaction_id", value.get("interaction_id")),
        "session_incarnation": _identifier(
            "session_incarnation", value.get("session_incarnation")
        ),
        "operation_class": operation_class,
        "browser_family": browser_family,
    }
    client_operation_id = value.get("client_operation_id")
    if client_operation_id is not None:
        normalized["client_operation_id"] = _identifier(
            "client_operation_id", client_operation_id
        )
    return normalized


def trace_context_from_headers(headers) -> Optional[dict]:
    trace_id = headers.get("x-chaptera-trace-id")
    if trace_id is None:
        return None
    value = {
        "protocol_version": headers.get(
            "x-chaptera-trace-version", TRACE_PROTOCOL_VERSION
        ),
        "trace_id": trace_id,
        "interaction_id": headers.get("x-chaptera-interaction-id"),
        "session_incarnation": headers.get("x-chaptera-session-incarnation"),
        "operation_class": headers.get("x-chaptera-operation-class", "other"),
        "browser_family": headers.get("x-chaptera-browser-family", "unknown"),
    }
    return normalize_trace_context(value)


def _validate_metric_labels(labels: dict) -> dict:
    if not isinstance(labels, dict):
        raise ObservabilityContractError("metric labels must be an object")
    unknown = set(labels) - ALLOWED_METRIC_LABELS
    if unknown:
        raise ObservabilityContractError(
            "unbounded metric label(s): " + ", ".join(sorted(unknown))
        )
    forbidden = set(labels) & FORBIDDEN_METRIC_LABELS
    if forbidden:
        raise ObservabilityContractError(
            "high-cardinality metric label(s): " + ", ".join(sorted(forbidden))
        )
    normalized = {}
    for key, value in labels.items():
        if not isinstance(value, str) or len(value) > 64:
            raise ObservabilityContractError(f"metric label {key} must be bounded text")
        normalized[key] = value
    if normalized.get("outcome") not in ALLOWED_OUTCOMES:
        raise ObservabilityContractError("metric outcome is not bounded")
    return normalized


class TraceRecorder:
    def __init__(self, *, max_spans: int = 512) -> None:
        if max_spans <= 0:
            raise ValueError("max_spans must be positive")
        self.max_spans = max_spans
        self._spans: list[dict] = []
        self._metrics: Dict[tuple, dict] = {}

    @contextmanager
    def span(self, name: str, context: Optional[dict]) -> Iterator[None]:
        if context is None:
            yield
            return
        if not isinstance(name, str) or not _STAGE_RE.fullmatch(name):
            raise ObservabilityContractError("span name must be bounded")
        normalized = normalize_trace_context(context)
        started = time.perf_counter_ns()
        outcome = "success"
        try:
            yield
        except Exception:
            outcome = "error"
            raise
        finally:
            duration_ms = max(0.0, (time.perf_counter_ns() - started) / 1_000_000.0)
            self.record_span(
                name=name,
                context=normalized,
                outcome=outcome,
                duration_ms=duration_ms,
            )

    def record_span(
        self,
        *,
        name: str,
        context: dict,
        outcome: str,
        duration_ms: float,
    ) -> None:
        if not isinstance(name, str) or not _STAGE_RE.fullmatch(name):
            raise ObservabilityContractError("span name must be bounded")
        normalized = normalize_trace_context(context)
        if outcome not in ALLOWED_OUTCOMES:
            raise ObservabilityContractError("outcome is not bounded")
        if not isinstance(duration_ms, (int, float)) or duration_ms < 0:
            raise ObservabilityContractError("duration_ms must be non-negative")

        span = {
            "protocol_version": TRACE_PROTOCOL_VERSION,
            "name": name,
            "trace_id": normalized["trace_id"],
            "interaction_id": normalized["interaction_id"],
            "session_incarnation": normalized["session_incarnation"],
            "operation_class": normalized["operation_class"],
            "browser_family": normalized["browser_family"],
            "outcome": outcome,
            "duration_ms": round(float(duration_ms), 6),
        }
        if "client_operation_id" in normalized:
            span["client_operation_id"] = normalized["client_operation_id"]
        self._spans.append(span)
        if len(self._spans) > self.max_spans:
            del self._spans[: len(self._spans) - self.max_spans]

        labels = _validate_metric_labels(
            {
                "stage": name,
                "operation_class": normalized["operation_class"],
                "outcome": outcome,
                "region": "local",
                "protocol_major": "v1",
                "browser_family": normalized["browser_family"],
            }
        )
        key = tuple(sorted(labels.items()))
        metric = self._metrics.setdefault(
            key,
            {
                "name": "cloud_editor_stage_latency_ms",
                "labels": copy.deepcopy(labels),
                "count": 0,
                "sum_ms": 0.0,
                "max_ms": 0.0,
            },
        )
        metric["count"] += 1
        metric["sum_ms"] = round(metric["sum_ms"] + float(duration_ms), 6)
        metric["max_ms"] = round(max(metric["max_ms"], float(duration_ms)), 6)

    def trace_summary(self, trace_id: str) -> dict:
        trace_id = _identifier("trace_id", trace_id)
        spans = [copy.deepcopy(item) for item in self._spans if item["trace_id"] == trace_id]
        return {
            "protocol_version": TRACE_PROTOCOL_VERSION,
            "trace_id": trace_id,
            "span_count": len(spans),
            "spans": spans,
        }

    def recent_traces(self) -> list[dict]:
        trace_ids: list[str] = []
        for span in self._spans:
            if span["trace_id"] not in trace_ids:
                trace_ids.append(span["trace_id"])
        return [self.trace_summary(trace_id) for trace_id in trace_ids]

    def metrics_snapshot(self) -> list[dict]:
        return [copy.deepcopy(self._metrics[key]) for key in sorted(self._metrics)]

    @staticmethod
    def validate_metric_labels(labels: dict) -> dict:
        return _validate_metric_labels(labels)
