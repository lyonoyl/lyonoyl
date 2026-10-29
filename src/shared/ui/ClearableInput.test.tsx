import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { ClearableInput } from "./ClearableInput";

describe("a field with the search box's cross", () => {
  it("is a search field to the engine, which is what draws the cross, and a plain field to a reader", () => {
    render(<ClearableInput aria-label="Group name" maxLength={64} />);

    const field = screen.getByRole("textbox", { name: "Group name" });
    expect(field.getAttribute("type")).toBe("search");
    expect(field.getAttribute("maxLength")).toBe("64");
  });

  it("is a search to a reader only where it is one", () => {
    render(<ClearableInput role="searchbox" aria-label="Search mail" />);

    expect(screen.getByRole("searchbox", { name: "Search mail" })).toBeTruthy();
  });
});
