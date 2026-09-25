#!/usr/bin/env python3
from __future__ import annotations
import copy
import hashlib
import json

MAX_SAFE_EMU = 9_007_199_254_740_991
MIN_SAFE_EMU = -MAX_SAFE_EMU
PATH_SCHEMA = "chaptera.path-geometry.v1"

class PathGeometryError(ValueError):
    pass

def canonical_json(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))

def geometry_digest(value):
    return "sha256:" + hashlib.sha256(canonical_json(value).encode("utf-8")).hexdigest()

def _emu(value, label):
    if not isinstance(value, int) or isinstance(value, bool) or not (MIN_SAFE_EMU <= value <= MAX_SAFE_EMU):
        raise PathGeometryError(f"{label} must be a JavaScript-safe signed EMU integer")
    return value

def _command_points(command):
    op=command["op"]
    if op in {"MoveTo","LineTo"}:
        return [(command["x"],command["y"])]
    if op=="QuadTo":
        return [(command["cx"],command["cy"]),(command["x"],command["y"])]
    if op=="CubicTo":
        return [(command["c1x"],command["c1y"]),(command["c2x"],command["c2y"]),(command["x"],command["y"])]
    return []

def normalize_path_geometry(raw):
    if not isinstance(raw, dict):
        raise PathGeometryError("path geometry must be an object")
    commands=raw.get("commands")
    if not isinstance(commands, list) or not commands:
        raise PathGeometryError("path geometry requires non-empty commands")
    fill_rule=raw.get("fill_rule","nonzero")
    clip_rule=raw.get("clip_rule",fill_rule)
    if fill_rule not in {"nonzero","evenodd"} or clip_rule not in {"nonzero","evenodd"}:
        raise PathGeometryError("invalid fill/clip rule")
    out=[]
    subpath_open=False
    points=[]
    for i,cmd in enumerate(commands):
        if not isinstance(cmd, dict) or "op" not in cmd:
            raise PathGeometryError(f"commands[{i}] malformed")
        op=cmd["op"]
        if op=="MoveTo":
            norm={"op":op,"x":_emu(cmd.get("x"),f"commands[{i}].x"),"y":_emu(cmd.get("y"),f"commands[{i}].y")}
            subpath_open=True
        elif op=="LineTo":
            if not subpath_open: raise PathGeometryError("LineTo requires open subpath")
            norm={"op":op,"x":_emu(cmd.get("x"),f"commands[{i}].x"),"y":_emu(cmd.get("y"),f"commands[{i}].y")}
        elif op=="QuadTo":
            if not subpath_open: raise PathGeometryError("QuadTo requires open subpath")
            norm={"op":op,
                  "cx":_emu(cmd.get("cx"),f"commands[{i}].cx"),"cy":_emu(cmd.get("cy"),f"commands[{i}].cy"),
                  "x":_emu(cmd.get("x"),f"commands[{i}].x"),"y":_emu(cmd.get("y"),f"commands[{i}].y")}
        elif op=="CubicTo":
            if not subpath_open: raise PathGeometryError("CubicTo requires open subpath")
            norm={"op":op,
                  "c1x":_emu(cmd.get("c1x"),f"commands[{i}].c1x"),"c1y":_emu(cmd.get("c1y"),f"commands[{i}].c1y"),
                  "c2x":_emu(cmd.get("c2x"),f"commands[{i}].c2x"),"c2y":_emu(cmd.get("c2y"),f"commands[{i}].c2y"),
                  "x":_emu(cmd.get("x"),f"commands[{i}].x"),"y":_emu(cmd.get("y"),f"commands[{i}].y")}
        elif op=="Close":
            if not subpath_open: raise PathGeometryError("Close requires open subpath")
            norm={"op":"Close"}
            subpath_open=False
        else:
            raise PathGeometryError(f"unsupported path command: {op}")
        out.append(norm)
        points.extend(_command_points(norm))
    if not points:
        raise PathGeometryError("path has no coordinates")
    xs=[p[0] for p in points]; ys=[p[1] for p in points]
    bounds={"x":min(xs),"y":min(ys),"width":max(xs)-min(xs),"height":max(ys)-min(ys)}
    supplied=raw.get("bounds")
    if supplied is not None and supplied != bounds:
        raise PathGeometryError("supplied path bounds do not match derived bounds")
    payload={"schema":PATH_SCHEMA,"commands":out,"fill_rule":fill_rule,"clip_rule":clip_rule,"bounds":bounds}
    payload["path_digest"]=geometry_digest(payload)
    return payload

def build_path_table(raw_paths):
    if not isinstance(raw_paths,list):
        raise PathGeometryError("path_geometries must be a list")
    by_source={}
    by_digest={}
    for i,row in enumerate(raw_paths):
        if not isinstance(row,dict) or not isinstance(row.get("path_id"),str) or not row["path_id"]:
            raise PathGeometryError(f"path_geometries[{i}] requires path_id")
        if row["path_id"] in by_source:
            raise PathGeometryError("duplicate path_id")
        normalized=normalize_path_geometry(row)
        digest=normalized["path_digest"]
        by_digest.setdefault(digest, normalized)
        by_source[row["path_id"]]=digest
    table=[copy.deepcopy(by_digest[d]) for d in sorted(by_digest)]
    index={row["path_digest"]:i for i,row in enumerate(table)}
    lookup={pid:{"path_digest":digest,"path_index":index[digest]} for pid,digest in by_source.items()}
    return table,lookup
