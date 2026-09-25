const UI_STAGE = Object.freeze({
  queued: "Queued",
  running: "Running",
  publishing: "Publishing",
  ready: "Ready",
  failed: "Failed",
  cancelled: "Cancelled",
  expired: "Expired",
});

function clone(v){ return v == null ? v : structuredClone(v); }
function req(obj,name,label){ if(!obj || typeof obj[name] !== "function") throw new TypeError(label+"."+name+"() is required"); }

export function normalizeExportJobV1(job){
  if(!job || typeof job !== "object") throw new TypeError("export job required");
  if(typeof job.job_id !== "string" || !job.job_id) throw new TypeError("job_id required");
  if(typeof job.revision_id !== "string" || !job.revision_id) throw new TypeError("revision_id required");
  const status = String(job.status ?? "");
  if(!(status in UI_STAGE)) throw new TypeError("unsupported export job status");
  return Object.freeze({
    job_id: job.job_id,
    document_id: job.document_id ?? null,
    revision_id: job.revision_id,
    target_profile: job.target_profile ?? null,
    layout_environment_id: job.layout_environment_id ?? null,
    status,
    stage_label: UI_STAGE[status],
    artifact_id: status === "ready" ? (job.artifact_id ?? null) : null,
    loss_report_id: job.loss_report_id ?? null,
    error_code: status === "failed" ? (job.error_code ?? "export_failed") : null,
    progress_percent: null,
  });
}

export class WebExportUxV1 {
  constructor({ exportService, documentContext, requestIdFactory = null, onState = null }) {
    for(const name of ["previewExport","createExport","getExportJob","authorizeDownload"]) req(exportService,name,"exportService");
    req(documentContext,"currentExportContext","documentContext");
    this.exportService = exportService;
    this.documentContext = documentContext;
    this.requestIdFactory = requestIdFactory ?? (()=>crypto.randomUUID());
    this.onState = onState;
    this._attempt = null;
    this._state = Object.freeze({
      protocol_version:"chaptera.web-export-ux.v1",
      mode:"idle",
      preview:null,
      job:null,
      error_code:null,
    });
  }

  state(){ return clone(this._state); }

  async preview(targetProfile){
    const ctx = this._context(targetProfile);
    this._set({mode:"previewing",preview:null,error_code:null});
    try{
      const preview = await this.exportService.previewExport(clone(ctx));
      if(!preview || preview.revision_id !== ctx.revision_id || preview.target_profile !== ctx.target_profile){
        throw Object.assign(new Error("preview identity mismatch"),{code:"preview_identity_mismatch"});
      }
      const normalized = Object.freeze({
        document_id: ctx.document_id,
        revision_id: ctx.revision_id,
        target_profile: ctx.target_profile,
        layout_environment_id: ctx.layout_environment_id,
        loss_summary: clone(preview.loss_summary ?? null),
        capability_summary: clone(preview.capability_summary ?? null),
        allowed: preview.allowed !== false,
      });
      this._set({mode:"preview",preview:normalized});
      return clone(normalized);
    }catch(error){ this._fail(error); throw error; }
  }

  async start(targetProfile){
    const ctx = this._context(targetProfile);
    const preview = this._state.preview;
    if(!preview || preview.revision_id !== ctx.revision_id || preview.target_profile !== ctx.target_profile || preview.layout_environment_id !== ctx.layout_environment_id){
      throw new Error("matching export preview required before start");
    }
    if(preview.allowed === false) throw new Error("export target is not admitted");
    const requestId = this.requestIdFactory("export");
    if(typeof requestId !== "string" || requestId.length < 8) throw new TypeError("request id must be bounded string");
    this._attempt = {ctx:clone(ctx),request_id:requestId};
    return this._create(this._attempt);
  }

  async retryCreate(){
    if(!this._attempt) throw new Error("no export create attempt to retry");
    return this._create(this._attempt);
  }

  async resume(jobId){
    if(typeof jobId !== "string" || !jobId) throw new TypeError("jobId required");
    this._set({mode:"loading_job",error_code:null});
    try{
      const job = normalizeExportJobV1(await this.exportService.getExportJob(jobId));
      this._set({mode:"job",job});
      return clone(job);
    }catch(error){ this._fail(error); throw error; }
  }

  async refresh(){
    if(!this._state.job?.job_id) throw new Error("no export job to refresh");
    return this.resume(this._state.job.job_id);
  }

  async download(){
    const job = this._state.job;
    if(!job || job.status !== "ready" || !job.artifact_id) throw new Error("export artifact is not ready");
    const grant = await this.exportService.authorizeDownload({job_id:job.job_id,artifact_id:job.artifact_id});
    if(!grant || typeof grant.download_handle !== "string" || !grant.download_handle){
      throw Object.assign(new Error("download authorization failed"),{code:"download_not_authorized"});
    }
    return clone(grant);
  }

  resumableState(){
    return Object.freeze({schema_version:"chaptera.web-export-resume.v1",job_id:this._state.job?.job_id ?? null});
  }

  async _create(attempt){
    this._set({mode:"creating",error_code:null});
    try{
      const raw = await this.exportService.createExport({...clone(attempt.ctx),client_request_id:attempt.request_id});
      const job = normalizeExportJobV1(raw);
      if(job.revision_id !== attempt.ctx.revision_id || job.target_profile !== attempt.ctx.target_profile || job.layout_environment_id !== attempt.ctx.layout_environment_id){
        throw Object.assign(new Error("created export changed exact input identity"),{code:"export_identity_mismatch"});
      }
      this._set({mode:"job",job});
      return clone(job);
    }catch(error){ this._fail(error); throw error; }
  }

  _context(targetProfile){
    const ctx = this.documentContext.currentExportContext();
    if(!ctx || typeof ctx !== "object") throw new TypeError("export context required");
    const target = String(targetProfile ?? "");
    if(!Array.isArray(ctx.allowed_targets) || !ctx.allowed_targets.includes(target)) throw new Error("target profile not admitted");
    for(const key of ["document_id","revision_id","layout_environment_id"]){
      if(typeof ctx[key] !== "string" || !ctx[key]) throw new TypeError(key+" required");
    }
    return {document_id:ctx.document_id,revision_id:ctx.revision_id,layout_environment_id:ctx.layout_environment_id,target_profile:target};
  }

  _fail(error){ this._set({mode:"error",error_code:error?.code ?? "export_ux_failed"}); }
  _set(patch){ this._state=Object.freeze({...this._state,...patch}); this.onState?.(this.state()); }
}
