import test from "node:test";
import assert from "node:assert/strict";
import { WebExportUxV1, normalizeExportJobV1 } from "./export-ux-v1.mjs";

function setup(){
  const createIds=[];
  const jobs=new Map();
  const ctx={document_id:"doc:1",revision_id:"rev:exact-7",layout_environment_id:"env:a",allowed_targets:["idml:bounded-editable","odg:bounded-editable"]};
  const service={
    async previewExport(input){ return {...input,allowed:true,loss_summary:{level:"bounded"},capability_summary:{editable:true}}; },
    async createExport(input){
      createIds.push(input.client_request_id);
      const job={job_id:"job:1",document_id:input.document_id,revision_id:input.revision_id,target_profile:input.target_profile,layout_environment_id:input.layout_environment_id,status:"queued"};
      jobs.set(job.job_id,job); return structuredClone(job);
    },
    async getExportJob(id){ return structuredClone(jobs.get(id)); },
    async authorizeDownload({job_id,artifact_id}){ return {job_id,artifact_id,download_handle:"download:token"}; },
  };
  const ux=new WebExportUxV1({exportService:service,documentContext:{currentExportContext(){return structuredClone(ctx);}},requestIdFactory:()=>"export-request-0001"});
  return {ux,service,jobs,ctx,createIds};
}

test("job UI exposes coarse stage only and never fabricates percent",()=>{
  const job=normalizeExportJobV1({job_id:"j",revision_id:"r",status:"running"});
  assert.equal(job.stage_label,"Running");
  assert.equal(job.progress_percent,null);
});

test("preview is fenced to exact current revision and admitted target",async()=>{
  const {ux}=setup();
  const preview=await ux.preview("idml:bounded-editable");
  assert.equal(preview.revision_id,"rev:exact-7");
  assert.equal(preview.loss_summary.level,"bounded");
  await assert.rejects(()=>ux.preview("pdf:v1"),/not admitted/);
});

test("create requires matching preview and preserves exact revision/environment/profile",async()=>{
  const {ux}=setup();
  await assert.rejects(()=>ux.start("idml:bounded-editable"),/preview required/);
  await ux.preview("idml:bounded-editable");
  const job=await ux.start("idml:bounded-editable");
  assert.equal(job.revision_id,"rev:exact-7");
  assert.equal(job.layout_environment_id,"env:a");
  assert.equal(job.target_profile,"idml:bounded-editable");
});

test("retry create reuses the same logical request id",async()=>{
  const {ux,service,createIds}=setup();
  let calls=0;
  const original=service.createExport;
  service.createExport=async(input)=>{calls++;createIds.push(input.client_request_id);if(calls===1){const e=new Error("network");e.code="network";throw e;}return original(input);};
  await ux.preview("idml:bounded-editable");
  await assert.rejects(()=>ux.start("idml:bounded-editable"));
  const job=await ux.retryCreate();
  assert.equal(job.status,"queued");
  assert.equal(createIds[0],createIds[1]);
});

test("resume uses job identity and does not consult latest document head",async()=>{
  const {ux,jobs,ctx}=setup();
  jobs.set("job:old",{job_id:"job:old",document_id:"doc:1",revision_id:"rev:old",target_profile:"odg:bounded-editable",layout_environment_id:"env:old",status:"running"});
  ctx.revision_id="rev:new";
  const job=await ux.resume("job:old");
  assert.equal(job.revision_id,"rev:old");
  assert.equal(job.layout_environment_id,"env:old");
});

test("ready download requires fresh authorization",async()=>{
  const {ux,jobs}=setup();
  jobs.set("job:ready",{job_id:"job:ready",document_id:"doc:1",revision_id:"rev:exact-7",target_profile:"idml:bounded-editable",layout_environment_id:"env:a",status:"ready",artifact_id:"artifact:1"});
  await ux.resume("job:ready");
  const grant=await ux.download();
  assert.equal(grant.download_handle,"download:token");
});

test("non-ready jobs never expose artifact download",async()=>{
  const {ux,jobs}=setup();
  jobs.set("job:run",{job_id:"job:run",document_id:"doc:1",revision_id:"rev:exact-7",target_profile:"idml:bounded-editable",layout_environment_id:"env:a",status:"publishing",artifact_id:"artifact:premature"});
  const job=await ux.resume("job:run");
  assert.equal(job.artifact_id,null);
  await assert.rejects(()=>ux.download(),/not ready/);
});

test("resumable local state stores only durable job id, not artifact authority",async()=>{
  const {ux}=setup();
  await ux.preview("idml:bounded-editable");
  await ux.start("idml:bounded-editable");
  assert.deepEqual(ux.resumableState(),{schema_version:"chaptera.web-export-resume.v1",job_id:"job:1"});
});
