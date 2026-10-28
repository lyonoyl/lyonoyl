import { act, render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider, useI18n } from "@/shared/i18n";

import { NativeWords } from "./native-words";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    setNotificationWords: vi.fn(),
    setMenuWords: vi.fn(),
  };
});

const { setMenuWords, setNotificationWords } = await import(
  "@/shared/ipc/client"
);
const menu = vi.mocked(setMenuWords);
const notification = vi.mocked(setNotificationWords);

/** A way to change the language from outside, as the settings screen does. */
let choose: (language: "ru") => void = () => {};

function Chooser() {
  const { setPreference } = useI18n();
  choose = setPreference;
  return null;
}

beforeEach(() => {
  menu.mockReset();
  menu.mockResolvedValue(undefined);
  notification.mockReset();
  notification.mockResolvedValue(undefined);
});

describe("the words the core shows by itself", () => {
  it("hands over what the menu's settings item says, and what a notification says", async () => {
    render(
      <I18nProvider initial="en">
        <NativeWords />
      </I18nProvider>,
    );

    await waitFor(() => expect(menu).toHaveBeenLastCalledWith("Settings…"));
    expect(notification).toHaveBeenCalledTimes(1);
  });

  it("hands them over again in the language a person chooses", async () => {
    render(
      <I18nProvider initial="en">
        <Chooser />
        <NativeWords />
      </I18nProvider>,
    );
    await waitFor(() => expect(menu).toHaveBeenLastCalledWith("Settings…"));

    act(() => choose("ru"));

    await waitFor(() =>
      expect(menu).toHaveBeenLastCalledWith("Настройки…"),
    );
    expect(notification).toHaveBeenCalledTimes(2);
  });

  it("says nothing when the core refuses the word", async () => {
    // The menu goes without the item and the keys still open the settings: nothing to show.
    menu.mockRejectedValue(new Error("refused"));
    render(
      <I18nProvider initial="en">
        <NativeWords />
      </I18nProvider>,
    );

    await waitFor(() => expect(menu).toHaveBeenCalled());
  });
});
