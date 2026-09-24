# WEB-ACCESSIBILITY-FOCUS-01 — semantic accessibility and focus V1

This contract turns the prior renderer/accessibility and focus-routing probes into
reusable Web Editor product behavior.

## Authority split

Canonical Scene/authoring identities feed two independent outputs:

- visual rendering through SVG, Canvas2D, WebGL2-hybrid or future backends;
- a semantic accessibility/focus projection keyed by canonical NodeId.

Visual renderer DOM is never accessibility identity authority. Rebuilding or
switching a renderer must not change semantic object ids, logical order,
selection, the active semantic object, or the single roving scene Tab stop.

## Focus ownership

Exactly one product context interprets a key gesture:

- page navigation;
- semantic scene/object navigation;
- Story text editor;
- inspector/form control;
- modal/dialog.

Scene focus may lower Delete, nudge, Undo and object-navigation gestures into
document commands. Story, inspector and modal contexts fence those commands.
Composition state is an explicit stronger fence and is not delegated solely to
the browser event's `isComposing` bit.

Escape only cancels a transient scene operation when scene focus owns the
gesture.

## Browser acceptance

Chromium and Firefox acceptance exercise:

- semantic NodeId mirror projection;
- one roving scene Tab stop;
- renderer switches/rebuilds while semantic focus is active;
- scene command routing;
- inspector/Story/modal shortcut fencing;
- explicit composition fencing even after DOM focus moves;
- context-routed transient Escape;
- product-level Tab order independent of renderer DOM.

## Evidence boundary

This is synthetic BrowserScene product-contract evidence. It does not claim
native OS IME behavior, real screen-reader/assistive-technology acceptance,
macOS menu conventions, high contrast/reduced motion, keyboard
resize/rotate/multiselect, or WCAG conformance.
