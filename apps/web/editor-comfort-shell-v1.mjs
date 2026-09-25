function requireMethod(object, name, label) {
  if (!object || typeof object[name] !== "function") {
    throw new TypeError(label + "." + name + "() is required");
  }
}

function defaultViewport(shell) {
  return {
    width: Math.max(1, Number(shell.host?.clientWidth || 1200)),
    height: Math.max(1, Number(shell.host?.clientHeight || 800)),
  };
}

const VIEW_COMMANDS = new Set(["zoom", "zoom_in", "zoom_out", "zoom_100"]);

export function bindEditorComfortShellV1({
  shell,
  comfort,
  keyboardTarget = globalThis.window ?? null,
  pointerTarget = shell?.host ?? null,
  contextProvider = () => ({focusOwner:"scene"}),
  viewportProvider = null,
}) {
  for (const name of [
    "setView","currentView","setPointerInteractionEnabled","pageCanonicalGeometry","selectedCanonicalBounds"
  ]) requireMethod(shell,name,"shell");
  for (const name of [
    "handleKeyDown","handleKeyUp","handleWheel","panBy","fitPage","fitSelection","viewState"
  ]) requireMethod(comfort,name,"comfort");
  if (!pointerTarget || typeof pointerTarget.addEventListener !== "function") {
    throw new TypeError("pointerTarget EventTarget is required");
  }
  if (!keyboardTarget || typeof keyboardTarget.addEventListener !== "function") {
    throw new TypeError("keyboardTarget EventTarget is required");
  }
  if (typeof contextProvider !== "function") throw new TypeError("contextProvider() required");

  let pan = null;
  const viewport = () => {
    const v = viewportProvider ? viewportProvider() : defaultViewport(shell);
    if (!v || !(v.width > 0) || !(v.height > 0)) throw new RangeError("viewport must be positive");
    return v;
  };
  const syncView = () => shell.setView(comfort.viewState());

  const onKeyDown = (event) => {
    const result = comfort.handleKeyDown(event, contextProvider());
    if (result.command === "pan_mode") shell.setPointerInteractionEnabled(false);
    if (VIEW_COMMANDS.has(result.command)) syncView();
  };
  const onKeyUp = (event) => {
    const result = comfort.handleKeyUp(event);
    if (result.command === "pan_mode_end") {
      if (!pan) shell.setPointerInteractionEnabled(true);
    }
  };
  const onWheel = (event) => {
    const result = comfort.handleWheel(event, contextProvider());
    if (result.handled) syncView();
  };
  const onPointerDown = (event) => {
    if (!comfort.spaceHeld || event.button !== 0) return;
    event.preventDefault?.();
    shell.setPointerInteractionEnabled(false);
    pan = {x:event.clientX,y:event.clientY,pointer_id:event.pointerId};
    try { pointerTarget.setPointerCapture?.(event.pointerId); } catch {}
  };
  const onPointerMove = (event) => {
    if (!pan || event.pointerId !== pan.pointer_id) return;
    event.preventDefault?.();
    const dx = event.clientX - pan.x;
    const dy = event.clientY - pan.y;
    pan.x = event.clientX;
    pan.y = event.clientY;
    comfort.panBy(dx,dy);
    syncView();
  };
  const endPan = (event) => {
    if (!pan) return;
    if (event?.pointerId != null && event.pointerId !== pan.pointer_id) return;
    const pointerId = pan.pointer_id;
    pan = null;
    try { pointerTarget.releasePointerCapture?.(pointerId); } catch {}
    shell.setPointerInteractionEnabled(!comfort.spaceHeld);
  };

  keyboardTarget.addEventListener("keydown",onKeyDown);
  keyboardTarget.addEventListener("keyup",onKeyUp);
  pointerTarget.addEventListener("wheel",onWheel,{passive:false});
  pointerTarget.addEventListener("pointerdown",onPointerDown);
  pointerTarget.addEventListener("pointermove",onPointerMove);
  pointerTarget.addEventListener("pointerup",endPan);
  pointerTarget.addEventListener("pointercancel",endPan);

  return {
    fitPage(pageId = null) {
      const page = shell.pageCanonicalGeometry(pageId);
      if (!page) return false;
      const v = viewport();
      comfort.fitPage({
        viewport_width_css_px:v.width,
        viewport_height_css_px:v.height,
        page_width_emu:page.width_emu,
        page_height_emu:page.height_emu,
      });
      syncView();
      return true;
    },
    fitSelection() {
      const bounds = shell.selectedCanonicalBounds();
      if (!bounds) return false;
      const v = viewport();
      comfort.fitSelection({viewport_width_css_px:v.width,viewport_height_css_px:v.height,bounds});
      syncView();
      return true;
    },
    syncView,
    destroy() {
      keyboardTarget.removeEventListener("keydown",onKeyDown);
      keyboardTarget.removeEventListener("keyup",onKeyUp);
      pointerTarget.removeEventListener("wheel",onWheel);
      pointerTarget.removeEventListener("pointerdown",onPointerDown);
      pointerTarget.removeEventListener("pointermove",onPointerMove);
      pointerTarget.removeEventListener("pointerup",endPan);
      pointerTarget.removeEventListener("pointercancel",endPan);
      if (pan) endPan();
      shell.setPointerInteractionEnabled(true);
    },
  };
}
