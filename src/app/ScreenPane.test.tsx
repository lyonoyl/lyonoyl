import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ScreenPane } from "./ScreenPane";

/** The corner buttons, in the order they stand in the line. */
function corners() {
  return screen.getAllByRole("button").map((one) => one.getAttribute("aria-label"));
}

function renderPane(system: "mac" | "windows" | "linux") {
  render(
    <ScreenPane
      paneRef={{ current: null }}
      closeLabel="Close"
      onClose={vi.fn()}
      onBack={vi.fn()}
      backLabel="Back"
      system={system}
    >
      <p>A screen</p>
    </ScreenPane>,
  );
}

describe("ScreenPane", () => {
  it("puts the cross where macOS puts a window's own buttons, with the arrow back beside it", () => {
    renderPane("mac");

    expect(corners()).toEqual(["Close", "Back"]);
    expect(screen.getByRole("button", { name: "Close" }).closest("div[class*='sticky']")?.className).toContain(
      "justify-start",
    );
  });

  it("keeps the cross at the end of the line on Windows and Linux, opposite the arrow back", () => {
    for (const system of ["windows", "linux"] as const) {
      renderPane(system);

      expect(corners()).toEqual(["Back", "Close"]);
      expect(
        screen.getByRole("button", { name: "Close" }).closest("div[class*='sticky']")?.className,
      ).toContain("justify-between");
      document.body.innerHTML = "";
    }
  });

  it("names its corner buttons in the theme's hint under the pointer, below them, and not the system's", async () => {
    renderPane("mac");

    for (const name of ["Close", "Back"]) {
      const button = screen.getByRole("button", { name });
      expect(button.getAttribute("title")).toBeNull();

      fireEvent.pointerEnter(button);
      expect((await screen.findByRole("tooltip")).textContent).toBe(name);
      expect(
        document.querySelector("[data-slot=tooltip-content]")?.getAttribute("data-side"),
      ).toBe("bottom");

      fireEvent.pointerLeave(button);
      await waitFor(() => expect(screen.queryByRole("tooltip")).toBeNull());
    }
  });
});
