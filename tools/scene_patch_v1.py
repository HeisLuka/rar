#!/usr/bin/env python3
import copy
from render_scene_v1 import hash_id

PRIMITIVE_KINDS = ("rects","images","paths","glyph_runs")

class ScenePatchApplyPoisoned(AssertionError):
    """The renderer-owned compiled state must be discarded after this failure."""

def _empty_atom_buckets():
    return {kind:[] for kind in PRIMITIVE_KINDS}

def _diff_table_by_id(base_rows, target_rows, id_key):
    base = {row[id_key]: row for row in base_rows}
    target = {row[id_key]: row for row in target_rows}
    removed = sorted(set(base) - set(target))
    upserts = [
        copy.deepcopy(target[row_id])
        for row_id in sorted(target)
        if base.get(row_id) != target[row_id]
    ]
    if not removed and not upserts:
        return None
    return {"removed": removed, "upserts": upserts}

def _apply_table_delta(rows, delta, id_key):
    if delta is None:
        return rows
    removed = set(delta["removed"])
    upserts = {row[id_key]: copy.deepcopy(row) for row in delta["upserts"]}
    out = [row for row in rows if row[id_key] not in removed and row[id_key] not in upserts]
    out.extend(upserts.values())
    out.sort(key=lambda row: row[id_key])
    return out

def build_node_atom_index(scene, metrics=None):
    """Build one deterministic NodeId -> primitive ownership index.

    Atom lists preserve the compiler's existing primitive-table order. The index
    holds references only; changed target atoms are deep-copied when emitted into
    a patch.
    """
    index={row["node_id"]:_empty_atom_buckets() for row in scene["atom_map"]}
    primitive_visits=0
    owned_atoms=0
    for kind in PRIMITIVE_KINDS:
        for atom in scene["primitives"][kind]:
            primitive_visits+=1
            node_id=atom.get("node_id")
            if node_id is None:
                raise ValueError(f"{kind} atom missing node_id")
            bucket=index.get(node_id)
            if bucket is None:
                raise ValueError(f"{kind} atom references node absent from atom_map: {node_id}")
            bucket[kind].append(atom)
            owned_atoms+=1

    if metrics is not None:
        metrics.update({
            "node_buckets":len(index),
            "primitive_visits":primitive_visits,
            "owned_atoms":owned_atoms,
        })
    return index

def diff_render_scenes(base, target, metrics=None):
    base_nodes={x["node_id"]:x for x in base["atom_map"]}
    target_nodes={x["node_id"]:x for x in target["atom_map"]}

    base_index_metrics={}
    target_index_metrics={}
    base_atoms_by_node=build_node_atom_index(base,base_index_metrics)
    target_atoms_by_node=build_node_atom_index(target,target_index_metrics)

    removed=sorted(set(base_nodes)-set(target_nodes))
    upserts=[]
    node_comparisons=0
    for node_id in sorted(target_nodes):
        node_comparisons+=1
        base_atoms=base_atoms_by_node.get(node_id)
        target_atoms=target_atoms_by_node[node_id]
        if base_atoms!=target_atoms:
            upserts.append({
                "node_id":node_id,
                "primitives":copy.deepcopy(target_atoms),
            })

    patch={
        "patch_version":"chaptera.scene-patch.v1",
        "base_revision":base["scene_revision"],
        "target_revision":target["scene_revision"],
        "base_render_scene_id":base["render_scene_id"],
        "target_render_scene_id":target["render_scene_id"],
        "removed_nodes":removed,
        "upsert_nodes":upserts,
        "page_deltas": [] if base["pages"]==target["pages"] else copy.deepcopy(target["pages"]),
        "resource_deltas": [] if base["tables"]["resources"]==target["tables"]["resources"] else copy.deepcopy(target["tables"]["resources"]),
        "path_deltas": None if base["tables"].get("paths",[])==target["tables"].get("paths",[]) else copy.deepcopy(target["tables"].get("paths",[])),
        "effect_deltas": {
            "effects": _diff_table_by_id(
                base["tables"].get("effects", []),
                target["tables"].get("effects", []),
                "effect_id",
            ),
            "effect_groups": _diff_table_by_id(
                base["tables"].get("effect_groups", []),
                target["tables"].get("effect_groups", []),
                "effect_group_id",
            ),
        },
        "order_deltas": None if (base["order_authority"],base["paint_seq"])==(target["order_authority"],target["paint_seq"]) else {
            "order_authority":target["order_authority"],
            "paint_seq":copy.deepcopy(target["paint_seq"]),
        },
        "diagnostics": None if base["diagnostics"]==target["diagnostics"] else copy.deepcopy(target["diagnostics"]),
    }
    patch["patch_id"]=hash_id(patch)

    if metrics is not None:
        base_primitive_count=base_index_metrics["primitive_visits"]
        target_primitive_count=target_index_metrics["primitive_visits"]
        existing_target_nodes=len(set(base_nodes)&set(target_nodes))
        legacy_repeated_scan_visits=(
            len(target_nodes)*target_primitive_count
            + existing_target_nodes*base_primitive_count
        )
        metrics.update({
            "index_strategy":"single_pass_node_ownership",
            "base_node_count":len(base_nodes),
            "target_node_count":len(target_nodes),
            "node_comparisons":node_comparisons,
            "base_primitive_visits":base_primitive_count,
            "target_primitive_visits":target_primitive_count,
            "primitive_visits_total":base_primitive_count+target_primitive_count,
            "legacy_repeated_scan_primitive_visits":legacy_repeated_scan_visits,
            "changed_node_count":len(upserts),
            "removed_node_count":len(removed),
        })

    return patch

def _require_base_identity(scene, patch):
    if scene["render_scene_id"]!=patch["base_render_scene_id"]:
        raise ValueError("base render scene mismatch")

def _apply_patch_mutating(out, patch, metrics=None):
    """Mutate a renderer-owned disposable RenderScene after base validation.

    The caller owns failure recovery. If final target identity validation fails,
    this object is poisoned and must be discarded/recompiled.
    """
    removed=set(patch["removed_nodes"])
    upsert_ids={u["node_id"] for u in patch["upsert_nodes"]}
    affected=removed|upsert_ids

    primitive_filter_visits=0
    upsert_atom_copies=0
    for kind in PRIMITIVE_KINDS:
        previous=out["primitives"][kind]
        primitive_filter_visits+=len(previous)
        out["primitives"][kind]=[
            atom for atom in previous
            if atom.get("node_id") not in affected
        ]
        for upsert in patch["upsert_nodes"]:
            atoms=upsert["primitives"][kind]
            upsert_atom_copies+=len(atoms)
            out["primitives"][kind].extend(copy.deepcopy(atoms))

    atom_map_filter_visits=len(out["atom_map"])
    out["atom_map"]=[
        x for x in out["atom_map"] if x["node_id"] not in affected
    ]
    for upsert in patch["upsert_nodes"]:
        atoms=[]
        for kind in PRIMITIVE_KINDS:
            atoms.extend(a["atom_id"] for a in upsert["primitives"][kind])
        out["atom_map"].append({"node_id":upsert["node_id"],"atoms":atoms})
    out["atom_map"].sort(key=lambda x:x["node_id"])

    if patch["page_deltas"]:
        out["pages"]=copy.deepcopy(patch["page_deltas"])
    if patch["resource_deltas"]:
        out["tables"]["resources"]=copy.deepcopy(patch["resource_deltas"])
    if patch.get("path_deltas") is not None:
        out["tables"]["paths"]=copy.deepcopy(patch["path_deltas"])
    effect_deltas = patch.get("effect_deltas")
    if effect_deltas:
        out["tables"]["effects"] = _apply_table_delta(
            out["tables"].get("effects", []),
            effect_deltas.get("effects"),
            "effect_id",
        )
        out["tables"]["effect_groups"] = _apply_table_delta(
            out["tables"].get("effect_groups", []),
            effect_deltas.get("effect_groups"),
            "effect_group_id",
        )
    if patch["order_deltas"] is not None:
        out["order_authority"]=patch["order_deltas"]["order_authority"]
        out["paint_seq"]=copy.deepcopy(patch["order_deltas"]["paint_seq"])
    if patch["diagnostics"] is not None:
        out["diagnostics"]=copy.deepcopy(patch["diagnostics"])

    # Primitive table order is canonical RenderScene state, not an implementation
    # detail. Reconstruct each table from the target paint sequence rather than
    # sorting by atom_id, which diverges on interleaved multi-page NodeIds.
    paint_rank={atom_id:index for index,atom_id in enumerate(out["paint_seq"])}
    reorder_atoms=0
    for kind in PRIMITIVE_KINDS:
        reorder_atoms+=len(out["primitives"][kind])
        out["primitives"][kind].sort(
            key=lambda atom:(paint_rank.get(atom["atom_id"], 2**63-1), atom["atom_id"])
        )

    out["scene_revision"]=patch["target_revision"]
    out.pop("render_scene_id",None)
    out["render_scene_id"]=hash_id(out)

    if metrics is not None:
        metrics.update({
            "primitive_filter_visits":primitive_filter_visits,
            "atom_map_filter_visits":atom_map_filter_visits,
            "upsert_atom_copies":upsert_atom_copies,
            "reorder_atoms":reorder_atoms,
            "affected_node_count":len(affected),
        })

    if out["render_scene_id"]!=patch["target_render_scene_id"]:
        raise ScenePatchApplyPoisoned(
            "patched RenderScene does not equal target identity; discard renderer state"
        )
    return out

def apply_patch(base, patch, metrics=None):
    """Pure compatibility/oracle apply.

    The input base is never mutated. This intentionally pays for a full deep copy.
    """
    _require_base_identity(base,patch)
    out=copy.deepcopy(base)
    if metrics is not None:
        metrics["full_scene_deepcopy"]=True
    return _apply_patch_mutating(out,patch,metrics=metrics)

def apply_patch_in_place(scene, patch, metrics=None):
    """Hot renderer path for an exclusively owned disposable RenderScene.

    Wrong-base identity is rejected before mutation. Any later
    ScenePatchApplyPoisoned error means the scene may have been mutated and must
    be discarded. Never pass authoring/layout authority state to this function.
    """
    _require_base_identity(scene,patch)
    if metrics is not None:
        metrics["full_scene_deepcopy"]=False
    return _apply_patch_mutating(scene,patch,metrics=metrics)
