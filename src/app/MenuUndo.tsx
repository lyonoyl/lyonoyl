import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { onUndoAsked, setMenuUndoOffered } from "@/shared/ipc/client";
import { somebodyIsTyping } from "@/shared/ui/keys";

/**
 * The menu bar's Undo, answered the way the window answers Cmd+Z.
 *
 * On macOS the Edit menu's Undo is the application's own. The one the menu starts with undoes text
 * and nothing else: chosen with the pointer after a message was thrown away, it undid what was last
 * typed into a field that had lost the focus, the search box behind the list, and brought nothing
 * back. This one is heard here when it is chosen, and when the key is pressed wherever the window
 * leaves the key to the menu—in a field, with a screen in front, and under a panel—while the item
 * is offered. Windows and Linux have no menu bar, and nothing is heard there.
 *
 * It also tells the menu whether choosing it does anything now, so that it is greyed out when it
 * would not, as the system's is with nothing to undo: greyed, neither the pointer nor the key
 * reaches it. That is under a panel, in a field with no typing to undo, and while the window is
 * not the one in front, where the menu hears nothing for it.
 *
 * Renders nothing.
 */
export function MenuUndo({
  covered,
  screenInFront,
  canTakeBack,
  onTakeBack,
}: {
  /** Whether a panel covers the window, which says it is the only thing on the screen. */
  covered: boolean;
  /** Whether writing, the settings or adding an account stands over the list and the message. */
  screenInFront: boolean;
  /** Whether there is a message the key would take back now. */
  canTakeBack: boolean;
  /** Takes back the last message put away, as the key does. */
  onTakeBack: () => void;
}) {
  // What choosing it would do now, in the order the choosing goes below.
  const typing = useTyping();
  const offered = !typing.inFront
    ? false
    : typing.inAField
      ? typing.canUndo
      : covered
        ? false
        : screenInFront
          ? typing.canUndo
          : canTakeBack;
  /** What the menu was last told, so it is told only of a change. */
  const told = useRef<boolean | null>(null);
  useEffect(() => {
    if (told.current === offered) return;
    told.current = offered;
    void setMenuUndoOffered(offered).catch(() => {
      /* Not told, the item keeps what it last showed until the next change. */
    });
  }, [offered]);

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
 * Whether the window is the one in front, whether the focus is in a field, and whether the engine
 * has typing to undo, kept current.
 *
 * The engine's own answer: measured on it, `queryCommandEnabled("undo")` says yes once something
 * was typed, no once it was all undone, and yes again after a redo, wherever the focus is.
 */
function useTyping() {
  const [typing, setTyping] = useState(read);
  useEffect(() => {
    const look = () =>
      setTyping((was) => {
        const now = read();
        return now.inFront === was.inFront &&
          now.inAField === was.inAField &&
          now.canUndo === was.canUndo
          ? was
          : now;
      });
    // On its way out of a field, or out of the window, the focus has not arrived anywhere yet.
    const soon = () => window.setTimeout(look, 0);
    // Read again once the edit is over as well: measured on the engine, a paste reads inside its
    // own input event as nothing to undo, and as something to undo once the event is over.
    const edited = () => {
      look();
      soon();
    };
    document.addEventListener("focusin", look);
    document.addEventListener("focusout", soon);
    document.addEventListener("input", edited, true);
    window.addEventListener("focus", look);
    window.addEventListener("blur", soon);
    return () => {
      document.removeEventListener("focusin", look);
      document.removeEventListener("focusout", soon);
      document.removeEventListener("input", edited, true);
      window.removeEventListener("focus", look);
      window.removeEventListener("blur", soon);
    };
  }, []);
  return typing;
}

function read() {
  return {
    inFront: document.hasFocus(),
    inAField: somebodyIsTyping(document.activeElement),
    canUndo: canUndoTyping(),
  };
}

function canUndoTyping(): boolean {
  try {
    return typeof document.queryCommandEnabled === "function"
      ? document.queryCommandEnabled("undo")
      : false;
  } catch {
    return false;
  }
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
