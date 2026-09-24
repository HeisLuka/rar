#!/usr/bin/env python3
import collections
import datetime as dt
import json
import os
import re
import sys
import urllib.error
import urllib.request

API = "https://api.github.com"
RECEIPT_VERSION = "chaptera.ci-efficiency.v1"

def parse_ts(value):
    if not value:
        return None
    return dt.datetime.fromisoformat(value.replace("Z", "+00:00"))

def seconds_between(start, end):
    a=parse_ts(start)
    b=parse_ts(end)
    if not a or not b:
        return None
    return max(0.0,(b-a).total_seconds())

def normalize_step_name(name):
    value=(name or "").strip().lower()
    value=re.sub(r"\\s+"," ",value)
    value=re.sub(r"\\b(v|version)\\s*\\d+(?:\\.\\d+)*\\b","<version>",value)
    value=re.sub(r"\\d{6,}","<id>",value)
    return value

def categorize_step(name):
    n=(name or "").lower()
    if "checkout" in n:
        return "checkout"
    if "setup" in n or "set up" in n:
        return "runtime_setup"
    if "cache" in n:
        return "cache"
    if any(x in n for x in ("pip install","npm install","npm ci","cargo fetch","cargo install","apt-get","install pinned","install dependencies","install dependency")):
        return "dependency_install"
    if any(x in n for x in ("upload-artifact","upload artifact","download-artifact","download artifact","artifact")):
        return "artifact_io"
    if any(x in n for x in ("benchmark","bench","pytest","unittest","cargo test"," test ","tests","modelcheck","model check","contract","validate","validation","lint","clippy","fmt","check")):
        return "test_or_benchmark"
    if any(x in n for x in ("build","compile","cargo build")):
        return "build"
    return "other"

def gh_get(url, token):
    req=urllib.request.Request(
        url,
        headers={
            "Accept":"application/vnd.github+json",
            "Authorization":f"Bearer {token}",
            "X-GitHub-Api-Version":"2022-11-28",
            "User-Agent":"chaptera-ci-efficiency-v1",
        },
    )
    try:
        with urllib.request.urlopen(req,timeout=30) as resp:
            return json.load(resp)
    except urllib.error.HTTPError as exc:
        body=exc.read().decode("utf-8","replace")
        raise RuntimeError(f"GitHub API {exc.code} for {url}: {body[:500]}") from exc

def collect(repo, token, limit=30):
    limit=max(1,min(int(limit),50))
    runs_payload=gh_get(f"{API}/repos/{repo}/actions/runs?status=completed&per_page={limit}",token)
    raw_runs=runs_payload.get("workflow_runs",[])[:limit]

    runs=[]
    step_aggregate=collections.defaultdict(lambda:{
        "count":0,"seconds":0.0,"workflows":set(),"category":None,
    })
    category_seconds=collections.Counter()
    category_steps=collections.Counter()

    for run in raw_runs:
        run_id=run["id"]
        jobs_payload=gh_get(f"{API}/repos/{repo}/actions/runs/{run_id}/jobs?per_page=100",token)
        artifacts_payload=gh_get(f"{API}/repos/{repo}/actions/runs/{run_id}/artifacts?per_page=100",token)
        jobs_out=[]
        job_seconds=[]
        job_starts=[]
        job_ends=[]

        for job in jobs_payload.get("jobs",[]):
            duration=seconds_between(job.get("started_at"),job.get("completed_at"))
            if duration is not None:
                job_seconds.append(duration)
            if job.get("started_at"):
                job_starts.append(parse_ts(job["started_at"]))
            if job.get("completed_at"):
                job_ends.append(parse_ts(job["completed_at"]))

            steps=[]
            for step in job.get("steps",[]):
                sdur=seconds_between(step.get("started_at"),step.get("completed_at"))
                cat=categorize_step(step.get("name"))
                norm=normalize_step_name(step.get("name"))
                if sdur is not None:
                    category_seconds[cat]+=sdur
                category_steps[cat]+=1
                if norm:
                    agg=step_aggregate[norm]
                    agg["count"]+=1
                    agg["seconds"]+=sdur or 0.0
                    agg["workflows"].add(run.get("name") or str(run.get("workflow_id")))
                    agg["category"]=cat
                steps.append({
                    "number":step.get("number"),
                    "name":step.get("name"),
                    "normalized_name":norm,
                    "category":cat,
                    "status":step.get("status"),
                    "conclusion":step.get("conclusion"),
                    "duration_seconds":sdur,
                })
            jobs_out.append({
                "id":job.get("id"),
                "name":job.get("name"),
                "status":job.get("status"),
                "conclusion":job.get("conclusion"),
                "runner_name":job.get("runner_name"),
                "runner_group_name":job.get("runner_group_name"),
                "labels":job.get("labels",[]),
                "started_at":job.get("started_at"),
                "completed_at":job.get("completed_at"),
                "duration_seconds":duration,
                "steps":steps,
            })

        execution_span=None
        if job_starts and job_ends:
            execution_span=max(0.0,(max(job_ends)-min(job_starts)).total_seconds())

        artifacts=artifacts_payload.get("artifacts",[])
        artifact_bytes=sum(int(a.get("size_in_bytes") or 0) for a in artifacts)

        runs.append({
            "run_id":run_id,
            "workflow_id":run.get("workflow_id"),
            "workflow_name":run.get("name"),
            "event":run.get("event"),
            "status":run.get("status"),
            "conclusion":run.get("conclusion"),
            "head_sha":run.get("head_sha"),
            "head_branch":run.get("head_branch"),
            "run_number":run.get("run_number"),
            "run_attempt":run.get("run_attempt"),
            "created_at":run.get("created_at"),
            "run_started_at":run.get("run_started_at"),
            "updated_at":run.get("updated_at"),
            "queue_delay_seconds":seconds_between(run.get("created_at"),run.get("run_started_at")),
            "run_elapsed_seconds":seconds_between(run.get("created_at"),run.get("updated_at")),
            "execution_span_seconds":execution_span,
            "summed_job_seconds":sum(job_seconds),
            "summed_job_minutes":sum(job_seconds)/60.0,
            "approx_critical_path_job_seconds":max(job_seconds) if job_seconds else None,
            "job_count":len(jobs_out),
            "jobs_api_total_count":jobs_payload.get("total_count"),
            "jobs_page_truncated":(jobs_payload.get("total_count") or 0)>len(jobs_out),
            "artifact_count":artifacts_payload.get("total_count",len(artifacts)),
            "artifact_bytes_first_page":artifact_bytes,
            "artifact_page_truncated":(artifacts_payload.get("total_count") or 0)>len(artifacts),
            "jobs":jobs_out,
        })

    groups=collections.defaultdict(list)
    for r in runs:
        groups[(r["workflow_id"],r["head_sha"])].append(r)
    rerun_groups=[]
    for (workflow_id,head_sha),items in groups.items():
        max_attempt=max(int(x.get("run_attempt") or 1) for x in items)
        if len(items)>1 or max_attempt>1:
            rerun_groups.append({
                "workflow_id":workflow_id,
                "head_sha":head_sha,
                "run_ids":[x["run_id"] for x in items],
                "run_count":len(items),
                "max_run_attempt":max_attempt,
            })

    repeated=[]
    for name,agg in step_aggregate.items():
        if agg["count"]<2:
            continue
        repeated.append({
            "normalized_step_name":name,
            "category":agg["category"],
            "count":agg["count"],
            "total_seconds":agg["seconds"],
            "workflow_count":len(agg["workflows"]),
            "workflows":sorted(agg["workflows"]),
            "removability":"unknown_requires_semantic_review",
        })
    repeated.sort(key=lambda x:(-x["total_seconds"],-x["count"],x["normalized_step_name"]))

    total_job_seconds=sum(r["summed_job_seconds"] for r in runs)
    total_elapsed=sum(r["run_elapsed_seconds"] or 0.0 for r in runs)
    total_artifact_bytes=sum(r["artifact_bytes_first_page"] for r in runs)

    return {
        "receipt_version":RECEIPT_VERSION,
        "measurement_class":"real_public_github_actions_metadata",
        "repository":repo,
        "bounded_run_count":len(runs),
        "selection":"most_recent_completed_runs_first_page",
        "summary":{
            "summed_job_seconds":total_job_seconds,
            "summed_job_minutes":total_job_seconds/60.0,
            "summed_run_elapsed_seconds":total_elapsed,
            "artifact_bytes_first_pages":total_artifact_bytes,
            "rerun_group_count":len(rerun_groups),
            "step_category_seconds":dict(category_seconds),
            "step_category_counts":dict(category_steps),
        },
        "explicit_unknowns":{
            "billing_dollars":"unknown_without_account_billing_model",
            "github_billable_rounding":"unknown_not_in_run_job_metadata",
            "cache_hit_state":"unknown_unless_exposed_by_individual_step_outputs_logs",
            "network_transfer_cost":"unknown_without_billing_model",
            "true_dependency_critical_path":"unknown_from_flat_jobs_metadata; max_job_duration_is_only_an_approximation",
        },
        "candidate_repeated_work":repeated[:50],
        "rerun_groups":rerun_groups,
        "runs":runs,
        "guardrails":[
            "Repeated setup is a candidate cost, not proof that workflows can be merged or a guard removed.",
            "Independent public-boundary/provenance/correctness checks remain independent until a separate semantic equivalence review proves otherwise.",
            "Runner minutes are an engineering work unit here, not a dollar cost claim.",
        ],
    }

def main():
    repo=os.environ.get("GITHUB_REPOSITORY","HeisLuka/rar")
    token=os.environ.get("GITHUB_TOKEN")
    if not token:
        raise SystemExit("GITHUB_TOKEN is required")
    limit=int(os.environ.get("CI_EFFICIENCY_RUN_LIMIT","30"))
    receipt=collect(repo,token,limit)
    out=os.environ.get("CI_EFFICIENCY_OUTPUT")
    text=json.dumps(receipt,indent=2,sort_keys=True)+"\n"
    if out:
        os.makedirs(os.path.dirname(out),exist_ok=True)
        with open(out,"w",encoding="utf-8") as f:
            f.write(text)
    print(json.dumps({
        "receipt_version":receipt["receipt_version"],
        "repository":receipt["repository"],
        "bounded_run_count":receipt["bounded_run_count"],
        "summed_job_minutes":round(receipt["summary"]["summed_job_minutes"],3),
        "artifact_bytes_first_pages":receipt["summary"]["artifact_bytes_first_pages"],
        "rerun_group_count":receipt["summary"]["rerun_group_count"],
        "top_repeated_steps":receipt["candidate_repeated_work"][:10],
        "explicit_unknowns":receipt["explicit_unknowns"],
    },indent=2,sort_keys=True))

if __name__=="__main__":
    main()
