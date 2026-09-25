import test from "node:test";
import assert from "node:assert/strict";
import { WebEditorComfortV1, capabilityContextMenuV1 } from "./editor-comfort-v1.mjs";

function setup(overrides = {}) {
  const calls = [];
  const commands = {
    undo(){ calls.push(["undo"]); },
    redo(){ calls.push(["redo"]); },
    flushSaveFrontier(){ calls.push(["save"]); },
    cancelTransient(){ calls.push(["cancel"]); },
    deleteSelection(id){ calls.push(["delete", id]); },
    showShortcutHelp(){ calls.push(["help"]); },
  };
  const stateProvider = {
    currentProductState(){
      return { product_state:"saved", label:"Saved", durable:true, can_edit:true };
    },
  };
  return {
    calls,
    comfort: new WebEditorComfortV1({ stateProvider, commands, ...overrides }),
    stateProvider,
  };
}

function key(key, extra = {}) {
  let prevented = false;
  return {
    key, ctrlKey:false, metaKey:false, altKey:false, shiftKey:false, isComposing:false,
    preventDefault(){ prevented = true; },
    get prevented(){ return prevented; },
    ...extra,
  };
}

test("save surface is consumed from authoritative state provider, not inferred from canvas or transport", () => {
  const { comfort } = setup();
  assert.deepEqual(comfort.saveSurface(), {
    product_state:"saved", label:"Saved", can_edit:true, durable:true, local_recovery:null, attention_code:null,
  });
});

test("pan and zoom mutate only local view state", () => {
  const { comfort } = setup();
  const before = comfort.localUiState();
  comfort.panBy(20, -10);
  comfort.zoomIn();
  const after = comfort.localUiState();
  assert.notDeepEqual(after.view, before.view);
  assert.equal("revision_id" in after, false);
  assert.equal("document_id" in after, false);
});

test("fit page and fit selection produce bounded view transforms", () => {
  const { comfort } = setup();
  const page = comfort.fitPage({ viewport_width_css_px:1000, viewport_height_css_px:800, page_width_emu:8_500_000, page_height_emu:11_000_000 });
  assert.ok(page.zoom > 0 && page.zoom <= 8);
  const selection = comfort.fitSelection({ viewport_width_css_px:1000, viewport_height_css_px:800, bounds:{x:100000,y:200000,width:2_000_000,height:1_000_000} });
  assert.ok(Number.isFinite(selection.pan_x_css_px));
  assert.ok(Number.isFinite(selection.pan_y_css_px));
});

test("Ctrl/Cmd+S is intercepted as save-frontier action even when no view mutation occurs", () => {
  const { comfort, calls } = setup();
  const event = key("s", { ctrlKey:true });
  const result = comfort.handleKeyDown(event, { focusOwner:"scene" });
  assert.equal(result.command, "save_frontier");
  assert.equal(event.prevented, true);
  assert.deepEqual(calls, [["save"]]);
});

test("story/inspector/modal contexts fence document undo/delete shortcuts", () => {
  for (const owner of ["story","inspector","modal"]) {
    const { comfort, calls } = setup();
    comfort.handleKeyDown(key("z", { ctrlKey:true }), { focusOwner:owner });
    comfort.handleKeyDown(key("Delete"), { focusOwner:owner, capabilities:{can_delete:true}, selection:{node_id:"n1"} });
    assert.deepEqual(calls, []);
  }
});

test("composition fences global commands", () => {
  const { comfort, calls } = setup();
  comfort.handleKeyDown(key("Delete", { isComposing:true }), { focusOwner:"scene", composing:true, capabilities:{can_delete:true}, selection:{node_id:"n1"} });
  assert.deepEqual(calls, []);
});

test("scene shortcuts route undo redo zoom delete and help", () => {
  const { comfort, calls } = setup();
  comfort.handleKeyDown(key("z", { ctrlKey:true }), { focusOwner:"scene" });
  comfort.handleKeyDown(key("z", { ctrlKey:true, shiftKey:true }), { focusOwner:"scene" });
  const z0 = comfort.zoomPercent();
  comfort.handleKeyDown(key("+", { ctrlKey:true }), { focusOwner:"scene" });
  assert.ok(comfort.zoomPercent() > z0);
  comfort.handleKeyDown(key("Delete"), { focusOwner:"scene", capabilities:{can_delete:true}, selection:{node_id:"node:7"} });
  comfort.handleKeyDown(key("?"), { focusOwner:"scene" });
  assert.deepEqual(calls, [["undo"],["redo"],["delete","node:7"],["help"]]);
});

test("Escape cancels only an explicit transient focus owner", () => {
  const { comfort, calls } = setup();
  comfort.handleKeyDown(key("Escape"), { focusOwner:"scene" });
  comfort.handleKeyDown(key("Escape"), { focusOwner:"transient" });
  assert.deepEqual(calls, [["cancel"]]);
});

test("context menu is capability-filtered and unavailable in text/composition contexts", () => {
  assert.deepEqual(capabilityContextMenuV1({focusOwner:"story", selection:{node_id:"n"}, capabilities:{can_delete:true}}), []);
  assert.deepEqual(capabilityContextMenuV1({focusOwner:"scene", composing:true, selection:{node_id:"n"}, capabilities:{can_delete:true}}), []);
  assert.deepEqual(capabilityContextMenuV1({
    focusOwner:"scene", selection:{node_id:"n"}, capabilities:{can_copy:true,can_delete:true,can_replace_image:false}
  }), ["copy","delete"]);
});

test("wheel zoom and pan are local view commands and text owners retain browser behavior", () => {
  const { comfort } = setup();
  let prevented = false;
  const before = comfort.viewState();
  comfort.handleWheel({deltaX:0,deltaY:-100,ctrlKey:true,metaKey:false,preventDefault(){prevented=true;}},{focusOwner:"scene"});
  assert.equal(prevented,true);
  assert.ok(comfort.viewState().zoom > before.zoom);
  const held = comfort.viewState();
  const result = comfort.handleWheel({deltaX:5,deltaY:10,ctrlKey:false,metaKey:false},{focusOwner:"story"});
  assert.equal(result.handled,false);
  assert.deepEqual(comfort.viewState(), held);
});
