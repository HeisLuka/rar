import test from "node:test";
import assert from "node:assert/strict";
import { WebAssetDropUxV1, bindCanvasImageDropPasteV1, classifyCanvasImageFiles, isSupportedCanvasImageFile } from "./asset-drop-ux-v1.mjs";

function png(name="image.png"){ return {name,size:120,type:"image/png"}; }
function jpg(name="image.jpg"){ return {name,size:130,type:"image/jpeg"}; }

function setup(contextOverrides={}) {
  const calls=[];
  let context={
    focus_owner:"scene",document_id:"doc:1",base_revision_id:"rev:1",page_id:"page:1",
    selection:null,capabilities:{can_create_picture:true,can_replace_image:false},
    ...contextOverrides,
  };
  const assetService={
    async importImageAsset({file,client_request_id}) {
      calls.push(["import",file.name,client_request_id]);
      return {asset_id:"asset:1",intrinsic_width_px:1200,intrinsic_height_px:800};
    },
  };
  const pictureService={
    async planCreatePictureFrame(input) {
      calls.push(["plan",input.asset_id,input.page_id,input.document_point]);
      return {page_id:input.page_id,node_id:"node:new",frame:{x:100,y:200,width:1200,height:800}};
    },
    async createPictureFrame(input) {
      calls.push(["create",input.node_id,input.client_operation_id,input.frame]);
      return {document_id:input.document_id,revision_id:"rev:2"};
    },
    async replaceImage(input) {
      calls.push(["replace",input.node_id,input.asset_id,input.client_operation_id]);
      return {document_id:input.document_id,revision_id:"rev:2"};
    },
  };
  const controller=new WebAssetDropUxV1({
    assetService,pictureService,interactionContext:{currentContext(){return structuredClone(context);}},
    requestIdFactory:()=>"image-request-0001",
  });
  return {controller,calls,getContext:()=>context,setContext:(v)=>{context=v;}};
}

test("supported image classification is bounded to PNG/JPEG-like files",()=>{
  assert.equal(isSupportedCanvasImageFile(png()),true);
  assert.equal(isSupportedCanvasImageFile(jpg()),true);
  assert.equal(isSupportedCanvasImageFile({name:"x.gif",size:1,type:"image/gif"}),false);
  assert.equal(classifyCanvasImageFiles([png()]).kind,"single_image");
  assert.equal(classifyCanvasImageFiles([png("a.png"),jpg("b.jpg")]).kind,"multiple_images");
});

test("drop on empty admitted page imports then delegates exact geometry to canonical planner",async()=>{
  const {controller,calls}=setup();
  const state=await controller.handleFile(png(),{document_point:{x_emu:500,y_emu:600},source:"drop"});
  assert.equal(state.phase,"ready");
  assert.equal(state.route,"create_picture");
  assert.equal(state.node_id,"node:new");
  assert.deepEqual(calls.map(c=>c[0]),["import","plan","create"]);
  assert.deepEqual(calls[2][3],{x:100,y:200,width:1200,height:800});
});

test("compatible selected PictureFrame routes to ReplaceImage without create planner",async()=>{
  const {controller,calls}=setup({
    selection:{node_id:"node:picture"},
    capabilities:{can_create_picture:true,can_replace_image:true},
  });
  const state=await controller.handleFile(jpg(),{document_point:{x_emu:1,y_emu:2},source:"drop"});
  assert.equal(state.route,"replace_image");
  assert.deepEqual(calls.map(c=>c[0]),["import","replace"]);
  assert.equal(calls[1][1],"node:picture");
});

test("retry preserves the same client operation identity",async()=>{
  const {controller,calls}=setup({selection:{node_id:"node:picture"},capabilities:{can_replace_image:true}});
  let first=true;
  controller.pictureService.replaceImage=async(input)=>{
    calls.push(["replace",input.client_operation_id]);
    if(first){ first=false; const e=new Error("network"); e.code="network"; e.retryable=true; throw e; }
    return {document_id:input.document_id,revision_id:"rev:2"};
  };
  const failed=await controller.handleFile(png(),{document_point:{x_emu:1,y_emu:1}});
  assert.equal(failed.phase,"error");
  assert.equal(failed.retryable,true);
  const ready=await controller.retry();
  assert.equal(ready.phase,"ready");
  const ids=calls.filter(c=>c[0]==="replace").map(c=>c[1]);
  assert.deepEqual(ids,["image-request-0001","image-request-0001"]);
});

test("non-scene focus fails closed and never imports",async()=>{
  const {controller,calls}=setup({focus_owner:"story"});
  await assert.rejects(()=>controller.handleFile(png(),{document_point:{x_emu:1,y_emu:1}}),/another focus owner/);
  assert.deepEqual(calls,[]);
});

test("create route requires point and does not invent geometry for paste",async()=>{
  const {controller,calls}=setup();
  const state=await controller.handleFile(png(),{document_point:null,source:"paste"});
  assert.equal(state.phase,"error");
  assert.equal(state.error_code,"document_point_required");
  assert.deepEqual(calls.map(c=>c[0]),["import"]);
});

test("document change during asset import fails closed before semantic mutation",async()=>{
  const {controller,calls,setContext}=setup();
  controller.assetService.importImageAsset=async()=>{
    setContext({focus_owner:"scene",document_id:"doc:2",base_revision_id:"rev:x",page_id:"page:2",selection:null,capabilities:{can_create_picture:true}});
    return {asset_id:"asset:1",intrinsic_width_px:10,intrinsic_height_px:10};
  };
  const state=await controller.handleFile(png(),{document_point:{x_emu:1,y_emu:1}});
  assert.equal(state.phase,"error");
  assert.equal(state.error_code,"document_context_changed");
  assert.deepEqual(calls,[]);
});

class Target {
  constructor(){this.h=new Map();}
  addEventListener(t,f){const a=this.h.get(t)||[];a.push(f);this.h.set(t,a);}
  removeEventListener(t,f){this.h.set(t,(this.h.get(t)||[]).filter(x=>x!==f));}
  emit(t,e){for(const f of this.h.get(t)||[])f(e);}
}

test("paste is intercepted only for scene-focused image file items",()=>{
  const canvas=new Target(), clipboard=new Target();
  const calls=[];
  const controller={handleFile(file,args){calls.push([file,args]);return Promise.resolve();}};
  let focus="story";
  bindCanvasImageDropPasteV1({controller,canvasTarget:canvas,clipboardTarget:clipboard,screenToDocumentPoint:()=>({x_emu:1,y_emu:2}),getFocusOwner:()=>focus});
  let prevented=false;
  const event={isComposing:false,preventDefault(){prevented=true;},clipboardData:{items:[{kind:"file",getAsFile(){return png();}}]}};
  clipboard.emit("paste",event);
  assert.equal(prevented,false);
  assert.equal(calls.length,0);
  focus="scene";
  clipboard.emit("paste",event);
  assert.equal(prevented,true);
  assert.equal(calls.length,1);
});

test("drop converts screen point only after exactly one image is admitted",()=>{
  const canvas=new Target();
  const calls=[];
  const controller={handleFile(file,args){calls.push([file,args]);return Promise.resolve();}};
  bindCanvasImageDropPasteV1({controller,canvasTarget:canvas,clipboardTarget:null,screenToDocumentPoint:({x_css_px,y_css_px})=>({x_emu:x_css_px*10,y_emu:y_css_px*10}),getFocusOwner:()=>"scene"});
  let prevented=false;
  canvas.emit("drop",{clientX:3,clientY:4,preventDefault(){prevented=true;},dataTransfer:{files:[png()]}});
  assert.equal(prevented,true);
  assert.deepEqual(calls[0][1].document_point,{x_emu:30,y_emu:40});
});
