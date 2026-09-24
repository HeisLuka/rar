#!/usr/bin/env python3
import copy
from render_scene_v1 import hash_id

PRIMITIVE_KINDS = ("rects","images","glyph_runs")

def _atoms_for_node(scene, node_id):
    out={k:[] for k in PRIMITIVE_KINDS}
    for kind in PRIMITIVE_KINDS:
        for atom in scene["primitives"][kind]:
            if atom.get("node_id")==node_id:
                out[kind].append(copy.deepcopy(atom))
    return out

def diff_render_scenes(base, target):
    base_nodes={x["node_id"]:x for x in base["atom_map"]}
    target_nodes={x["node_id"]:x for x in target["atom_map"]}
    removed=sorted(set(base_nodes)-set(target_nodes))
    upserts=[]
    for node_id in sorted(target_nodes):
        base_atoms=_atoms_for_node(base,node_id) if node_id in base_nodes else None
        target_atoms=_atoms_for_node(target,node_id)
        if base_atoms!=target_atoms:
            upserts.append({"node_id":node_id,"primitives":target_atoms})

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
        "order_deltas": None if (base["order_authority"],base["paint_seq"])==(target["order_authority"],target["paint_seq"]) else {
            "order_authority":target["order_authority"],
            "paint_seq":copy.deepcopy(target["paint_seq"]),
        },
        "diagnostics": None if base["diagnostics"]==target["diagnostics"] else copy.deepcopy(target["diagnostics"]),
    }
    patch["patch_id"]=hash_id(patch)
    return patch

def apply_patch(base, patch):
    if base["render_scene_id"]!=patch["base_render_scene_id"]:
        raise ValueError("base render scene mismatch")
    out=copy.deepcopy(base)

    removed=set(patch["removed_nodes"])
    upsert_ids={u["node_id"] for u in patch["upsert_nodes"]}
    affected=removed|upsert_ids

    for kind in PRIMITIVE_KINDS:
        out["primitives"][kind]=[
            atom for atom in out["primitives"][kind]
            if atom.get("node_id") not in affected
        ]
        for upsert in patch["upsert_nodes"]:
            out["primitives"][kind].extend(copy.deepcopy(upsert["primitives"][kind]))

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
    if patch["order_deltas"] is not None:
        out["order_authority"]=patch["order_deltas"]["order_authority"]
        out["paint_seq"]=copy.deepcopy(patch["order_deltas"]["paint_seq"])
    if patch["diagnostics"] is not None:
        out["diagnostics"]=copy.deepcopy(patch["diagnostics"])

    # Primitive table order is canonical RenderScene state, not an implementation
    # detail. Reconstruct each table from the target paint sequence rather than
    # sorting by atom_id, which diverges on interleaved multi-page NodeIds.
    paint_rank={atom_id:index for index,atom_id in enumerate(out["paint_seq"])}
    for kind in PRIMITIVE_KINDS:
        out["primitives"][kind].sort(
            key=lambda atom:(paint_rank.get(atom["atom_id"], 2**63-1), atom["atom_id"])
        )

    out["scene_revision"]=patch["target_revision"]
    out.pop("render_scene_id",None)
    out["render_scene_id"]=hash_id(out)
    if out["render_scene_id"]!=patch["target_render_scene_id"]:
        raise AssertionError("patched RenderScene does not equal target identity")
    return out
