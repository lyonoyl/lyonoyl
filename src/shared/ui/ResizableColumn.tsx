import { useCallback, useRef, useState, type ReactNode } from "react";

import { useTranslation } from "@/shared/i18n";

/**
 * A column the reader can drag wider or narrower by its trailing edge.
 *
 * The width is held here while the drag is happening and handed to the caller once, when the drag
 * ends: storing it on every pixel would be a write per frame, and a drag is one decision.
 *
 * The handle is a real control, so the column can also be sized from the keyboard — a drag target
 * four pixels wide is unusable for anyone who cannot aim a mouse at it.
 */

/** Whether the window around an element runs right to left, as the nearest `dir` says. */
function mirrored(element: Element): boolean {
  return element.closest("[dir]")?.getAttribute("dir") === "rtl";
}

/** Matches the clamp the core applies, so the interface cannot ask for a width it will not keep. */
export const MIN_WIDTH = 140;
export const MAX_WIDTH = 720;

/** How far one arrow key moves the edge. */
const STEP = 16;

/** How long the edge of a column that folds takes to get where it is going, in milliseconds. */
export const EDGE_MOVES_FOR = 200;

function clamp(width: number): number {
  return Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, Math.round(width)));
}

export function ResizableColumn({
  width,
  onResize,
  label,
  className,
  foldsTo,
  folded = false,
  moving = false,
  children,
}: {
  width: number;
  /** Called once the reader lets go, with the width to keep. */
  onResize: (width: number) => void;
  /** Names the edge for a screen reader: "Accounts", "Messages". */
  label: string;
  className?: string;
  /** How wide the column is folded, for a column that folds. */
  foldsTo?: number;
  /**
   * Folded to `foldsTo`. Its edge cannot be dragged there, and the width it was dragged to is
   * kept for when it opens.
   */
  folded?: boolean;
  /**
   * Whether the edge moves to where it goes rather than jumping there, and what is inside is cut
   * off at the edge on the way rather than spilling over the column beside it. Never during a
   * drag, where the edge is wherever the pointer is.
   */
  moving?: boolean;
  children: ReactNode;
}) {
  const [dragging, setDragging] = useState<number | null>(null);
  const column = useRef<HTMLElement>(null);
  const latest = useRef(width);

  const shown =
    folded && foldsTo !== undefined ? foldsTo : (dragging ?? width);
  latest.current = shown;

  const start = useCallback((event: React.PointerEvent) => {
    // The pointer is captured, so a drag that leaves the window keeps working and ends where the
    // reader lets go rather than wherever the cursor last crossed a border.
    event.currentTarget.setPointerCapture(event.pointerId);
    setDragging(column.current?.getBoundingClientRect().width ?? MIN_WIDTH);
  }, []);

  const move = useCallback(
    (event: React.PointerEvent) => {
      if (dragging === null || column.current === null) return;
      // The handle is on the column's far edge: its right in a left-to-right window, its left in
      // a mirrored one, where the column grows as the pointer moves left.
      const edges = column.current.getBoundingClientRect();
      setDragging(
        clamp(
          mirrored(column.current)
            ? edges.right - event.clientX
            : event.clientX - edges.left,
        ),
      );
    },
    [dragging],
  );

  const finish = useCallback(
    (event: React.PointerEvent) => {
      if (dragging === null) return;
      event.currentTarget.releasePointerCapture(event.pointerId);
      setDragging(null);
      onResize(clamp(latest.current));
    },
    [dragging, onResize],
  );

  const nudge = useCallback(
    (event: React.KeyboardEvent) => {
      // The arrow pointing away from the column makes it wider, whichever way the window runs.
      const [narrower, wider] = mirrored(event.currentTarget as HTMLElement)
        ? ["ArrowRight", "ArrowLeft"]
        : ["ArrowLeft", "ArrowRight"];
      const step =
        event.key === narrower ? -STEP : event.key === wider ? STEP : 0;
      if (step === 0) return;
      event.preventDefault();
      onResize(clamp(width + step));
    },
    [onResize, width],
  );

  return (
    <ColumnFrame
      column={column}
      width={shown}
      moving={moving && dragging === null}
      className={className}
    >
      {children}
      {!folded && (
        <Handle
          label={label}
          width={shown}
          dragging={dragging !== null}
          onStart={start}
          onMove={move}
          onFinish={finish}
          onKeyDown={nudge}
        />
      )}
    </ColumnFrame>
  );
}

function ColumnFrame({
  column,
  width,
  moving,
  className,
  children,
}: {
  column: React.RefObject<HTMLElement | null>;
  width: number;
  moving: boolean;
  className?: string;
  children: ReactNode;
}) {
  return (
    <section
      ref={column as React.RefObject<HTMLElement>}
      style={{
        width: `${width}px`,
        transitionDuration: moving ? `${EDGE_MOVES_FOR}ms` : undefined,
      }}
      className={`relative flex shrink-0 flex-col ${
        moving
          ? "overflow-hidden transition-[width] ease-out motion-reduce:transition-none"
          : ""
      } ${className ?? ""}`}
    >
      {children}
    </section>
  );
}

function Handle({
  label,
  width,
  dragging,
  onStart,
  onMove,
  onFinish,
  onKeyDown,
}: {
  label: string;
  width: number;
  dragging: boolean;
  onStart: (event: React.PointerEvent) => void;
  onMove: (event: React.PointerEvent) => void;
  onFinish: (event: React.PointerEvent) => void;
  onKeyDown: (event: React.KeyboardEvent) => void;
}) {
  const t = useTranslation();

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={t("shell.resize", { pane: label })}
      aria-valuenow={width}
      aria-valuemin={MIN_WIDTH}
      aria-valuemax={MAX_WIDTH}
      tabIndex={0}
      onPointerDown={onStart}
      onPointerMove={onMove}
      onPointerUp={onFinish}
      onPointerCancel={onFinish}
      onKeyDown={onKeyDown}
      // Wider than it looks: a four-pixel target is one nobody can hit on purpose.
      className={`absolute top-0 -end-1 z-10 h-full w-2 cursor-col-resize
        after:absolute after:inset-y-0 after:start-1 after:w-px after:bg-border
        hover:after:bg-primary/40 focus-visible:outline-none focus-visible:after:bg-primary
        ${dragging ? "after:bg-primary" : ""}`}
    />
  );
}
