import { doubleClickTitleBar, pressTitleBar } from "@/shared/ipc/client";

/**
 * How tall the strip at the top of the window is, in rem: the room `under-window-controls` in
 * `src/index.css` leaves under the window's buttons, which is where a title bar would be.
 */
export const STRIP = 2;

/** What answers the pointer itself wherever it stands: a control, a frame, a row the window drags. */
const ANSWERS_ITSELF = [
  "a[href]",
  "button",
  "input",
  "select",
  "textarea",
  "label",
  "summary",
  "iframe",
  '[contenteditable]:not([contenteditable="false"])',
  '[tabindex]:not([tabindex="-1"])',
  ...[
    "button",
    "link",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "tab",
    "checkbox",
    "radio",
    "switch",
    "slider",
    "spinbutton",
    "combobox",
    "textbox",
    "searchbox",
    "separator",
    "treeitem",
  ].map((role) => `[role="${role}"]`),
  "[data-drags]",
  '[draggable="true"]',
].join(", ");

/**
 * Makes the top of the window answer as a title bar, where the window has none of its own.
 *
 * On macOS the content reaches the top edge and the window's buttons float over its corner
 * (`markWindowControls`), so a press up there lands in the page, and a window with no bar to take
 * hold of could be moved from nowhere. The strip a bar would be in, across every column, answers as
 * the bar would: pressed, it moves the window with the pointer, and double-clicked, it does what a
 * double click on a title bar does, which a person chooses in System Settings.
 *
 * Only where nothing else answers the pointer. A control in the strip stays a control—the search
 * field over the list, the buttons over a message, the cross that leaves a screen, the edge a column
 * is resized by—and so does text a person may take with them, where the press is on the text
 * itself: the subject at the top of a message is selected, and the room above it moves the window.
 *
 * Answers with a function that stops it again.
 */
export function actAsTitleBar(root: Document = document): () => void {
  if (root.documentElement.dataset.titlebar !== "overlay") return () => {};

  // Where the second press of a double click came down, while the button is held. The system's bar
  // answers a double click as the button comes up, and not at all when the pointer moved in between.
  let secondPress: { x: number; y: number } | null = null;

  const onMouseDown = (event: MouseEvent) => {
    secondPress = null;
    if (!onTheStrip(event)) return;

    // Nothing else comes of this press: no caret placed, no text selected, no focus moved.
    event.preventDefault();

    if (event.detail === 1) {
      pressTitleBar().catch(() => {
        /* The window stays where it is, which is all a refusal can mean here. */
      });
    } else {
      secondPress = { x: event.clientX, y: event.clientY };
    }
  };

  const onMouseUp = (event: MouseEvent) => {
    const at = secondPress;
    secondPress = null;
    if (at === null || event.button !== 0 || event.detail !== 2) return;
    if (event.clientX !== at.x || event.clientY !== at.y) return;

    doubleClickTitleBar().catch(() => {
      /* The window stays as it is. */
    });
  };

  root.addEventListener("mousedown", onMouseDown);
  root.addEventListener("mouseup", onMouseUp);

  return () => {
    root.removeEventListener("mousedown", onMouseDown);
    root.removeEventListener("mouseup", onMouseUp);
  };
}

function onTheStrip(event: MouseEvent): boolean {
  // The main button alone: a press with Control held is the secondary click on a Mac.
  if (event.button !== 0 || event.ctrlKey) return false;
  // A third press in a row is neither a press nor a double click.
  if (event.detail !== 1 && event.detail !== 2) return false;

  const target = event.target;
  if (!(target instanceof Element)) return false;

  if (event.clientY >= STRIP * remOf(target.ownerDocument)) return false;
  if (target.closest(ANSWERS_ITSELF) !== null) return false;

  // Text a person may select, pressed on the text itself rather than on the room around it.
  const selectable = target.closest("[data-selectable]") !== null;
  return !(selectable && holdsText(target));
}

/** A rem of the page, in the pixels a press is told where it landed in. */
function remOf(page: Document): number {
  const root = page.defaultView?.getComputedStyle(page.documentElement);
  return parseFloat(root?.fontSize ?? "") || 16;
}

function holdsText(element: Element): boolean {
  return [...element.childNodes].some(
    (node) =>
      node.nodeType === Node.TEXT_NODE &&
      (node.textContent ?? "").trim() !== "",
  );
}
