import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import type { LinkAsked } from "@/shared/ipc/generated/LinkAsked";
import type { MessageBodyDto } from "@/shared/ipc/generated/MessageBodyDto";
import type { ShowPicturesDto } from "@/shared/ipc/generated/ShowPicturesDto";
import { MessageBody } from "./MessageBody";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    openMessageBody: vi.fn(),
    closeMessageBody: vi.fn(),
    onPictureMissing: vi.fn(),
    onMessageLink: vi.fn(),
    openLink: vi.fn(),
    saveAttachment: vi.fn(),
    whatTheWindowWears: vi.fn(),
  };
});

const {
  openMessageBody,
  closeMessageBody,
  onPictureMissing,
  onMessageLink,
  openLink,
  saveAttachment,
} = await import("@/shared/ipc/client");
const open = vi.mocked(openMessageBody);
const wearing = vi.mocked(
  (await import("@/shared/ipc/client")).whatTheWindowWears,
);
const close = vi.mocked(closeMessageBody);
const missing = vi.mocked(onPictureMissing);
/** The handler the component registered, so a test can play the frame coming up empty. */
let cameUpEmpty: (() => void) | null = null;
const onLink = vi.mocked(onMessageLink);
const follow = vi.mocked(openLink);
const saving = vi.mocked(saveAttachment);

/** The handler the component registered, so a test can play the frame following a link. */
let followed: ((link: LinkAsked) => void) | null = null;

function body(over: Partial<MessageBodyDto> = {}): MessageBodyDto {
  return {
    name: "deadbeef",
    url: "lyomsg://localhost/deadbeef",
    kind: "html",
    remoteBlocked: 0,
    remoteNotShown: 0,
    remoteNotHad: 0,
    truncated: false,
    attachments: [],
    ...over,
  };
}

function show(messageId = "7", showPictures: ShowPicturesDto = "whenAsked") {
  return render(
    <I18nProvider initial="en">
      <MessageBody messageId={messageId} showPictures={showPictures} />
    </I18nProvider>,
  );
}

/**
 * Whether the body was asked for with that message, that way of looking and that answer about
 * pictures.
 *
 * The colours the window is wearing are passed along too, and are none of these tests' business:
 * pinning them here would fail this file every time the interface learns to say something new.
 */
function askedFor(messageId: string, form: string, pictures: boolean): boolean {
  return open.mock.calls.some(
    (call) => call[0] === messageId && call[1] === form && call[2] === pictures,
  );
}

/**
 * The element that shows a file's whole name.
 *
 * Found by what it reads as rather than by a single piece of text: the name is drawn in two parts,
 * the start that may be cut and the ending that may not, so no one text node holds all of it. And
 * by the direction it is laid out in, which is left to right whatever the name: the start of a name
 * with no ending holds all of it too, and keeps the name's own direction.
 */
function nameShown(name: string) {
  return (_: string, element: Element | null) =>
    element?.getAttribute("dir") === "ltr" && element.textContent === name;
}

describe("MessageBody", () => {
  it("tells the core what the window is wearing", async () => {
    // A message that says nothing about colour is shown on these. Without them it is painted by
    // whatever the system thinks, which is a black rectangle inside a window that is not black.
    wearing.mockReturnValue({ background: "#282a36", foreground: "#f8f8f2" });
    render(
      <I18nProvider initial="en">
        <MessageBody messageId="7" />
      </I18nProvider>,
    );

    await waitFor(() => expect(open).toHaveBeenCalled());
    expect(open.mock.calls[0]?.[3]).toEqual({
      background: "#282a36",
      foreground: "#f8f8f2",
    });
  });

  it("asks for the body again when the window changes colour", async () => {
    // A body is drawn in the colours of the moment it is asked for, so an open message cannot be
    // repainted — it has to be asked for again. Otherwise choosing a scheme in settings leaves the
    // message that is on screen wearing the one before it.
    render(
      <I18nProvider initial="en">
        <MessageBody messageId="7" />
      </I18nProvider>,
    );
    await waitFor(() => expect(open).toHaveBeenCalled());
    const before = open.mock.calls.length;

    document.documentElement.setAttribute("data-scheme", "dracula");
    document.documentElement.classList.add("dark");

    await waitFor(() => expect(open.mock.calls.length).toBeGreaterThan(before));

    document.documentElement.classList.remove("dark");
    document.documentElement.removeAttribute("data-scheme");
  });

  it("asks for the body again when the light colours change", async () => {
    // The light side has palettes of its own, and a message open while one is chosen has to come
    // back in it as well.
    render(
      <I18nProvider initial="en">
        <MessageBody messageId="7" />
      </I18nProvider>,
    );
    await waitFor(() => expect(open).toHaveBeenCalled());
    const before = open.mock.calls.length;

    document.documentElement.setAttribute("data-light-scheme", "catppuccinLatte");

    await waitFor(() => expect(open.mock.calls.length).toBeGreaterThan(before));

    document.documentElement.removeAttribute("data-light-scheme");
  });

  beforeEach(() => {
    vi.clearAllMocks();
    // Nothing by default: what most of these tests are about is the message, not its surface.
    wearing.mockReturnValue(null);
    open.mockResolvedValue(body());
    close.mockResolvedValue(undefined);
    follow.mockResolvedValue(undefined);
    followed = null;
    onLink.mockImplementation(async (handler) => {
      followed = handler;
      return () => {
        followed = null;
      };
    });
    cameUpEmpty = null;
    missing.mockImplementation(async (handler) => {
      cameUpEmpty = handler;
      return () => {
        cameUpEmpty = null;
      };
    });
  });

  it("shows the body in a frame that is allowed nothing", async () => {
    // The rule this pins is the whole of the third layer: an empty sandbox, and no `allow-*` in it.
    // One of them turns the frame into a place a message can run in.
    show();

    const frame = await screen.findByTitle(/message body/i);

    expect(frame.tagName).toBe("IFRAME");
    expect(frame.getAttribute("sandbox")).toBe("");
    expect(frame.getAttribute("referrerpolicy")).toBe("no-referrer");
    expect(frame.getAttribute("src")).toBe("lyomsg://localhost/deadbeef");
    expect(frame.getAttribute("srcdoc")).toBeNull();
  });

  it("never holds the body itself, only where to fetch it", async () => {
    open.mockResolvedValue(body({ url: "lyomsg://localhost/abc" }));
    show();

    await screen.findByTitle(/message body/i);

    // What came back over IPC is an address and four facts about the message. If a body ever
    // arrived here, it would be parsed by the window that holds IPC.
    expect(Object.keys(open.mock.results[0]!.value ?? {})).not.toContain(
      "html",
    );
  });

  it("says how much of the message was kept out of it", async () => {
    open.mockResolvedValue(body({ remoteBlocked: 3, truncated: true }));
    show();

    await waitFor(() =>
      expect(screen.getByText(/3 pictures were not loaded/i)).toBeTruthy(),
    );
    expect(screen.getByText(/only the beginning/i)).toBeTruthy();
  });

  it("says so when there was nothing to show, instead of an empty frame", async () => {
    open.mockResolvedValue(body({ kind: "nothing" }));
    show();

    await waitFor(() =>
      expect(screen.getByText(/nothing that can be shown/i)).toBeTruthy(),
    );
    expect(screen.queryByTitle(/message body/i)).toBeNull();
  });

  it("lists the files where there is nothing to show but something to save", async () => {
    // A message whose whole content is a file, and a reply too deep to draw whose pictures
    // travelled inside it, both have nothing to show and files to hand over. A page that stopped
    // at the notice would drop them.
    open.mockResolvedValue(
      body({
        kind: "nothing",
        attachments: [{ at: 2, name: "invoice.pdf", bytes: "2500000" }],
      }),
    );
    show();

    await waitFor(() =>
      expect(screen.getByText(/nothing that can be shown/i)).toBeTruthy(),
    );
    expect(screen.getByText(nameShown("invoice.pdf"))).toBeTruthy();
    expect(
      screen.getByRole("button", { name: /save invoice\.pdf/i }),
    ).toBeTruthy();
  });

  it("says the machine is busy rather than that the message is empty", async () => {
    // The two used to be one sentence. A person opening a message while the round that reads mail
    // for the index held every place was told their message had nothing in it — and went to argue
    // with a message that was fine.
    open.mockResolvedValue(body({ kind: "busy" }));
    show();

    await waitFor(() =>
      expect(screen.getByText(/checking this message/i)).toBeTruthy(),
    );
    expect(screen.queryByText(/nothing that can be shown/i)).toBeNull();
    expect(screen.getByRole("button", { name: /try again/i })).toBeTruthy();
  });

  it("asks again for a message that was too busy to read the first time", async () => {
    open.mockResolvedValueOnce(body({ kind: "busy" }));
    open.mockResolvedValue(body({ kind: "html" }));
    show();

    await screen.findByText(/checking this message/i);
    fireEvent.click(screen.getByRole("button", { name: /try again/i }));

    await waitFor(() =>
      expect(screen.getByTitle(/message body/i)).toBeTruthy(),
    );
  });

  it("asks before a link leaves this machine, and shows what it would open", async () => {
    // The frame cannot open anything: what following a link does is reach this application. The
    // question is where the address is finally shown, and where a person can say no.
    show();
    await screen.findByTitle(/message body/i);

    followed?.({
      name: "abc",
      host: "ζήτα.example",
      hostAsWritten: "xn--80ajf9b.example",
      url: "https://xn--80ajf9b.example/offer?who=you",
    });

    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toMatch(/ζήτα\.example/);
    // Both halves: a name that reads as one thing and is written as another is the attack. The
    // written half is looked for in its own sentence, because the address below carries it too.
    expect(dialog.textContent).toMatch(/written as xn--80ajf9b\.example/);
    expect(dialog.textContent).toMatch(/offer\?who=you/);
    // An address reads left to right in every language: laid out the other way, the box would open
    // on the end of a long one.
    expect(screen.getByText("ζήτα.example").getAttribute("dir")).toBe("ltr");
    // And it stands at the window's side of the dialog, beside everything else in it.
    expect(screen.getByText("ζήτα.example").className).toContain(
      "[text-align:match-parent]",
    );
    expect(screen.getByText(/offer\?who=you/).getAttribute("dir")).toBe("ltr");
    expect(follow).not.toHaveBeenCalled();
  });

  it("opens the link by its name, never by the address it showed", async () => {
    show();
    await screen.findByTitle(/message body/i);
    followed?.({
      name: "abc",
      host: "example.com",
      hostAsWritten: null,
      url: "https://example.com/a",
    });
    await screen.findByRole("dialog");

    fireEvent.click(screen.getByRole("button", { name: /^open$/i }));

    await waitFor(() => expect(follow).toHaveBeenCalledWith("abc"));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("opens nothing when the answer is no", async () => {
    show();
    await screen.findByTitle(/message body/i);
    followed?.({
      name: "abc",
      host: "example.com",
      hostAsWritten: null,
      url: "https://example.com/a",
    });
    await screen.findByRole("dialog");

    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));

    expect(follow).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("offers a way to look at a message that shows nothing", async () => {
    // A message that renders as almost nothing is often doing it on purpose. A blank pane with no
    // explanation is the one outcome the reader is not owed.
    open.mockResolvedValue(body({ kind: "nothing" }));
    show();

    await screen.findByText(/nothing that can be shown/i);

    expect(screen.getByRole("button", { name: /source/i })).toBeTruthy();
    expect(screen.getByRole("button", { name: /^text$/i })).toBeTruthy();
  });

  it("asks the core again when the reader looks another way", async () => {
    show("7");
    await screen.findByTitle(/message body/i);
    expect(askedFor("7", "sent", false)).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: /source/i }));

    await waitFor(() => expect(askedFor("7", "source", false)).toBe(true));
  });

  it("goes back to the message as sent when another one is opened", async () => {
    const { rerender } = render(
      <I18nProvider initial="en">
        <MessageBody messageId="7" />
      </I18nProvider>,
    );
    await screen.findByTitle(/message body/i);
    fireEvent.click(screen.getByRole("button", { name: /source/i }));
    await waitFor(() => expect(askedFor("7", "source", false)).toBe(true));

    rerender(
      <I18nProvider initial="en">
        <MessageBody messageId="8" />
      </I18nProvider>,
    );

    await waitFor(() => expect(askedFor("8", "sent", false)).toBe(true));
  });

  it("does not offer to try again for a picture that arrived and is not shown", async () => {
    // A drawing that can carry script is fetched and refused. Counting it with the ones that never
    // arrived would put a button in front of somebody that cannot succeed however often it is
    // pressed.
    open.mockResolvedValue(body({ remoteBlocked: 0, remoteNotShown: 2 }));
    show("7");
    await screen.findByTitle(/message body/i);

    expect(screen.getByText(/not shown here/i)).toBeTruthy();
    expect(
      screen.queryByRole("button", { name: /show pictures|try again/i }),
    ).toBeNull();
  });

  it("says both when one picture failed and another is not shown", async () => {
    open.mockResolvedValue(body({ remoteBlocked: 1, remoteNotShown: 1 }));
    show("7");
    await screen.findByTitle(/message body/i);

    expect(screen.getByText(/was not loaded/i)).toBeTruthy();
    expect(screen.getByText(/not shown here/i)).toBeTruthy();
    expect(screen.getByRole("button", { name: /show pictures/i })).toBeTruthy();
  });

  it("offers to fetch the pictures, and does not until it is asked", async () => {
    // Fetching one tells its host that this message was opened, by whom and when. Nothing is
    // fetched on the reader's behalf without the reader saying so.
    open.mockResolvedValue(body({ remoteBlocked: 3 }));
    show("7");
    await screen.findByTitle(/message body/i);

    // The first request is the one that matters: a later correction would be after the hosts had
    // already been told.
    expect(open.mock.calls[0]?.slice(0, 3)).toEqual(["7", "sent", false]);

    fireEvent.click(screen.getByRole("button", { name: /show pictures/i }));

    await waitFor(() => expect(askedFor("7", "sent", true)).toBe(true));
  });

  it("makes the press itself where somebody went and said to", async () => {
    // The setting is the whole of what makes this happen: nothing else in the reader may decide to
    // go out to a host on somebody's behalf.
    open.mockImplementation((_messageId, _form, pictures) =>
      Promise.resolve(body({ remoteBlocked: pictures ? 0 : 3 })),
    );
    show("7", "always");
    await screen.findByTitle(/message body/i);

    await waitFor(() => expect(askedFor("7", "sent", true)).toBe(true));
    // Nothing is missing any more, so there is nothing to say and nothing to press.
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: /pictures/i })).toBeNull(),
    );
  });

  it("puts the text on screen before it goes out for the pictures", async () => {
    // Otherwise the setting reads as a client that stopped working: the pictures are fetched while
    // the body is being built, six at a time with twenty seconds each, so a message pointing at one
    // host that will not answer would show nothing at all for twenty seconds.
    open.mockImplementation((_messageId, _form, pictures) =>
      Promise.resolve(body({ remoteBlocked: pictures ? 0 : 3 })),
    );
    show("7", "always");
    await screen.findByTitle(/message body/i);

    expect(open.mock.calls[0]?.slice(0, 3)).toEqual(["7", "sent", false]);
  });

  it("asks for nothing a second time where the message points at nothing", async () => {
    // A message with no pictures elsewhere is finished when its text arrives. Asking again would be
    // a second render of the same thing for no picture at all.
    open.mockImplementation(() => Promise.resolve(body({ remoteBlocked: 0 })));
    show("7", "always");
    await screen.findByTitle(/message body/i);

    await waitFor(() => expect(open).toHaveBeenCalledTimes(1));
    expect(askedFor("7", "sent", true)).toBe(false);
  });

  it("says what did not arrive even where nobody pressed anything", async () => {
    // The notice tells "the host said no" from "we never asked", and which of them it is was
    // decided by a press. With the answer standing there is no press, and the sentence must still
    // be the right one.
    open.mockImplementation((_messageId, _form, pictures) =>
      Promise.resolve(
        pictures
          ? body({ remoteBlocked: 0, remoteNotHad: 2 })
          : body({ remoteBlocked: 2 }),
      ),
    );
    show("7", "always");
    await screen.findByTitle(/message body/i);

    await waitFor(() =>
      expect(screen.getByText(/2 pictures did not arrive/i)).toBeTruthy(),
    );
    expect(screen.getByRole("button", { name: /try again/i })).toBeTruthy();
  });

  it("shows the pictures of the message already open when the answer changes", async () => {
    // Somebody who says "show them" is looking at a message while they say it, and a setting that
    // only reached the next message would read as one that did not work.
    open.mockImplementation((_messageId, _form, pictures) =>
      Promise.resolve(body({ remoteBlocked: pictures ? 0 : 1 })),
    );
    const { rerender } = render(
      <I18nProvider initial="en">
        <MessageBody messageId="7" showPictures="whenAsked" />
      </I18nProvider>,
    );
    await screen.findByTitle(/message body/i);
    expect(askedFor("7", "sent", true)).toBe(false);

    rerender(
      <I18nProvider initial="en">
        <MessageBody messageId="7" showPictures="always" />
      </I18nProvider>,
    );

    await waitFor(() => expect(askedFor("7", "sent", true)).toBe(true));
  });

  it("goes back to asking about the open message when the answer is taken back", async () => {
    open.mockImplementation((_messageId, _form, pictures) =>
      Promise.resolve(body({ remoteBlocked: pictures ? 0 : 1 })),
    );
    const { rerender } = render(
      <I18nProvider initial="en">
        <MessageBody messageId="7" showPictures="always" />
      </I18nProvider>,
    );
    await screen.findByTitle(/message body/i);

    rerender(
      <I18nProvider initial="en">
        <MessageBody messageId="7" showPictures="whenAsked" />
      </I18nProvider>,
    );

    await waitFor(() =>
      expect(screen.getByRole("button", { name: /show pictures/i })).toBeTruthy(),
    );
    expect(askedFor("7", "sent", false)).toBe(true);
  });

  it("asks again for every message, never once for all of them", async () => {
    open.mockResolvedValue(body({ remoteBlocked: 1 }));
    const { rerender } = render(
      <I18nProvider initial="en">
        <MessageBody messageId="7" />
      </I18nProvider>,
    );
    await screen.findByTitle(/message body/i);
    fireEvent.click(screen.getByRole("button", { name: /show pictures/i }));
    await waitFor(() => expect(askedFor("7", "sent", true)).toBe(true));

    rerender(
      <I18nProvider initial="en">
        <MessageBody messageId="8" />
      </I18nProvider>,
    );

    await waitFor(() => expect(askedFor("8", "sent", false)).toBe(true));
  });

  it("says the pictures did not arrive, rather than that they were fetched", async () => {
    // The failure this is about: a message that points at hosts nobody can reach was told its
    // pictures had been fetched, and the way to ask again disappeared.
    // Answered by what is asked rather than in turn. The body is fetched again whenever the way
    // of looking changes, and a queue of answers handed out one at a time goes out of step with
    // the questions the moment there is one more question than the test expected — which showed
    // up as this file failing about once in twenty runs, in whichever test happened to be next.
    open.mockImplementation(() =>
      Promise.resolve(body({ remoteBlocked: 3, remoteNotHad: 3 })),
    );
    show("7");
    await screen.findByTitle(/message body/i);

    fireEvent.click(screen.getByRole("button", { name: /show pictures/i }));

    await waitFor(() =>
      expect(screen.getByText(/3 pictures did not arrive/i)).toBeTruthy(),
    );
    expect(screen.getByRole("button", { name: /try again/i })).toBeTruthy();
  });

  it("says nothing at all once every picture is there", async () => {
    // Two before the pictures were asked for, none after. By what is asked rather than in turn,
    // for the reason above.
    open.mockImplementation((_messageId, _form, pictures) =>
      Promise.resolve(body({ remoteBlocked: pictures ? 0 : 2 })),
    );
    show("7");
    await screen.findByTitle(/message body/i);

    fireEvent.click(screen.getByRole("button", { name: /show pictures/i }));

    // Both in one wait. While the second body is on its way the whole strip is replaced by a line
    // saying so, and asserting one of them the moment the other disappears is asserting about the
    // waiting rather than about the answer.
    await waitFor(() => {
      expect(screen.queryByText(/could not be fetched/i)).toBeNull();
      expect(screen.queryByRole("button", { name: /pictures/i })).toBeNull();
    });
  });

  it("offers no second press for pictures it never asked about", async () => {
    // "The host said no" and "we never asked" are one sentence to a reader and two different
    // faults. Lumped together, the second sent somebody back to try again at a picture that is
    // not an address this build fetches -- which cannot succeed however many times it is pressed.
    open.mockImplementation(() =>
      Promise.resolve(body({ remoteBlocked: 2, remoteNotHad: 0 })),
    );
    show("7");
    await screen.findByTitle(/message body/i);

    fireEvent.click(screen.getByRole("button", { name: /show pictures/i }));

    await waitFor(() =>
      expect(
        screen.getByText(/2 pictures are not where the message says they are/i),
      ).toBeTruthy(),
    );
    expect(screen.queryByRole("button", { name: /try again/i })).toBeNull();
  });

  it("asks again when the reader asks again", async () => {
    open.mockResolvedValue(body({ remoteBlocked: 1, remoteNotHad: 1 }));
    show("7");
    await screen.findByTitle(/message body/i);
    fireEvent.click(screen.getByRole("button", { name: /show pictures/i }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /try again/i })).toBeTruthy(),
    );

    const before = open.mock.calls.length;
    fireEvent.click(screen.getByRole("button", { name: /try again/i }));

    await waitFor(() => expect(open.mock.calls.length).toBeGreaterThan(before));
  });

  it("forgets the body when the reader moves on", async () => {
    const { unmount } = show();
    await screen.findByTitle(/message body/i);

    unmount();

    // Awaited: the closing waits for the asking to settle, so that leaving before the answer
    // arrives closes the body too.
    await waitFor(() => expect(close).toHaveBeenCalledWith("deadbeef"));
  });

  it("explains a failure rather than showing an empty pane", async () => {
    open.mockRejectedValue({
      code: "offline",
      message: "core said so",
      path: null,
    });
    show();

    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toMatch(
        /could not be reached/i,
      ),
    );
  });

  function askAboutALink() {
    followed?.({
      name: "abc",
      host: "example.com",
      hostAsWritten: null,
      url: "https://example.com/a",
    });
  }

  it("puts the focus on refusing, not on opening", async () => {
    // This panel appears because somebody clicked a link inside a message, and a message is
    // hostile input. A stray Enter must decline. Focus stayed wherever it was before, so the
    // panel was a wall a keyboard could not answer at all.
    show();
    await screen.findByTitle(/message body/i);
    askAboutALink();
    await screen.findByRole("dialog");

    await waitFor(() =>
      expect(document.activeElement).toBe(
        screen.getByRole("button", { name: /cancel/i }),
      ),
    );
  });

  it("declines on Escape and opens nothing", async () => {
    show();
    await screen.findByTitle(/message body/i);
    askAboutALink();
    await screen.findByRole("dialog");

    fireEvent.keyDown(screen.getByRole("dialog"), {
      key: "Escape",
      bubbles: true,
    });

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(follow).not.toHaveBeenCalled();
  });
});

describe("MessageBody, the files a message came with", () => {
  const WITH_A_FILE = {
    at: 2,
    name: "invoice.pdf",
    bytes: "2500000",
  };

  function showWith(attachments: (typeof WITH_A_FILE)[]) {
    open.mockResolvedValue({
      name: "body-1",
      url: "lyonoyl-body://body-1",
      kind: "html",
      remoteBlocked: 0,
      remoteNotShown: 0,
      remoteNotHad: 0,
      truncated: false,
      attachments,
    });
  }

  it("lists what came with the message, by name, kind and size", async () => {
    showWith([WITH_A_FILE]);

    show();

    expect(await screen.findByText(nameShown("invoice.pdf"))).toBeTruthy();
    expect(screen.getByText("PDF · 2.5 MB")).toBeTruthy();
  });

  it("lists both where a message came with two", async () => {
    // A message carrying one file is what every other case here uses, and it says nothing about
    // the second: a list that stopped after the first would pass all of them.
    showWith([
      WITH_A_FILE,
      { at: 3, name: "photo.jpg", bytes: "840000" },
    ]);

    show();

    expect(await screen.findByText(nameShown("invoice.pdf"))).toBeTruthy();
    expect(screen.getByText("PDF · 2.5 MB")).toBeTruthy();
    expect(screen.getByText(nameShown("photo.jpg"))).toBeTruthy();
    expect(screen.getByText("JPG · 840 kB")).toBeTruthy();
    expect(
      screen.getAllByRole("button", { name: /^save /i }),
    ).toHaveLength(2);
  });

  it("says only the size where the name ends in no kind anybody reads", async () => {
    // No extension, and one that is not a word, and nothing stands in for either. The type the
    // message declared never reaches the window: it is the sender's claim, and the name is what
    // the system goes by.
    showWith([
      { at: 2, name: "thing", bytes: "1024" },
      { at: 3, name: "Minutes v2. final draft", bytes: "2048" },
    ]);

    show();

    expect(await screen.findByText(nameShown("thing"))).toBeTruthy();
    expect(screen.getByText("1 kB")).toBeTruthy();
    expect(screen.getByText(nameShown("Minutes v2. final draft"))).toBeTruthy();
    expect(screen.getByText("2 kB")).toBeTruthy();
  });

  it("names the kind its name ends in, never the one the message declared", async () => {
    // A message can call an application a PDF, in the type it declares, which never reaches the
    // window, and earlier in the name, which does. The end of the name is what the system opens it
    // by, so the row says what the end says, and the claim appears nowhere.
    showWith([{ at: 2, name: "Statement.pdf.app", bytes: "2500000" }]);

    show();

    expect(await screen.findByText(nameShown("Statement.pdf.app"))).toBeTruthy();
    expect(screen.getByText("APP · 2.5 MB")).toBeTruthy();
    expect(screen.queryByText(/^PDF/)).toBeNull();
  });

  it("keeps how a long name ends on screen, cutting it in the middle", async () => {
    // Cut at its end, a long name loses its extension off screen, and the extension is what decides
    // what the file does when it is opened. The start gives way; the ending does not.
    const name = "Statement-for-your-account-ending-0000-and-more-words.pdf.app";
    showWith([{ at: 2, name, bytes: "2500000" }]);

    show();

    const shown = await screen.findByText(nameShown(name));
    const [start, ending] = Array.from(shown.children);
    expect(ending?.textContent).toBe(".app");
    expect(ending?.className).toContain("shrink-0");
    expect(ending?.className).not.toMatch(/truncate|ellipsis|overflow/);
    expect(start?.textContent).toBe(
      "Statement-for-your-account-ending-0000-and-more-words.pdf",
    );
    expect(start?.className).toContain("text-ellipsis");
    // And the whole of it is what the button is called, for whoever hears it rather than sees it.
    expect(
      screen.getByRole("button", { name: (called) => called.includes(name) }),
    ).toBeTruthy();
  });

  it("keeps only the last characters of an ending longer than any extension", async () => {
    // Kept whole, an ending of any length would take the whole room, and how the name starts would
    // be cut down to nothing.
    const name = `notes.${"x".repeat(40)}`;
    showWith([{ at: 2, name, bytes: "1024" }]);

    show();

    const shown = await screen.findByText(nameShown(name));
    const [start, ending] = Array.from(shown.children);
    expect(ending?.textContent).toBe("x".repeat(20));
    expect(start?.textContent).toBe(`notes.${"x".repeat(20)}`);
    expect(screen.getByText("1 kB")).toBeTruthy();
  });

  it("keeps an ending too wide for the room inside it, off the size beside it", async () => {
    // A dot and nineteen ideographs are within the characters an ending keeps whole, and wider
    // than the room a name has. Unclipped, they are drawn over the size beside them. The room
    // clips what does not fit it; the ending itself is still never cut short.
    const wide = "\u5831".repeat(19);
    const name = `notes.${wide}`;
    showWith([{ at: 2, name, bytes: "1024" }]);

    show();

    const shown = await screen.findByText(nameShown(name));
    expect(shown.lastElementChild?.textContent).toBe(`.${wide}`);
    expect(shown.className).toMatch(/\bmax-w-56\b/);
    expect(shown.className).toMatch(/\boverflow-hidden\b/);
    expect(screen.getByText("1 kB")).toBeTruthy();
  });

  it("keeps the ending at the right of a name that starts right to left", async () => {
    // Laid out in the name's own direction, one Hebrew letter at the front turns the row around:
    // the ending goes to the far left, drawn `app.`, and the last thing read is the `.pdf` before
    // it. The row runs left to right whatever the name; only the start keeps its own direction.
    const start = "\u05D0Statement-for-your-account-ending-0000.pdf";
    const name = `${start}.app`;
    showWith([{ at: 2, name, bytes: "2500000" }]);

    show();

    const button = await screen.findByRole("button", {
      name: (called) => called.includes(name),
    });
    const shown = button.firstElementChild;
    expect(shown?.textContent).toBe(name);
    expect(shown?.getAttribute("dir")).toBe("ltr");
    expect(shown?.lastElementChild?.textContent).toBe(".app");
    expect(
      shown?.lastElementChild?.closest("[dir]")?.getAttribute("dir"),
    ).toBe("ltr");
    expect(shown?.firstElementChild?.textContent).toBe(start);
    expect(shown?.firstElementChild?.getAttribute("dir")).toBe("auto");
  });

  it("asks for it by where it sits, never by its name", async () => {
    // Two files may share a name and a message is free to give none. And a name reaching the core
    // as an identifier is a name an attacker chose being used to find something.
    showWith([WITH_A_FILE]);
    saving.mockResolvedValue({ name: "invoice.pdf" });

    show();
    fireEvent.click(
      await screen.findByRole("button", { name: /save invoice\.pdf/i }),
    );

    await waitFor(() => expect(saving).toHaveBeenCalledWith("7", 2));
  });

  it("shows nothing where a message came with nothing", async () => {
    showWith([]);

    show();
    await screen.findByTitle(/message body/i);

    expect(screen.queryByText(/^Files$/)).toBeNull();
  });

  it("says the words in the language that was chosen", async () => {
    // Asserting the English words proves nothing: a hard-coded English string shows exactly the
    // same. What tells the two apart is another language, so the same row is drawn twice and the
    // words have to differ — neither of them written down here, because a test that repeats the
    // catalogue is a second copy of it.
    showWith([WITH_A_FILE]);

    async function drawnIn(language: "en" | "ru") {
      const drawn = render(
        <I18nProvider initial={language}>
          <MessageBody messageId="7" />
        </I18nProvider>,
      );
      // The file's own name is in both, which is how the row is found without knowing either.
      const save = await drawn.findByRole("button", { name: /invoice\.pdf/ });
      const words = {
        heading:
          save.closest("ul")?.previousElementSibling?.textContent ?? "",
        save: save.getAttribute("aria-label") ?? "",
      };
      drawn.unmount();
      return words;
    }

    const english = await drawnIn("en");
    const russian = await drawnIn("ru");

    expect(english.heading).toBeTruthy();
    expect(russian.heading).not.toBe(english.heading);
    expect(russian.save).toContain("invoice.pdf");
    expect(russian.save).not.toBe(english.save);
  });
});

describe("MessageBody, asking for the pictures a message points at", () => {
  // Its own, because the one above belongs to another `describe` and does not run here: without
  // it these count every call the file made before them.
  beforeEach(() => {
    vi.clearAllMocks();
    wearing.mockReturnValue(null);
    close.mockResolvedValue(undefined);
    onLink.mockImplementation(async () => () => {});
  });

  it("asks once, and puts the frame on the body that came back", async () => {
    // The whole of what pressing it does: one more request, saying yes this time, and the frame
    // pointed at what that request answered with. A second request would render the message again
    // and fetch every picture it points at a second time.
    open.mockResolvedValueOnce(
      body({
        name: "first",
        url: "lyomsg://localhost/first",
        remoteBlocked: 2,
      }),
    );
    open.mockResolvedValueOnce(
      body({ name: "second", url: "lyomsg://localhost/second" }),
    );

    show();
    await waitFor(() => expect(open).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: /show pictures/i }));

    await waitFor(() => expect(askedFor("7", "sent", true)).toBe(true));
    await waitFor(() =>
      expect(document.querySelector("iframe")?.getAttribute("src")).toBe(
        "lyomsg://localhost/second",
      ),
    );

    expect(
      open.mock.calls.filter((call) => call[2] === true),
      "asked for the pictures more than once",
    ).toHaveLength(1);
    // And the one it left is the one it closed, not the one it is showing.
    expect(close).toHaveBeenCalledWith("first");
    expect(close).not.toHaveBeenCalledWith("second");
  });

  it("closes a body whose name arrived after the reader had left", async () => {
    // Leaving before the answer is the ordinary case, not the rare one: a development build mounts
    // twice, and the colours settle a frame after the message opens. Closing only what was already
    // named left every one of those held until something else pushed it out -- which is a message
    // losing its pictures while somebody is reading it.
    let answer: (dto: MessageBodyDto) => void = () => {};
    open.mockImplementationOnce(
      () =>
        new Promise<MessageBodyDto>((settle) => {
          answer = settle;
        }),
    );

    const shown = show();
    shown.unmount();

    answer(body({ name: "held-and-left" }));
    await waitFor(() => expect(close).toHaveBeenCalledWith("held-and-left"));
  });

  it("says nothing about a body from before the press until that body is here", async () => {
    // Between pressing and the answer, the flag has changed and the body has not. Read together
    // they said "these pictures are not where the message says they are" about pictures nobody had
    // asked for yet -- and offered no press, because the press is only offered for the ones that
    // were asked for and did not arrive.
    open.mockResolvedValue(body({ remoteBlocked: 5 }));

    show();
    await screen.findByText(/5 pictures were not loaded/i);

    let answer: (dto: MessageBodyDto) => void = () => {};
    open.mockImplementationOnce(
      () =>
        new Promise<MessageBodyDto>((settle) => {
          answer = settle;
        }),
    );

    fireEvent.click(screen.getByRole("button", { name: /show pictures/i }));

    // The answer is not here yet, and what is on screen is still the body from before it.
    expect(screen.queryByText(/not where the message says/i)).toBeNull();
    expect(screen.getByText(/5 pictures were not loaded/i)).not.toBeNull();

    answer(body({ remoteNotHad: 1 }));
    await screen.findByText(/one picture did not arrive/i);
  });

  it("says the pictures are being fetched while they are", async () => {
    // A picture is given twenty seconds to arrive. Without this the press changed nothing on
    // screen for that whole time -- and, because the words come from the body rather than from the
    // press, the button went on offering the press that had already been made.
    open.mockResolvedValue(body({ remoteBlocked: 2 }));
    show();
    await screen.findByText(/2 pictures were not loaded/i);

    let answer: (dto: MessageBodyDto) => void = () => {};
    open.mockImplementationOnce(
      () =>
        new Promise<MessageBodyDto>((settle) => {
          answer = settle;
        }),
    );

    fireEvent.click(screen.getByRole("button", { name: /show pictures/i }));

    const waiting = await screen.findByRole("button", {
      name: /fetching pictures/i,
    });
    expect(waiting).toHaveProperty("disabled", true);
    expect(screen.queryByRole("button", { name: /show pictures/i })).toBeNull();

    answer(body({ remoteNotHad: 1 }));
    await screen.findByRole("button", { name: /try again/i });
  });

  it("does not let go of the body the frame is still fetching pictures from", async () => {
    // The page reaches the frame long before the pictures do. Letting go of a body when the effect
    // that opened it re-ran -- a press, the colours settling, a development build mounting twice --
    // answered every one of those pictures with nothing, and left a message whose text is there and
    // whose pictures are empty frames.
    open.mockResolvedValue(body({ name: "on-screen", remoteBlocked: 2 }));
    show();
    await screen.findByText(/2 pictures were not loaded/i);

    let answer: (dto: MessageBodyDto) => void = () => {};
    open.mockImplementationOnce(
      () =>
        new Promise<MessageBodyDto>((settle) => {
          answer = settle;
        }),
    );

    fireEvent.click(screen.getByRole("button", { name: /show pictures/i }));
    await screen.findByRole("button", { name: /fetching pictures/i });

    expect(
      close.mock.calls.some((call) => call[0] === "on-screen"),
      "the body the frame is showing was let go while it was showing",
    ).toBe(false);

    answer(body({ name: "its-replacement", remoteNotHad: 1 }));
    await screen.findByRole("button", { name: /try again/i });
    await waitFor(() => expect(close).toHaveBeenCalledWith("on-screen"));
  });

  it("says so when the frame came for a picture and it was not there", async () => {
    // The counts are worked out when the message is opened and the frame comes for the pictures
    // after that, so a picture can be counted as arrived and be gone by the time it is wanted.
    // Without this the message rendered with holes in it and told the reader everything was well.
    show();
    await screen.findByTitle(/message body/i);
    expect(screen.queryByText(/were not shown/i)).toBeNull();

    act(() => cameUpEmpty?.());

    await screen.findByText(/some pictures of this message were not shown/i);
    expect(
      screen.getByRole("button", { name: /try again/i }),
      "nothing was offered to put it right",
    ).not.toBeNull();
  });
});
