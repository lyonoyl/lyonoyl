import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import type { AttachedDto } from "@/shared/ipc/generated/AttachedDto";
import { Attachments } from "./Attachments";
import { forgetEverything } from "./filesAsked";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    openAttachment: vi.fn(),
    saveAttachment: vi.fn(),
  };
});

const { openAttachment, saveAttachment } = await import("@/shared/ipc/client");
const opening = vi.mocked(openAttachment);
const saving = vi.mocked(saveAttachment);

const A_PDF: AttachedDto = {
  at: 2,
  name: "Statement.pdf",
  bytes: "2500000",
  opens: true,
};
const A_DOCUMENT: AttachedDto = {
  at: 3,
  name: "Minutes.docx",
  bytes: "40000",
  opens: false,
};

function show(files: AttachedDto[], messageId = "7") {
  return render(
    <I18nProvider initial="en">
      <Attachments messageId={messageId} files={files} />
    </I18nProvider>,
  );
}

/** Opens the menu beside a file, as a pointer does. */
function theMenuBeside(name: string) {
  fireEvent.pointerDown(
    screen.getByRole("button", { name: `Open or save ${name}` }),
    { button: 0, ctrlKey: false },
  );
}

/** An answer the test gives when it chooses, to look at what is on screen before it. */
function heldBack() {
  let answer: () => void = () => {};
  let refuse: (reason: unknown) => void = () => {};
  const promise = new Promise<void>((resolve, reject) => {
    answer = resolve;
    refuse = reject;
  });
  return { promise, answer, refuse };
}

/** A file's name as a sentence says it: set apart from the words around it. */
const said = (name: string) => `\u2068${name}\u2069`;

beforeEach(() => {
  forgetEverything();
  opening.mockReset();
  saving.mockReset();
  opening.mockResolvedValue(undefined);
  saving.mockResolvedValue({ name: null });
});

describe("Attachments, opening what a message came with", () => {
  it("opens a file the core says opens, by where it sits", async () => {
    show([A_PDF]);

    fireEvent.click(screen.getByRole("button", { name: "Open Statement.pdf" }));

    await waitFor(() => expect(opening).toHaveBeenCalledWith("7", 2));
    expect(saving).not.toHaveBeenCalled();
  });

  it("saves every other file, whatever its name says", async () => {
    // The core read the bytes and said no: a name ending in .pdf is the sender's word, and the
    // window never asks it.
    show([{ ...A_PDF, name: "invoice.pdf", opens: false }, A_DOCUMENT]);

    fireEvent.click(screen.getByRole("button", { name: "Save invoice.pdf" }));
    fireEvent.click(screen.getByRole("button", { name: "Save Minutes.docx" }));

    await waitFor(() => expect(saving).toHaveBeenCalledTimes(2));
    expect(saving).toHaveBeenCalledWith("7", 2);
    expect(saving).toHaveBeenCalledWith("7", 3);
    expect(opening).not.toHaveBeenCalled();
  });

  it("offers a menu beside a file that opens, and saves it from there", async () => {
    show([A_PDF]);

    theMenuBeside("Statement.pdf");
    expect(await screen.findByRole("menuitem", { name: "Open" })).toBeTruthy();
    fireEvent.click(screen.getByRole("menuitem", { name: "Save…" }));

    await waitFor(() => expect(saving).toHaveBeenCalledWith("7", 2));
    expect(opening).not.toHaveBeenCalled();
  });

  it("opens it from the menu as well", async () => {
    show([A_PDF]);

    theMenuBeside("Statement.pdf");
    fireEvent.click(await screen.findByRole("menuitem", { name: "Open" }));

    await waitFor(() => expect(opening).toHaveBeenCalledWith("7", 2));
  });

  it("puts no menu beside a file that is only saved", () => {
    show([A_DOCUMENT]);

    expect(
      screen.queryByRole("button", { name: "Open or save Minutes.docx" }),
    ).toBeNull();
  });

  it("asks once while a file is on its way, and shows that it is", async () => {
    const answer = heldBack();
    opening.mockReturnValue(answer.promise);
    show([A_PDF]);
    const file = screen.getByRole("button", { name: "Open Statement.pdf" });

    fireEvent.click(file);
    fireEvent.click(file);

    expect(opening).toHaveBeenCalledTimes(1);
    expect(file.getAttribute("aria-busy")).toBe("true");
    expect(file.getAttribute("aria-disabled")).toBe("true");

    await act(async () => answer.answer());

    expect(file.getAttribute("aria-busy")).toBeNull();
    expect(file.getAttribute("aria-disabled")).toBeNull();
  });

  it("keeps the focus on a file pressed from the keyboard while it is on its way", async () => {
    // A disabled button gives the focus up, and a person at the keyboard is left nowhere.
    const answer = heldBack();
    opening.mockReturnValue(answer.promise);
    show([A_PDF]);
    const file = screen.getByRole("button", { name: "Open Statement.pdf" });

    file.focus();
    fireEvent.click(file);

    expect((file as HTMLButtonElement).disabled).toBe(false);
    expect(document.activeElement).toBe(file);
    await act(async () => answer.answer());
    expect(document.activeElement).toBe(file);
  });

  it("shows a file still on its way when its row is drawn again, and does not ask again", async () => {
    // The person looked at another message and came back before the file had opened.
    const answer = heldBack();
    opening.mockReturnValue(answer.promise);
    const first = show([A_PDF]);
    fireEvent.click(screen.getByRole("button", { name: "Open Statement.pdf" }));
    first.unmount();

    show([A_PDF]);
    const file = screen.getByRole("button", { name: "Open Statement.pdf" });
    expect(file.getAttribute("aria-busy")).toBe("true");
    fireEvent.click(file);
    expect(opening).toHaveBeenCalledTimes(1);

    // What the first row asked for comes to nothing here, and this row says so.
    await act(async () =>
      answer.refuse({ code: "fileNotOpened", message: "", path: null }),
    );
    expect(file.getAttribute("aria-busy")).toBeNull();
    expect(screen.getByRole("alert").textContent).toBe(
      `${said("Statement.pdf")} could not be opened. You can still save it from the menu beside it`,
    );
  });

  it("says each file that did not open, one line for each", async () => {
    opening.mockRejectedValue({ code: "fileNotOpened", message: "", path: null });
    show([A_PDF, { ...A_PDF, at: 4, name: "Photo.jpg" }]);

    fireEvent.click(screen.getByRole("button", { name: "Open Statement.pdf" }));
    fireEvent.click(screen.getByRole("button", { name: "Open Photo.jpg" }));

    await waitFor(() => expect(screen.getAllByRole("alert")).toHaveLength(2));
    expect(screen.getAllByRole("alert").map((line) => line.textContent)).toEqual([
      `${said("Statement.pdf")} could not be opened. You can still save it from the menu beside it`,
      `${said("Photo.jpg")} could not be opened. You can still save it from the menu beside it`,
    ]);
  });

  it("sets a name written right to left apart from the sentence it is said in", async () => {
    // Laid into a sentence that runs left to right, a name that starts right to left would carry
    // the words after it with it.
    opening.mockRejectedValue({ code: "fileNotOpened", message: "", path: null });
    show([{ ...A_PDF, name: "\u062a\u0642\u0631\u064a\u0631.pdf" }]);

    fireEvent.click(
      screen.getByRole("button", {
        name: "Open \u062a\u0642\u0631\u064a\u0631.pdf",
      }),
    );

    expect((await screen.findByRole("alert")).textContent).toContain(
      "\u2068\u062a\u0642\u0631\u064a\u0631.pdf\u2069 could not be opened",
    );
  });

  it("says a file the message no longer holds, and a message not read just now, as such", async () => {
    opening.mockRejectedValueOnce({ code: "fileNotThere", message: "", path: null });
    show([A_PDF]);
    fireEvent.click(screen.getByRole("button", { name: "Open Statement.pdf" }));
    expect((await screen.findByRole("alert")).textContent).toBe(
      `This message changed after its files were listed, and ${said("Statement.pdf")} was not found. Open another message, then this one again`,
    );

    opening.mockRejectedValueOnce({
      code: "messageNotReadNow",
      message: "",
      path: null,
    });
    fireEvent.click(screen.getByRole("button", { name: "Open Statement.pdf" }));
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toBe(
        "Lyonoyl\u00a0Mail is busy. Try again in a moment",
      ),
    );
  });

  it("says a failure that names no file once, however many files it befell", async () => {
    opening.mockRejectedValue({ code: "messageNotReadNow", message: "", path: null });
    show([A_PDF, { ...A_PDF, at: 4, name: "Photo.jpg" }]);

    fireEvent.click(screen.getByRole("button", { name: "Open Statement.pdf" }));
    fireEvent.click(screen.getByRole("button", { name: "Open Photo.jpg" }));

    await waitFor(() => expect(opening).toHaveBeenCalledTimes(2));
    await waitFor(() =>
      expect(screen.getAllByRole("alert").map((line) => line.textContent)).toEqual([
        "Lyonoyl\u00a0Mail is busy. Try again in a moment",
      ]),
    );
  });

  it("offers no second Open in the menu while the file is on its way", async () => {
    const answer = heldBack();
    opening.mockReturnValue(answer.promise);
    show([A_PDF]);
    fireEvent.click(screen.getByRole("button", { name: "Open Statement.pdf" }));

    expect(
      screen
        .getByRole("button", { name: "Open or save Statement.pdf" })
        .getAttribute("aria-busy"),
    ).toBe("true");
    theMenuBeside("Statement.pdf");
    const item = await screen.findByRole("menuitem", { name: "Open" });
    expect(item.getAttribute("aria-disabled")).toBe("true");

    await act(async () => answer.answer());
  });

  it("asks once for two presses that come before the window has drawn either", async () => {
    // A key held down repeats faster than a frame: the button is not disabled yet for the second.
    const answer = heldBack();
    opening.mockReturnValue(answer.promise);
    show([A_PDF]);
    const file = screen.getByRole("button", { name: "Open Statement.pdf" });

    act(() => {
      file.click();
      file.click();
    });

    expect(opening).toHaveBeenCalledTimes(1);
    await act(async () => answer.answer());
  });

  it("opens two files at once when both are pressed", async () => {
    const first = heldBack();
    opening.mockReturnValueOnce(first.promise);
    show([A_PDF, { ...A_PDF, at: 4, name: "Photo.jpg" }]);

    fireEvent.click(screen.getByRole("button", { name: "Open Statement.pdf" }));
    fireEvent.click(screen.getByRole("button", { name: "Open Photo.jpg" }));

    expect(opening).toHaveBeenCalledWith("7", 2);
    expect(opening).toHaveBeenCalledWith("7", 4);
    await act(async () => first.answer());
  });

  it("says why a file did not open, and nothing before the answer", async () => {
    const answer = heldBack();
    opening.mockReturnValue(answer.promise);
    show([A_PDF]);

    fireEvent.click(screen.getByRole("button", { name: "Open Statement.pdf" }));
    expect(screen.queryByRole("alert")).toBeNull();

    await act(async () =>
      answer.refuse({ code: "fileNotOpened", message: "", path: null }),
    );

    expect(screen.getByRole("alert").textContent).toBe(
      `${said("Statement.pdf")} could not be opened. You can still save it from the menu beside it`,
    );
  });

  it("names the file that did not open where a message came with several", async () => {
    opening.mockImplementation(async (_message, at) => {
      if (at === 4) throw { code: "fileNotOpened", message: "", path: null };
    });
    show([A_PDF, { ...A_PDF, at: 4, name: "Photo.jpg" }]);

    fireEvent.click(screen.getByRole("button", { name: "Open Statement.pdf" }));
    fireEvent.click(screen.getByRole("button", { name: "Open Photo.jpg" }));

    expect((await screen.findByRole("alert")).textContent).toBe(
      `${said("Photo.jpg")} could not be opened. You can still save it from the menu beside it`,
    );
  });

  it("says why a file was not saved, where it says why one did not open", async () => {
    // The line about a file that did not open sends the person to Save…: a save that then fails
    // in silence leaves that line standing over a file that is not saved either.
    saving.mockRejectedValue({ code: "fileNotWritten", message: "", path: null });
    show([A_DOCUMENT]);

    fireEvent.click(screen.getByRole("button", { name: "Save Minutes.docx" }));

    expect((await screen.findByRole("alert")).textContent).toBe(
      "The file could not be saved where you chose",
    );
  });

  it("says nothing when the dialog is closed without saving", async () => {
    saving.mockResolvedValue({ name: null });
    show([A_DOCUMENT]);

    fireEvent.click(screen.getByRole("button", { name: "Save Minutes.docx" }));

    await waitFor(() => expect(saving).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("takes the line about a file that did not open away once Save… is chosen", async () => {
    opening.mockRejectedValue({ code: "fileNotOpened", message: "", path: null });
    const answer = heldBack();
    saving.mockReturnValue(answer.promise.then(() => ({ name: null })));
    show([A_PDF]);
    fireEvent.click(screen.getByRole("button", { name: "Open Statement.pdf" }));
    await screen.findByRole("alert");

    theMenuBeside("Statement.pdf");
    fireEvent.click(await screen.findByRole("menuitem", { name: "Save…" }));

    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    await act(async () => answer.answer());
  });

  it("says an unreachable server as such", async () => {
    // A message this device does not keep is fetched first, and a server that does not answer is
    // a different thing to do something about than a file that will not open.
    opening.mockRejectedValue({ code: "offline", message: "", path: null });
    show([A_PDF]);

    fireEvent.click(screen.getByRole("button", { name: "Open Statement.pdf" }));

    expect((await screen.findByRole("alert")).textContent).toBe(
      "The mail server could not be reached",
    );
  });

  it("does not say under another message what one file did", async () => {
    opening.mockRejectedValue({ code: "fileNotOpened", message: "", path: null });
    const drawn = show([A_PDF]);
    fireEvent.click(screen.getByRole("button", { name: "Open Statement.pdf" }));
    await screen.findByRole("alert");

    drawn.rerender(
      <I18nProvider initial="en">
        <Attachments messageId="8" files={[A_PDF]} />
      </I18nProvider>,
    );
    expect(screen.queryByRole("alert")).toBeNull();

    drawn.rerender(
      <I18nProvider initial="en">
        <Attachments messageId="7" files={[A_PDF]} />
      </I18nProvider>,
    );
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("does not say under another message what a file left behind did", async () => {
    // The answer comes after the person has moved on: it is about the message they left.
    const answer = heldBack();
    opening.mockReturnValue(answer.promise);
    const drawn = show([A_PDF]);
    fireEvent.click(screen.getByRole("button", { name: "Open Statement.pdf" }));

    drawn.rerender(
      <I18nProvider initial="en">
        <Attachments messageId="8" files={[A_PDF]} />
      </I18nProvider>,
    );
    await act(async () =>
      answer.refuse({ code: "fileNotOpened", message: "", path: null }),
    );

    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("takes a failed opening back once it is pressed again", async () => {
    opening.mockRejectedValueOnce({
      code: "fileNotOpened",
      message: "",
      path: null,
    });
    show([A_PDF]);
    const file = screen.getByRole("button", { name: "Open Statement.pdf" });
    fireEvent.click(file);
    await screen.findByRole("alert");

    fireEvent.click(file);

    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(opening).toHaveBeenCalledTimes(2);
  });
});
