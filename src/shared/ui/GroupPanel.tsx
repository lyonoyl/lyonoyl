import { useId, useLayoutEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";

/**
 * The panel over the window, drawn at the top of the document so that nothing it is asked from —
 * a scrolling column, a pane sliding in — can clip it or carry it off.
 *
 * The typing starts in its field, and what had the focus before gets it back when the panel goes,
 * if it is still there to take it. Keys stay inside: Escape leaves, and nothing typed reaches a
 * row or a list underneath that would move on a letter or an arrow.
 */
export function GroupPanel({
  title,
  onCancel,
  onSubmit,
  children,
}: {
  title: string;
  onCancel: () => void;
  onSubmit: () => void;
  children: ReactNode;
}) {
  const heading = useId();
  const form = useRef<HTMLFormElement>(null);

  // Before the window is drawn again, in both directions: given back a frame later, the focus
  // sat on nothing for that frame, and a key pressed in it went nowhere.
  useLayoutEffect(() => {
    const before = document.activeElement;
    form.current?.querySelector("input")?.focus();
    return () => {
      if (before instanceof HTMLElement && before.isConnected) before.focus();
    };
  }, []);

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-background/80 p-6"
      onKeyDown={(event) => {
        event.stopPropagation();
        if (event.key === "Escape") onCancel();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={heading}
        className="flex w-full max-w-sm flex-col gap-4 rounded-lg border border-border bg-background p-6 shadow-lg"
      >
        <h1
          id={heading}
          className="text-lg font-semibold tracking-tight break-words"
        >
          {title}
        </h1>
        <form
          ref={form}
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            onSubmit();
          }}
        >
          {children}
        </form>
      </div>
    </div>,
    document.body,
  );
}
