import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import { MAX_WIDTH, MIN_WIDTH, ResizableColumn } from "./ResizableColumn";

function renderColumn(
  width = 300,
  onResize = vi.fn(),
  dir: "ltr" | "rtl" = "ltr",
) {
  render(
    <I18nProvider initial="en">
      <div dir={dir}>
        <ResizableColumn width={width} onResize={onResize} label="Mailboxes">
          <p>Contents</p>
        </ResizableColumn>
      </div>
    </I18nProvider>,
  );
  return { onResize, handle: screen.getByRole("separator") };
}

/** happy-dom lays nothing out, so the column's own geometry is what a test decides it is. */
function pretendColumnIsAt(left: number, width: number) {
  const column = screen.getByRole("separator").parentElement;
  if (!column) throw new Error("the handle must sit inside its column");
  column.getBoundingClientRect = () =>
    ({
      left,
      width,
      right: left + width,
      top: 0,
      bottom: 0,
      height: 0,
      x: left,
      y: 0,
    }) as DOMRect;
}

function drag(handle: HTMLElement, toX: number) {
  handle.setPointerCapture = () => {};
  handle.releasePointerCapture = () => {};
  fireEvent.pointerDown(handle, { pointerId: 1 });
  fireEvent.pointerMove(handle, { pointerId: 1, clientX: toX });
  fireEvent.pointerUp(handle, { pointerId: 1 });
}

describe("ResizableColumn", () => {
  it("opens at the width it was given", () => {
    renderColumn(320);
    expect(screen.getByRole("separator").parentElement?.style.width).toBe(
      "320px",
    );
  });

  it("reports the width the reader dragged to, once", () => {
    // A write per frame would be a write per pixel; a drag is one decision.
    const { onResize, handle } = renderColumn(300);
    pretendColumnIsAt(0, 300);

    drag(handle, 420);

    expect(onResize).toHaveBeenCalledTimes(1);
    expect(onResize).toHaveBeenCalledWith(420);
  });

  it("will not let a column be dragged too narrow to grab again", () => {
    const { onResize, handle } = renderColumn(300);
    pretendColumnIsAt(0, 300);

    drag(handle, 4);

    expect(onResize).toHaveBeenCalledWith(MIN_WIDTH);
  });

  it("will not let a column swallow the ones beside it", () => {
    const { onResize, handle } = renderColumn(300);
    pretendColumnIsAt(0, 300);

    drag(handle, 5000);

    expect(onResize).toHaveBeenCalledWith(MAX_WIDTH);
  });

  it("measures from the column's own edge, not from the window's", () => {
    // The message list does not start at zero, and a column that ignores that jumps on grab.
    const { onResize, handle } = renderColumn(300);
    pretendColumnIsAt(240, 300);

    drag(handle, 640);

    expect(onResize).toHaveBeenCalledWith(400);
  });

  it("can be sized from the keyboard", () => {
    // A four-pixel drag target is unusable for anyone who cannot aim a mouse at it.
    const { onResize, handle } = renderColumn(300);

    fireEvent.keyDown(handle, { key: "ArrowRight" });
    fireEvent.keyDown(handle, { key: "ArrowLeft" });

    expect(onResize.mock.calls.map((call) => call[0])).toEqual([316, 284]);
  });

  it("grows as the pointer moves left in a window that runs right to left", () => {
    // Mirrored, the column stands on the other side and its handle is on its left edge.
    const { onResize, handle } = renderColumn(300, vi.fn(), "rtl");
    pretendColumnIsAt(500, 300);

    drag(handle, 380);

    expect(onResize).toHaveBeenCalledWith(420);
  });

  it("is made wider by the arrow pointing away from it, whichever way the window runs", () => {
    // Mirrored, the arrow pointing away from the column is the left one.
    const { onResize, handle } = renderColumn(300, vi.fn(), "rtl");

    fireEvent.keyDown(handle, { key: "ArrowLeft" });
    fireEvent.keyDown(handle, { key: "ArrowRight" });

    expect(onResize.mock.calls.map((call) => call[0])).toEqual([316, 284]);
  });

  it("says what it is and where it stands", () => {
    const { handle } = renderColumn(320);

    expect(handle.getAttribute("aria-label")).toBe("Resize Mailboxes");
    expect(handle.getAttribute("aria-valuenow")).toBe("320");
    expect(handle.getAttribute("aria-valuemin")).toBe(String(MIN_WIDTH));
    expect(handle.getAttribute("aria-valuemax")).toBe(String(MAX_WIDTH));
  });

  it("keeps showing its contents", () => {
    renderColumn();
    expect(screen.getByText("Contents")).toBeTruthy();
  });
});

describe("ResizableColumn, a column that folds", () => {
  function renderFolding({
    folded = false,
    moving = false,
  }: {
    folded?: boolean;
    moving?: boolean;
  }) {
    const column = (folded: boolean, moving: boolean) => (
      <I18nProvider initial="en">
        <ResizableColumn
          width={300}
          onResize={vi.fn()}
          label="Mailboxes"
          foldsTo={72}
          folded={folded}
          moving={moving}
        >
          <p>Contents</p>
        </ResizableColumn>
      </I18nProvider>
    );
    const view = render(column(folded, moving));
    return {
      section: () => screen.getByText("Contents").parentElement!,
      rerender: (next: { folded: boolean; moving: boolean }) =>
        view.rerender(column(next.folded, next.moving)),
    };
  }

  it("is as wide as it folds to, with no edge to drag", () => {
    const { section } = renderFolding({ folded: true });

    expect(section().style.width).toBe("72px");
    expect(screen.queryByRole("separator")).toBeNull();
  });

  it("goes back to the width it was dragged to when it opens", () => {
    const { section, rerender } = renderFolding({ folded: true });

    rerender({ folded: false, moving: false });

    expect(section().style.width).toBe("300px");
    expect(screen.getByRole("separator")).toBeTruthy();
  });

  it("moves its edge only while it is told to, and cuts off what is inside on the way", () => {
    const { section, rerender } = renderFolding({ folded: false });
    expect(section().className).not.toContain("transition-[width]");
    expect(section().style.transitionDuration).toBe("");

    rerender({ folded: true, moving: true });

    const classes = section().className.split(/\s+/);
    expect(classes).toContain("transition-[width]");
    expect(classes).toContain("overflow-hidden");
    // Nothing moves for somebody who asked their system for less motion.
    expect(classes).toContain("motion-reduce:transition-none");
    expect(section().style.transitionDuration).toBe("200ms");

    rerender({ folded: true, moving: false });
    expect(section().className).not.toContain("transition-[width]");
    expect(section().className).not.toContain("overflow-hidden");
  });

  it("follows the pointer during a drag rather than moving after it", () => {
    const { section } = renderFolding({ folded: false, moving: true });
    const handle = screen.getByRole("separator");
    handle.setPointerCapture = () => {};

    fireEvent.pointerDown(handle, { pointerId: 1 });

    expect(section().className).not.toContain("transition-[width]");
    expect(section().style.transitionDuration).toBe("");
  });
});
