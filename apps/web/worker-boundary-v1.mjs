function tuple(value={}) { return Object.freeze({scene:value.scene??0,view:value.view??0,resource:value.resource??0,surface:value.surface??0,worker:value.worker??0,request:value.request??0}); }
function same(a,b){return ["scene","view","resource","surface","worker","request"].every(k=>a[k]===b[k]);}

export class WorkerBoundaryV1 {
  constructor({maxInflight=2,maxBytes=8*1024*1024}={}) { this.maxInflight=maxInflight; this.maxBytes=maxBytes; this.workerGeneration=1; this.inflight=new Map(); this.seq=0; this.metrics={submitted:0,dropped:0,stale:0,bytes_cloned:0,bytes_transferred:0,peak_inflight_bytes:0,restarts:0}; }
  submit({kind="derived",generations={},bytes=0,transport="clone"}={}) {
    if (!Number.isSafeInteger(bytes)||bytes<0) throw new RangeError("bytes");
    const currentBytes=[...this.inflight.values()].reduce((n,x)=>n+x.bytes,0);
    if(this.inflight.size>=this.maxInflight || currentBytes+bytes>this.maxBytes){this.metrics.dropped++; return null;}
    const id=++this.seq; const fence=tuple({...generations,worker:this.workerGeneration,request:id});
    this.inflight.set(id,{id,kind,bytes,transport,fence}); this.metrics.submitted++;
    if(transport==="transfer") this.metrics.bytes_transferred+=bytes; else this.metrics.bytes_cloned+=bytes;
    this.metrics.peak_inflight_bytes=Math.max(this.metrics.peak_inflight_bytes,currentBytes+bytes);
    return Object.freeze({id,kind,bytes,transport,fence});
  }
  complete(id,currentGenerations={}) {
    const job=this.inflight.get(id); if(!job) return {accepted:false,reason:"unknown_or_released"}; this.inflight.delete(id);
    const expected=tuple({...currentGenerations,worker:this.workerGeneration,request:id});
    if(!same(job.fence,expected)){this.metrics.stale++; return {accepted:false,reason:"stale_generation",job};}
    return {accepted:true,reason:"current",job};
  }
  restart(){this.workerGeneration++; this.inflight.clear(); this.metrics.restarts++; return this.workerGeneration;}
  snapshot(){return Object.freeze({...this.metrics,worker_generation:this.workerGeneration,inflight:this.inflight.size,inflight_bytes:[...this.inflight.values()].reduce((n,x)=>n+x.bytes,0),shared_array_buffer_baseline:false,canonical_authority:"server_editor_session"});}
}

export function transferableArrayBuffer(bytes){ const b=new ArrayBuffer(bytes); return {payload:b,transfer:[b]}; }
