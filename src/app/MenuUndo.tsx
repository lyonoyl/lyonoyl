import { useEffect, useLayoutEffect, useRef } from "react";

import { onUndoAsked } from "@/shared/ipc/client";
import { somebodyIsTyping } from "@/shared/ui/keys";

/**
 * The menu bar's Undo, answered the way the window answers Cmd+Z.
 *
 * On macOS the Edit menu's Undo is the application's own. The one the menu starts with undoes text
 * and nothing else: chosen with the pointer after a message was thrown away, it undid what was last
 * typed into a field that had lost the focus, the search box behind the list, and brought nothing
 * back. This one is heard here when it is chosen, and when the key is pressed wherever the window
 * leaves the key to the menu: in a field, with a screen in front, and under a panel. Windows and
 * Linux have no menu bar, and nothing is heard there.
 *
 * Renders nothing.
 */
export function MenuUndo({
  covered,
  screenInFront,
  onTakeBack,
}: {
  /** Whether a panel covers the window, which says it is the only thing on the screen. */
  covered: boolean;
  /** Whether writing, the settings or adding an account stands over the list and the message. */
  screenInFront: boolean;
  /** Takes back the last message put away, as the key does. */
  onTakeBack: () => void;
}) {
  /** What is true when the item is chosen, rather than when the listener was put in place. */
  const now = useRef({ covered, screenInFront, onTakeBack });
  useLayoutEffect(() => {
    now.current = { covered, screenInFront, onTakeBack };
  });

  useEffect(() => {
    let stopListening: (() => void) | null = null;
    let listening = true;

    void onUndoAsked(() => {
      const { covered, screenInFront, onTakeBack } = now.current;

      // Where somebody types, the field's own undo, as the menu's first item gave it.
      if (somebodyIsTyping(document.activeElement)) {
        undoTyping();
        return;
      }
      // Under a panel, nothing: undone behind it, the typing would change where nobody sees.
      if (covered) return;
      // With a screen in front, the window's last typing, wherever it was, as the system's Undo
      // gave it: the list and the message are not there to take anything back into.
      if (screenInFront) {
        undoTyping();
        return;
      }
      onTakeBack();
    })
      .then((stop) => {
        if (listening) stopListening = stop;
        else stop();
      })
      .catch(() => {
        /* The menu cannot reach this window; the key still takes back what was put away. */
      });

    return () => {
      listening = false;
      stopListening?.();
    };
  }, []);

  return null;
}

/**
 * What the Undo the menu started with did: the last thing typed into a field of this window,
 * undone, wherever the focus is now.
 *
 * The engine's own undo, which its menu item asks for too. Measured on the engine: it undoes the
 * field's typing with the focus in the field or elsewhere, and leaves a redo the system's Redo
 * carries out.
 */
function undoTyping() {
  document.execCommand("undo");
}
