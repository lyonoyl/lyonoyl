import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { PointerHint } from "./PointerHint";

function renderHint(side?: "top" | "right" | "bottom" | "left") {
  render(
    <PointerHint label="Get new mail" side={side}>
      <button type="button" aria-label="Get new mail">
        ↻
      </button>
    </PointerHint>,
  );
  return screen.getByRole("button", { name: "Get new mail" });
}

describe("PointerHint", () => {
  it("names the control in the theme's hint under the pointer, and takes it away with the pointer", async () => {
    const button = renderHint();

    fireEvent.pointerEnter(button);
    expect((await screen.findByRole("tooltip")).textContent).toBe("Get new mail");

    fireEvent.pointerLeave(button);
    await waitFor(() => expect(screen.queryByRole("tooltip")).toBeNull());
  });

  it("opens nothing on the focus, which these controls are handed back by hand", async () => {
    // A hint opened on a focus handed back stands over whatever came next, with no pointer to take
    // it away.
    const button = renderHint();

    act(() => button.focus());
    await act(async () => {});

    expect(document.activeElement).toBe(button);
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("opens where it is told to", async () => {
    const button = renderHint("bottom");

    fireEvent.pointerEnter(button);
    await screen.findByRole("tooltip");

    expect(
      document.querySelector("[data-slot=tooltip-content]")?.getAttribute("data-side"),
    ).toBe("bottom");
  });
});
