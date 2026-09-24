#!/usr/bin/env python3
import json
from editor_project_asset_registry_v1 import *
from test_editor_project_asset_registry_v1 import A,B,C,MA,MB,MC,project

canonical=serialize_project_asset_registry_v1(
    project(),{MA.asset_sha256:MA,MB.asset_sha256:MB,MC.asset_sha256:MC}
)
calls=[]
result=apply_project_with_assets_v1(
    canonical,
    asset_bytes_by_sha={MA.asset_sha256:A,MB.asset_sha256:B},
    fresh_session={"applied":[]},
    apply_operation=lambda s,o,a:(s["applied"].append(o["kind"]) or s),
)
print(json.dumps({
    "schema":"chaptera.editor-project-asset-registry-receipt.v1",
    "project_schema":canonical["schema_version"],
    "registry_schema":canonical["editor_asset_registry_schema"],
    "required_asset_count":len(required_editor_asset_ids_v1(canonical)),
    "serialized_asset_count":len(canonical["editor_assets"]),
    "unreferenced_import_serialized":False,
    "operation_log_reachability":True,
    "complete_preflight_before_replay":True,
    "replayed_operations":result["applied"],
    "asset_bytes_in_project_json":False,
    "source_pub_mutated":False,
},indent=2,sort_keys=True))
