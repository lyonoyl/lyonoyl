import { act, fireEvent, render, screen } from "@testing-library/react";
import { Inbox } from "lucide-react";
import { describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import { RailButton } from "./RailButton";

function renderButton(language = "en", hints: string[] = []) {
  const onClick = vi.fn();
  render(
    <I18nProvider initial={language}>
      <RailButton label="Inbox" hints={hints} onClick={onClick}>
        <Inbox />
      </RailButton>
    </I18nProvider>,
  );
  return { onClick, button: screen.getByRole("button") };
}

describe("RailButton", () => {
  it("is named by its label and its lines, which a hint shows on the focus", async () => {
    const { button } = renderButton("en", ["zeta@example.com", "2 unread"]);

    expect(button.textContent).toBe("Inboxzeta@example.com2 unread");
    expect(screen.getByRole("button", { name: "Inbox zeta@example.com 2 unread" })).toBe(
      button,
    );

    act(() => button.focus());
    const hint = await screen.findByRole("tooltip");
    expect(hint.textContent).toBe("Inboxzeta@example.com2 unread");
  });

  it("does what it is for", () => {
    const { button, onClick } = renderButton();

    fireEvent.click(button);

    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("opens its hint towards the mail, whichever way the window runs", async () => {
    const { button } = renderButton("en");
    act(() => button.focus());
    await screen.findByRole("tooltip");
    expect(
      document.querySelector("[data-slot=tooltip-content]")!.getAttribute("data-side"),
    ).toBe("right");
  });

  it("opens its hint to the left in a window that runs right to left", async () => {
    const { button } = renderButton("ar");
    act(() => button.focus());
    await screen.findByRole("tooltip");
    expect(
      document.querySelector("[data-slot=tooltip-content]")!.getAttribute("data-side"),
    ).toBe("left");
  });
});
