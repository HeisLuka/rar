import test from "node:test";
import assert from "node:assert/strict";
import { BrowserEditorShellV1 } from "./editor-shell-v1.mjs";
import { WebEditorComfortV1 } from "./editor-comfort-v1.mjs";
import { bindEditorComfortShellV1 } from "./editor-comfort-shell-v1.mjs";

class Target {
  constructor(){this.h=new Map();this.clientWidth=1000;this.clientHeight=800;}
  addEventListener(t,f){const a=this.h.get(t)||[];a.push(f);this.h.set(t,a);}
  removeEventListener(t,f){this.h.set(t,(this.h.get(t)||[]).filter(x=>x!==f));}
  emit(t,e){for(const f of this.h.get(t)||[])f(e);}
  setPointerCapture(){}
  releasePointerCapture(){}
}

function event(key,extra={}){
  let prevented=false;
  return {key,ctrlKey:false,metaKey:false,altKey:false,shiftKey:false,isComposing:false,preventDefault(){prevented=true;},get prevented(){return prevented;},...extra};
}

function comfort(){
  return new WebEditorComfortV1({
    stateProvider:{currentProductState(){return {product_state:"saved",label:"Saved",durable:true,can_edit:true};}},
    commands:{undo(){},redo(){},flushSaveFrontier(){},cancelTransient(){},deleteSelection(){},showShortcutHelp(){}},
  });
}

test("BrowserEditorShell public view seam forwards normalized view to live renderer",()=>{
  const shell=new BrowserEditorShellV1({host:{},service:{commit(){},sceneForRevision(){}}});
  let received=null;
  shell.snapshot={pages:[],nodes:[]};
  shell.renderer={setView(v){received=v;},updateOverlay(){}};
  const next=shell.setView({zoom:2,pan_x_css_px:10,pan_y_css_px:-4});
  assert.equal(next.zoom,2);
  assert.equal(received.zoom,2);
  assert.equal(shell.currentView().pan_x_css_px,10);
});

test("shell exposes canonical page and selected bounds without display geometry",()=>{
  const shell=new BrowserEditorShellV1({host:{},service:{commit(){},sceneForRevision(){}}});
  shell.snapshot={
    pages:[{page_id:"p1",width_emu:1000,height_emu:2000}],
    nodes:[{node_id:"n1",bounds:{x:10,y:20,width:30,height:40}}],
  };
  shell.renderer={setView(){},updateOverlay(){}};
  shell.selection.select("n1");
  assert.deepEqual(shell.pageCanonicalGeometry(),{page_id:"p1",width_emu:1000,height_emu:2000});
  assert.deepEqual(shell.selectedCanonicalBounds(),{x:10,y:20,width:30,height:40});
});

test("wheel zoom physically updates shell view",()=>{
  const pointer=new Target(), keyboard=new Target(), c=comfort();
  const views=[];
  const shell={
    host:pointer,
    enabled:true,
    setView(v){views.push(structuredClone(v));this.view=v;return v;},
    currentView(){return this.view;},
    setPointerInteractionEnabled(v){this.enabled=v;return v;},
    pageCanonicalGeometry(){return null;},
    selectedCanonicalBounds(){return null;},
  };
  bindEditorComfortShellV1({shell,comfort:c,keyboardTarget:keyboard,pointerTarget:pointer,contextProvider:()=>({focusOwner:"scene"})});
  let prevented=false;
  pointer.emit("wheel",{deltaX:0,deltaY:-100,ctrlKey:true,metaKey:false,preventDefault(){prevented=true;}});
  assert.equal(prevented,true);
  assert.equal(views.at(-1).zoom>1,true);
});

test("Space-drag pan disables semantic pointer admission and moves only view",()=>{
  const pointer=new Target(), keyboard=new Target(), c=comfort();
  const enabled=[]; const views=[];
  const shell={
    host:pointer,
    setView(v){views.push(structuredClone(v));return v;},
    currentView(){return views.at(-1);},
    setPointerInteractionEnabled(v){enabled.push(v);return v;},
    pageCanonicalGeometry(){return null;},
    selectedCanonicalBounds(){return null;},
  };
  bindEditorComfortShellV1({shell,comfort:c,keyboardTarget:keyboard,pointerTarget:pointer,contextProvider:()=>({focusOwner:"scene"})});
  keyboard.emit("keydown",event(" "));
  assert.equal(enabled.at(-1),false);
  pointer.emit("pointerdown",{button:0,clientX:10,clientY:20,pointerId:1,preventDefault(){}});
  pointer.emit("pointermove",{clientX:30,clientY:50,pointerId:1,preventDefault(){}});
  assert.equal(views.at(-1).pan_x_css_px,20);
  assert.equal(views.at(-1).pan_y_css_px,30);
  pointer.emit("pointerup",{pointerId:1});
  assert.equal(enabled.at(-1),false);
  keyboard.emit("keyup",event(" "));
  assert.equal(enabled.at(-1),true);
});

test("Fit Page and Fit Selection consume canonical shell geometry",()=>{
  const pointer=new Target(), keyboard=new Target(), c=comfort();
  const views=[];
  const shell={
    host:pointer,
    setView(v){views.push(structuredClone(v));return v;},
    currentView(){return views.at(-1);},
    setPointerInteractionEnabled(v){return v;},
    pageCanonicalGeometry(){return {page_id:"p1",width_emu:8_500_000,height_emu:11_000_000};},
    selectedCanonicalBounds(){return {x:100_000,y:200_000,width:2_000_000,height:1_000_000};},
  };
  const binding=bindEditorComfortShellV1({shell,comfort:c,keyboardTarget:keyboard,pointerTarget:pointer});
  assert.equal(binding.fitPage(),true);
  const pageZoom=views.at(-1).zoom;
  assert.equal(pageZoom>0,true);
  assert.equal(binding.fitSelection(),true);
  assert.equal(Number.isFinite(views.at(-1).pan_x_css_px),true);
});
