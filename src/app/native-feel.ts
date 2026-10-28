/**
 * Makes the window behave like an application rather than like a page.
 *
 * A webview brings the browser's habits with it: right-clicking a folder name offers Reload and
 * Inspect, dragging across it selects it blue, and dragging an image tears a copy off. None of
 * that belongs to a control. Text a person may genuinely want to take with them — the message they
 * are reading — is marked `data-selectable`, and there the platform's own menu is left alone.
 *
 * What this refuses is the browser doing something of its own. Where the application itself moves
 * something — a mailbox dragged into a new place in the sidebar — the row is marked `data-drags`,
 * and the refusal steps aside for it. Without that mark the row could be picked up and nothing
 * would happen: the drag was cancelled here, one level above every handler that was listening for
 * it, and the feature looked built and was dead.
 */

/**
 * Marks the document when the window has no bar of its own.
 *
 * On macOS the content reaches the top of the window and the close, minimise and zoom buttons
 * float over its left corner. Everything below the corner may use that space; what is under the
 * buttons may not. The mark is what the stylesheet keys that inset off.
 *
 * Read from the user agent because it is the one thing available before anything else loads, and
 * being wrong about it costs a few pixels of padding rather than anything a person notices.
 */
export function markWindowControls(root: Document = document): void {
  const overlaid = /Mac/i.test(root.defaultView?.navigator.userAgent ?? "");

  if (overlaid) root.documentElement.dataset.titlebar = "overlay";
}

/** Installs the handlers. Answers with a function that removes them again. */
export function applyNativeFeel(root: Document = document): () => void {
  const onContextMenu = (event: Event) => {
    if (withinSelectableText(event.target)) return;
    event.preventDefault();
  };

  const onDragStart = (event: Event) => {
    // Nothing in the interface is a thing to drag out of the window — except the things this
    // application drags around inside it, which say so.
    if (withinSelectableText(event.target) || theApplicationDragsIt(event.target)) {
      return;
    }
    event.preventDefault();
  };

  root.addEventListener("contextmenu", onContextMenu);
  root.addEventListener("dragstart", onDragStart);

  return () => {
    root.removeEventListener("contextmenu", onContextMenu);
    root.removeEventListener("dragstart", onDragStart);
  };
}

function withinSelectableText(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  return target.closest("[data-selectable], input, textarea") !== null;
}

/** Something the window moves itself, rather than something the browser would carry out of it. */
function theApplicationDragsIt(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  return target.closest("[data-drags]") !== null;
}
