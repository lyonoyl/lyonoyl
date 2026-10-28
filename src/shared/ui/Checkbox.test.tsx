import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { Checkbox } from "./Checkbox";

describe("Checkbox", () => {
  it("is ticked by its words as well as by its box", () => {
    const onChange = vi.fn();
    render(<Checkbox label="Keep it" checked={false} onChange={onChange} />);

    fireEvent.click(screen.getByText("Keep it"));

    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("hands back the box unticked when it was ticked", () => {
    const onChange = vi.fn();
    render(<Checkbox label="Keep it" checked onChange={onChange} />);

    fireEvent.click(screen.getByRole("checkbox", { name: "Keep it" }));

    expect(onChange).toHaveBeenCalledWith(false);
  });

  it("reads its hint as its description, and has none without one", () => {
    const { unmount } = render(
      <Checkbox
        label="Keep it"
        hint="What keeping it means"
        checked={false}
        onChange={() => {}}
      />,
    );
    const described = screen
      .getByRole("checkbox", { name: "Keep it" })
      .getAttribute("aria-describedby");
    expect(document.getElementById(described ?? "")?.textContent).toBe(
      "What keeping it means",
    );
    unmount();

    render(<Checkbox label="Keep it" checked={false} onChange={() => {}} />);
    expect(
      screen
        .getByRole("checkbox", { name: "Keep it" })
        .hasAttribute("aria-describedby"),
    ).toBe(false);
  });

  it("is not changed while it is turned off", () => {
    const onChange = vi.fn();
    render(
      <Checkbox label="Keep it" checked={false} disabled onChange={onChange} />,
    );

    fireEvent.click(screen.getByRole("checkbox", { name: "Keep it" }));

    expect(onChange).not.toHaveBeenCalled();
  });
});
