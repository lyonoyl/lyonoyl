import {
  act,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import type { SentHereDto } from "@/shared/ipc/generated/SentHereDto";

import { SentHereReader } from "./SentHereReader";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return { ...actual, askAboutAWrittenLink: vi.fn(), openLink: vi.fn() };
});

const client = await import("@/shared/ipc/client");
const asking = vi.mocked(client.askAboutAWrittenLink);
const opening = vi.mocked(client.openLink);

beforeEach(() => {
  asking.mockReset();
  opening.mockReset();
  opening.mockResolvedValue(undefined);
});

const SENT: SentHereDto = {
  id: 4,
  sender: "zeta@example.com",
  to: ["kappa@example.net", "sigma@example.org"],
  cc: ["records@example.com"],
  bcc: ["archive@example.com"],
  subject: "About Tuesday",
  body: "Shall we meet\n\n  at noon?",
  preview: "Shall we meet at noon?",
  files: ["agenda.pdf", "notes.txt"],
  writtenAt: 1_700_000_000,
  onItsWay: true,
};

function show(sent: SentHereDto) {
  render(
    <I18nProvider initial="en">
      <SentHereReader sent={sent} accountId="acc-1" />
    </I18nProvider>,
  );
  return screen.getByRole("article");
}

/** What each label in the header says, in the order it is drawn. */
function header(article: HTMLElement): [string, string][] {
  return [...article.querySelectorAll("dt")].map((term) => [
    term.textContent ?? "",
    term.nextElementSibling?.textContent ?? "",
  ]);
}

describe("a message sent from here, in the reader", () => {
  it("shows who it is from and who it went to, what it says and what it carries", () => {
    const article = show(SENT);

    expect(
      within(article).getByRole("heading", { level: 1 }).textContent,
    ).toBe("About Tuesday");
    expect(header(article)).toEqual([
      ["From", "zeta@example.com"],
      ["To", "kappa@example.net and sigma@example.org"],
      ["Copy to", "records@example.com"],
      ["Blind copy to", "archive@example.com"],
    ]);
    // As written, line breaks and all.
    expect(
      within(article).getByText(
        (_, element) =>
          element?.tagName === "DIV" &&
          element.textContent === "Shall we meet\n\n  at noon?",
      ).className,
    ).toContain("whitespace-pre-wrap");
    expect(
      within(article)
        .getAllByRole("listitem")
        .map((file) => file.textContent),
    ).toEqual(["agenda.pdf", "notes.txt"]);
  });

  it("leaves out a field nobody was put in, and the files when there are none", () => {
    const article = show({ ...SENT, cc: [], bcc: [], files: [] });

    expect(header(article)).toEqual([
      ["From", "zeta@example.com"],
      ["To", "kappa@example.net and sigma@example.org"],
    ]);
    expect(within(article).queryByText("Files")).toBeNull();
    expect(within(article).queryByRole("list")).toBeNull();
  });

  it("names a message with no subject as the list does", () => {
    const article = show({ ...SENT, subject: "   " });

    expect(
      within(article).getByRole("heading", { level: 1 }).textContent,
    ).toBe("(no subject)");
  });

  it("draws text that reads like markup as the text it is", () => {
    // What was quoted from a message being answered is part of what was written, and nothing in
    // it becomes part of the window.
    const article = show({
      ...SENT,
      body: "<b>bold</b><img src=x onerror=alert(1)>",
      subject: "<i>About</i>",
      files: ["<script>.txt"],
    });

    expect(article.querySelector("b, i, img, script")).toBeNull();
    expect(article.textContent).toContain(
      "<b>bold</b><img src=x onerror=alert(1)>",
    );
    expect(article.textContent).toContain("<i>About</i>");
  });

  it("keeps the direction each piece was written in", () => {
    const article = show(SENT);

    expect(
      within(article).getByRole("heading", { level: 1 }).getAttribute("dir"),
    ).toBe("auto");
    for (const description of article.querySelectorAll("dd")) {
      expect(description.getAttribute("dir")).toBe("auto");
    }
    for (const file of within(article).getAllByRole("listitem")) {
      expect(file.getAttribute("dir")).toBe("auto");
    }
  });

  it("offers nothing to act on", () => {
    // Answering, filing and throwing away are done to the server's copy, which takes this one's
    // place in seconds; a button here would act on a message no server holds yet.
    const article = show(SENT);

    expect(within(article).queryByRole("button")).toBeNull();
  });

  it("offers the addresses it was written with as links, and leaves the rest as it was written", () => {
    const article = show({
      ...SENT,
      body: "The agenda is at https://example.com/agenda.\nOr www.example.org, as before.",
    });

    const links = within(article).getAllByRole("button");
    expect(links.map((link) => link.textContent)).toEqual([
      "https://example.com/agenda",
      "www.example.org",
    ]);
    // Every character of what was written is still there, in its place.
    expect(article.textContent).toContain(
      "The agenda is at https://example.com/agenda.\nOr www.example.org, as before.",
    );
  });

  it("asks the core about the address pressed, and asks the person the question every link gets", async () => {
    asking.mockResolvedValue({
      name: "held-1",
      host: "example.com",
      hostAsWritten: null,
      url: "https://example.com/agenda",
    });
    const article = show({ ...SENT, body: "See https://example.com/agenda now" });

    fireEvent.click(
      within(article).getByRole("button", { name: "https://example.com/agenda" }),
    );

    const question = await screen.findByRole("dialog", { name: "Open this link?" });
    expect(asking).toHaveBeenCalledWith("acc-1", 4, "https://example.com/agenda");
    expect(question.textContent).toContain("example.com");

    fireEvent.click(within(question).getByRole("button", { name: "Open" }));
    expect(opening).toHaveBeenCalledWith("held-1");
  });

  it("asks nothing when the core does not find the address in the message", async () => {
    // Read back since it was drawn, or not an address it holds: nothing to ask about, nothing opened.
    asking.mockRejectedValue({ code: "invalidRequest", message: "", path: null });
    const article = show({ ...SENT, body: "See https://example.com/agenda now" });

    fireEvent.click(within(article).getByRole("button"));
    await act(async () => {});

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(opening).not.toHaveBeenCalled();
  });
});
