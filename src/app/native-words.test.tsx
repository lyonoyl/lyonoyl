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

/** Every word of the menu bar in English, each in its own place. The product's name never breaks. */
const ENGLISH_MENU = {
  settings: "Settings…",
  about: "About Lyonoyl\u00a0Mail",
  services: "Services",
  hide: "Hide Lyonoyl\u00a0Mail",
  hideOthers: "Hide Others",
  quit: "Quit Lyonoyl\u00a0Mail",
  file: "File",
  fileClose: "Close Window",
  edit: "Edit",
  undo: "Undo",
  redo: "Redo",
  cut: "Cut",
  copy: "Copy",
  paste: "Paste",
  selectAll: "Select All",
  view: "View",
  window: "Window",
  minimize: "Minimize",
  zoom: "Zoom",
  windowClose: "Close Window",
  help: "Help",
};

describe("the words the core shows by itself", () => {
  it("hands over every word of the menu bar, and what a notification and its buttons say", async () => {
    render(
      <I18nProvider initial="en">
        <NativeWords />
      </I18nProvider>,
    );

    await waitFor(() => expect(menu).toHaveBeenLastCalledWith(ENGLISH_MENU));
    expect(notification).toHaveBeenCalledTimes(1);
    // Each button takes a key of its own. In English the one that throws a message away says what
    // the reader's own button for that says.
    expect(notification).toHaveBeenLastCalledWith(
      "New mail",
      "Move to trash",
      "Move to junk",
    );
  });

  it("hands them over again in the language a person chooses", async () => {
    render(
      <I18nProvider initial="en">
        <Chooser />
        <NativeWords />
      </I18nProvider>,
    );
    await waitFor(() => expect(menu).toHaveBeenLastCalledWith(ENGLISH_MENU));

    act(() => choose("ru"));

    // Every word again, the menu bar made whole in the language chosen: the words macOS writes in
    // Russian, and the product's name unbroken.
    await waitFor(() =>
      expect(menu).toHaveBeenLastCalledWith({
        settings: "Настройки…",
        about: "О приложении «Lyonoyl\u00a0Mail»",
        services: "Службы",
        hide: "Скрыть Lyonoyl\u00a0Mail",
        hideOthers: "Скрыть остальные",
        quit: "Завершить Lyonoyl\u00a0Mail",
        file: "Файл",
        fileClose: "Закрыть окно",
        edit: "Правка",
        undo: "Отменить",
        redo: "Повторить",
        cut: "Вырезать",
        copy: "Скопировать",
        paste: "Вставить",
        selectAll: "Выбрать все",
        view: "Вид",
        window: "Окно",
        // A one-letter preposition kept with its word, as macOS writes it.
        minimize: "Убрать в\u00a0Dock",
        zoom: "Изменить масштаб",
        windowClose: "Закрыть окно",
        help: "Справка",
      }),
    );
    expect(notification).toHaveBeenCalledTimes(2);
    expect(notification).toHaveBeenLastCalledWith(
      "Новая почта",
      "В корзину",
      "В спам",
    );
  });

  it("says nothing when the core refuses the word", async () => {
    // The menu bar stays as it was and the keys still open the settings: nothing to show.
    menu.mockRejectedValue(new Error("refused"));
    render(
      <I18nProvider initial="en">
        <NativeWords />
      </I18nProvider>,
    );

    await waitFor(() => expect(menu).toHaveBeenCalled());
  });
});
