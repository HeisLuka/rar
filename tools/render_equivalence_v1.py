from __future__ import annotations

import copy
from typing import Any

from optimization_receipt_v1 import compare_measurements
from visreg_v1 import compare_scenes

SCHEMA = "chaptera.render-equivalence.receipt.v1"


class RenderEquivalenceIncomparable(ValueError):
    pass


def _environment_key(env: dict[str, Any]) -> dict[str, Any]:
    required = (
        "viewport_width_px",
        "viewport_height_px",
        "dpr",
        "output_width_px",
        "output_height_px",
        "color_profile",
        "resource_state",
    )
    missing = [key for key in required if key not in env]
    if missing:
        raise RenderEquivalenceIncomparable("missing_environment:" + ",".join(missing))
    return {key: copy.deepcopy(env[key]) for key in required}


def _visreg(
    baseline_scene: dict[str, Any],
    candidate_scene: dict[str, Any],
    baseline_render: dict[str, Any] | None,
    candidate_render: dict[str, Any] | None,
) -> dict[str, Any]:
    try:
        return compare_scenes(
            baseline_scene,
            candidate_scene,
            baseline_render=baseline_render,
            candidate_render=candidate_render,
        )
    except AssertionError as exc:
        raise RenderEquivalenceIncomparable(str(exc)) from exc


def compare_render_paths(
    *,
    baseline_scene: dict[str, Any],
    candidate_scene: dict[str, Any],
    baseline_render: dict[str, Any] | None,
    candidate_render: dict[str, Any] | None,
    baseline_environment: dict[str, Any],
    candidate_environment: dict[str, Any],
    reference_identity: dict[str, Any],
    candidate_identity: dict[str, Any],
    mode: str = "settled",
    admitted_transient_render_codes: set[str] | None = None,
    settled_baseline_render: dict[str, Any] | None = None,
    settled_candidate_render: dict[str, Any] | None = None,
) -> dict[str, Any]:
    left_env = _environment_key(baseline_environment)
    right_env = _environment_key(candidate_environment)
    if left_env != right_env:
        raise RenderEquivalenceIncomparable("render_environment_mismatch")
    if mode not in {"settled", "transient"}:
        raise ValueError("mode must be settled or transient")

    report = _visreg(baseline_scene, candidate_scene, baseline_render, candidate_render)
    diffs = report["differences"]
    non_render = [item for item in diffs if item["stage"] != "render"]

    if mode == "settled":
        correctness_equivalent = not non_render
        fidelity_equivalent = report["summary"]["equivalent"] is True
        convergence = {"required": False, "equivalent": fidelity_equivalent}
        disallowed = [item["code"] for item in diffs]
    else:
        allowed = set(admitted_transient_render_codes or set())
        disallowed = [item["code"] for item in diffs if item["stage"] != "render" or item["code"] not in allowed]
        if settled_baseline_render is None or settled_candidate_render is None:
            raise RenderEquivalenceIncomparable("transient_mode_requires_settled_convergence_evidence")
        settled = _visreg(
            baseline_scene,
            candidate_scene,
            settled_baseline_render,
            settled_candidate_render,
        )
        converged = settled["summary"]["equivalent"] is True
        correctness_equivalent = not non_render
        fidelity_equivalent = not disallowed and converged
        convergence = {
            "required": True,
            "equivalent": converged,
            "settled_report_hash": settled["report_hash"],
        }

    return {
        "schema": SCHEMA,
        "mode": mode,
        "reference_identity": copy.deepcopy(reference_identity),
        "candidate_identity": copy.deepcopy(candidate_identity),
        "environment": left_env,
        "visreg_report_hash": report["report_hash"],
        "visreg_summary": copy.deepcopy(report["summary"]),
        "correctness_equivalent": correctness_equivalent,
        "fidelity_equivalent": fidelity_equivalent,
        "passed": correctness_equivalent and fidelity_equivalent,
        "disallowed_difference_codes": sorted(set(disallowed)),
        "convergence": convergence,
        "evidence_authority": {
            "real_pub": False,
            "representative": False,
            "technology_decision_allowed": False,
        },
    }


def compare_optimization_with_equivalence(
    *,
    equivalence_receipt: dict[str, Any],
    optimization_id: str,
    hot_path: str,
    baseline_measurement: dict[str, Any],
    candidate_measurement: dict[str, Any],
    budgets: dict[str, dict[str, float]] | None = None,
    decision_scope: str = "local_optimization",
) -> dict[str, Any]:
    if equivalence_receipt.get("schema") != SCHEMA:
        raise ValueError("unexpected equivalence receipt schema")
    return compare_measurements(
        optimization_id=optimization_id,
        hot_path=hot_path,
        baseline=baseline_measurement,
        candidate=candidate_measurement,
        budgets=budgets,
        correctness_equivalent=equivalence_receipt["correctness_equivalent"],
        fidelity_equivalent=equivalence_receipt["fidelity_equivalent"],
        decision_scope=decision_scope,
    )
