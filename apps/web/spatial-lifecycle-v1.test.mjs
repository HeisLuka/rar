import test from "node:test";
import assert from "node:assert/strict";
import { SpatialWindowV1, identityKey } from "./spatial-lifecycle-v1.mjs";

const ident = (n, pages=["p1","p2","p3"]) => ({
  document_id:"doc:1", revision_id:"rev:"+n, layout_environment_id:"env:1", window_id:"w:1", window_generation:n,
  page_ids:pages, shard_fingerprints:pages.map(p=>p+":sha:"+n),
});
const rows = () => [
  {node_id:"a",page_id:"p1",bounds:{x:-100,y:0,width:100,height:100},paint_order:0,z_order:0},
  {node_id:"b",page_id:"p1",bounds:{x:0,y:0,width:100,height:100},paint_order:1,z_order:1},
  {node_id:"c",page_id:"p2",bounds:{x:0,y:0,width:100,height:100},paint_order:0,z_order:0},
  {node_id:"d",page_id:"p3",bounds:{x:0,y:0,width:100,height:100},paint_order:0,z_order:0},
];

test("point query is fenced to current generation and old generation is rejected",()=>{
  const s=new SpatialWindowV1({cellSizeEmu:100}); const g1=identityKey(ident(1)); s.replaceWindow(ident(1),rows());
  assert.deepEqual(s.queryPoint("p1",{x_emu:10,y_emu:10},g1).candidates.map(x=>x.node_id),["b"]);
  s.replaceWindow(ident(2),rows()); assert.throws(()=>s.queryPoint("p1",{x_emu:10,y_emu:10},g1),/stale/);
});

test("marquee candidate set equals trusted current-window full scan",()=>{
  const s=new SpatialWindowV1({cellSizeEmu:50}); s.replaceWindow(ident(1),rows()); const g=identityKey(ident(1));
  const q={x:-50,y:0,width:120,height:80};
  const got=s.queryBox("p1",q,g).candidates.map(x=>x.node_id);
  const oracle=rows().filter(r=>r.page_id==="p1" && !(r.bounds.x+r.bounds.width<=q.x || q.x+q.width<=r.bounds.x || r.bounds.y+r.bounds.height<=q.y || q.y+q.height<=r.bounds.y)).map(r=>r.node_id).sort();
  assert.deepEqual(got,oracle);
});

test("snap and culling views carry the exact same current generation",()=>{
  const s=new SpatialWindowV1(); s.replaceWindow(ident(1),rows()); const g=identityKey(ident(1));
  const box={x:-200,y:-50,width:500,height:500};
  assert.equal(s.snapAnchors("p1",box,g).generation_key,g);
  assert.equal(s.cullCandidates("p1",box,g).generation_key,g);
});

test("one-node patch equals clean rebuild results",()=>{
  const s=new SpatialWindowV1({cellSizeEmu:100}); s.replaceWindow(ident(1),rows()); const g1=identityKey(ident(1));
  const moved={node_id:"b",page_id:"p1",bounds:{x:500,y:0,width:100,height:100},paint_order:1,z_order:1};
  s.applyNodePatch({base_generation_key:g1,next_identity:ident(2),upsert_nodes:[moved]});
  const clean=new SpatialWindowV1({cellSizeEmu:100}); clean.replaceWindow(ident(2),[rows()[0],moved,...rows().slice(2)]);
  const g2=identityKey(ident(2)); const q={x:450,y:-10,width:200,height:200};
  assert.deepEqual(s.queryBox("p1",q,g2).candidates,clean.queryBox("p1",q,g2).candidates);
});

test("off-page geometry remains queryable",()=>{
  const s=new SpatialWindowV1(); s.replaceWindow(ident(1),rows()); const g=identityKey(ident(1));
  assert.deepEqual(s.queryPoint("p1",{x_emu:-50,y_emu:50},g).candidates.map(x=>x.node_id),["a"]);
});

test("unsupported and invalid transformed bounds fail closed",()=>{
  const s=new SpatialWindowV1(); const bad=[
    ...rows(),
    {node_id:"bad1",page_id:"p1",bounds:{x:0,y:0,width:10,height:10},bounds_fidelity:"unsupported"},
    {node_id:"bad2",page_id:"p1",bounds:{x:0,y:0,width:-1,height:10}},
  ];
  const r=s.replaceWindow(ident(1),bad); assert.equal(r.indexed_nodes,4); assert.equal(r.metrics.skipped_invalid,2);
});

test("page eviction discards local index state and clean revisit reconstructs it",()=>{
  const s=new SpatialWindowV1(); s.replaceWindow(ident(1),rows()); let g=identityKey(ident(1));
  s.evictPage({base_generation_key:g,next_identity:ident(2,["p1","p3"]),page_id:"p2"}); g=identityKey(ident(2,["p1","p3"]));
  assert.equal(s.queryBox("p2",{x:0,y:0,width:100,height:100},g).candidates.length,0);
  s.replaceWindow(ident(3),rows()); g=identityKey(ident(3)); assert.deepEqual(s.queryPoint("p2",{x_emu:10,y_emu:10},g).candidates.map(x=>x.node_id),["c"]);
});

test("equal current windows have equal operation cost independent of total document pages",()=>{
  const a=new SpatialWindowV1({cellSizeEmu:100}), b=new SpatialWindowV1({cellSizeEmu:100});
  a.replaceWindow({...ident(1),total_document_pages:100},rows()); b.replaceWindow({...ident(1),total_document_pages:500},rows());
  const g=identityKey(ident(1)); const q={x:-100,y:0,width:300,height:200};
  assert.deepEqual(a.queryBox("p1",q,g).stats,b.queryBox("p1",q,g).stats);
});

test("index lifecycle emits no authoring mutation or revision",()=>{
  const s=new SpatialWindowV1(); s.replaceWindow(ident(1),rows()); const r=s.receipt();
  assert.equal(r.authority.authoring_mutations,0); assert.equal(r.authority.revisions_emitted,0);
});
