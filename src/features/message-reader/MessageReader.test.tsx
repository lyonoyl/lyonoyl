import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import type { AccountNodeDto } from "@/shared/ipc/generated/AccountNodeDto";
import type { ForwardDto } from "@/shared/ipc/generated/ForwardDto";
import type { MailboxTreeDto } from "@/shared/ipc/generated/MailboxTreeDto";
import type { MessageDetailsDto } from "@/shared/ipc/generated/MessageDetailsDto";
import { MessageReader } from "./MessageReader";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    getMessage: vi.fn(),
    openMessageBody: vi.fn(),
    closeMessageBody: vi.fn(),
    saveMessage: vi.fn(),
    answerAudience: vi.fn(),
    forwardMessage: vi.fn(),
    forgetAttachment: vi.fn(),
    archiveMessage: vi.fn(),
    deleteMessage: vi.fn(),
    moveMessage: vi.fn(),
    getMailboxTree: vi.fn(),
    undoAction: vi.fn(),
  };
});

const {
  getMessage,
  openMessageBody,
  closeMessageBody,
  saveMessage,
  answerAudience,
  forwardMessage,
  forgetAttachment,
  archiveMessage,
  deleteMessage,
  moveMessage,
  getMailboxTree,
  undoAction,
} = await import("@/shared/ipc/client");
const message = vi.mocked(getMessage);
const bodyOf = vi.mocked(openMessageBody);
const forget = vi.mocked(closeMessageBody);
const save = vi.mocked(saveMessage);
const asking = vi.mocked(answerAudience);
const forwarding = vi.mocked(forwardMessage);
const lettingGo = vi.mocked(forgetAttachment);
const archiving = vi.mocked(archiveMessage);
const moving = vi.mocked(moveMessage);
const tree = vi.mocked(getMailboxTree);

/** The sidebar as the core answers it, read before anything was asked for. */
function sidebar(accounts: AccountNodeDto[]): MailboxTreeDto {
  return { accounts, askedThrough: 0 };
}
const deleting = vi.mocked(deleteMessage);
const undoing = vi.mocked(undoAction);

const DETAILS: MessageDetailsDto = {
  messageId: "42",
  subject: "Quarterly report",
  sender: "Zeta Quorin",
  senderAddress: "zeta@example.com",
  receivedAt: 1_700_000_000,
  size: 4096,
  unread: true,
  bodyState: "headersOnly",
  accountId: "acc-1",
  addressedToMore: true,
  named: "quarterly@example.com",
  chain: ["kickoff@example.com", "agenda@example.com"],
  placements: [
    { folderId: "1", folderName: "INBOX", folderRole: "inbox" },
    { folderId: "7", folderName: "Projects 2026", folderRole: "custom" },
  ],
};

function renderReader(messageId: string | null, language = "en") {
  const onReply = vi.fn();
  render(
    <I18nProvider initial={language}>
      <MessageReader messageId={messageId} folderId="1" onReply={onReply} />
    </I18nProvider>,
  );
  return { onReply };
}

beforeEach(() => {
  bodyOf.mockResolvedValue({
    name: "deadbeef",
    url: "lyomsg://localhost/deadbeef",
    kind: "html",
    remoteBlocked: 0,
    remoteNotShown: 0,
    remoteNotHad: 0,
    truncated: false,
    attachments: [],
  });
  forget.mockResolvedValue(undefined);
  message.mockReset();
  save.mockReset();
  save.mockResolvedValue({ name: "Quarterly report.eml" });
  asking.mockReset();
  asking.mockResolvedValue({ to: ["zeta@example.com"], cc: [], quoted: "" });
  forwarding.mockReset();
  forwarding.mockResolvedValue({
    to: ["kappa@example.net"],
    cc: [],
    text: "The figures for the quarter.",
    files: [{ token: "t1", name: "report.pdf", bytes: "2048" }],
    leftBehind: [],
  });
  lettingGo.mockReset();
  lettingGo.mockResolvedValue(undefined);
  archiving.mockResolvedValue(undefined);
  // Reset rather than left standing: a call another test made is not one this test caused.
  deleting.mockReset();
  deleting.mockResolvedValue(undefined);
  moving.mockReset();
  moving.mockResolvedValue(undefined);
  tree.mockReset();
  tree.mockResolvedValue(
    sidebar([
      {
        place: 0,
        accountId: "acc-1",
        email: "someone@example.com",
        displayName: "Someone",
        collapsed: false,
        answered: true,
        unread: 0,
        folders: [
          {
            folderId: "1",
            name: "INBOX",
            role: "inbox",
            total: 1,
            unread: 0,
            draftsHere: 0,
            canReceive: true,
          },
          {
            folderId: "8",
            name: "Work",
            role: "custom",
            total: 0,
            unread: 0,
            draftsHere: 0,
            canReceive: true,
          },
          {
            folderId: "9",
            name: "Sent",
            role: "sent",
            total: 0,
            unread: 0,
            draftsHere: 0,
            canReceive: false,
          },
        ],
      },
    ]),
  );
  undoing.mockResolvedValue(true);
});

describe("MessageReader", () => {
  it("files in the folder it is being read in, not the first one the message sits in", async () => {
    // A message can be in an inbox and in a folder somebody made at the same time. Read off the
    // message, the answer was whichever placement the index listed first — and those are ordered by
    // role, which puts a folder somebody made before the inbox. Archiving from the reader then took
    // the message out of a folder nobody was looking at.
    message.mockResolvedValue({
      ...DETAILS,
      placements: [
        { folderId: "7", folderName: "Projects 2026", folderRole: "custom" },
        { folderId: "1", folderName: "INBOX", folderRole: "inbox" },
      ],
    });

    render(
      <I18nProvider initial="en">
        <MessageReader messageId="42" folderId="1" />
      </I18nProvider>,
    );
    await screen.findByText("Quarterly report");

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    await waitFor(() => expect(archiving).toHaveBeenCalledWith("42", "1"));
  });
  it("asks for nothing until a message is opened", () => {
    renderReader(null);

    expect(screen.getByText("Select a message to read it")).toBeTruthy();
    expect(message).not.toHaveBeenCalled();
  });

  it("shows what the index holds about the message", async () => {
    message.mockResolvedValue(DETAILS);
    renderReader("42");

    expect(await screen.findByText("Quarterly report")).toBeTruthy();
    // The name the message gave, which is what a person reads. The address behind it is what an
    // answer is sent to, and it is carried rather than shown.
    expect(screen.getByText("Zeta Quorin")).toBeTruthy();
    // 4096 bytes, said in powers of a thousand like every other size in this application and like
    // the operating system beside the same file. The reader used to answer this one question its
    // own way, in powers of 1024.
    expect(screen.getByText("4.1 kB")).toBeTruthy();
  });

  it("names the folders the message sits in, translating the ones with a role", async () => {
    message.mockResolvedValue(DETAILS);
    renderReader("42");

    // One message, two folders: the inbox is named in the reader's language, the folder the user
    // made keeps its own name.
    expect(await screen.findByText("Inbox and Projects 2026")).toBeTruthy();
  });

  it("keeps a folder's own name in its own direction in a window that runs right to left", async () => {
    // `Projects 2026` inside an Arabic list keeps its order, and the list is joined around the names
    // rather than through them.
    message.mockResolvedValue(DETAILS);
    renderReader("42", "ar");

    const folders = await screen.findByText(
      (_, element) =>
        element?.tagName === "DD" &&
        (element.textContent ?? "").includes("Projects 2026"),
    );
    expect(folders.textContent).toContain("\u2068Projects 2026\u2069");
  });

  it("puts the body in a frame of its own rather than in this window", async () => {
    // The reader shows what the index holds and hands the body to a frame that is allowed
    // nothing. What it must never do is render a message in the window that holds IPC.
    message.mockResolvedValue(DETAILS);
    renderReader("42");

    await screen.findByText("Inbox and Projects 2026");

    const frame = await screen.findByTitle(/message body/i);
    expect(frame.getAttribute("sandbox")).toBe("");
  });

  it("says so when the message is no longer in the index", async () => {
    message.mockResolvedValue(null);
    renderReader("42");

    expect(
      await screen.findByText("This message is no longer in the local index"),
    ).toBeTruthy();
  });

  it("explains a failure in the interface language", async () => {
    message.mockRejectedValue({
      code: "storageUnavailable",
      message: "",
      path: null,
    });
    renderReader("42", "ru");

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(
      screen.getByText("Не удалось открыть локальную базу данных"),
    ).toBeTruthy();
  });

  it("reads the message again when another one is opened", async () => {
    message.mockResolvedValue(DETAILS);
    const { rerender } = render(
      <I18nProvider initial="en">
        <MessageReader messageId="42" folderId="1" />
      </I18nProvider>,
    );
    await screen.findByText("Quarterly report");

    message.mockResolvedValue({
      ...DETAILS,
      messageId: "43",
      subject: "Another one",
    });
    rerender(
      <I18nProvider initial="en">
        <MessageReader messageId="43" folderId="1" />
      </I18nProvider>,
    );

    expect(await screen.findByText("Another one")).toBeTruthy();
    expect(message).toHaveBeenLastCalledWith("43");
  });

  it("asks nowhere it could go until somebody opens the menu", async () => {
    message.mockResolvedValue(DETAILS);
    renderReader("42");
    await screen.findByRole("heading", { level: 1 });

    expect(tree).not.toHaveBeenCalled();
  });

  it("offers the folders that take messages, and not the one it is in", async () => {
    message.mockResolvedValue(DETAILS);
    renderReader("42");
    await screen.findByRole("heading", { level: 1 });

    fireEvent.pointerDown(screen.getByRole("button", { name: "Move to…" }), {
      button: 0,
      ctrlKey: false,
    });

    const work = await screen.findByRole("menuitem", { name: "Work" });
    // The server's own name keeps its own direction, as it does in the tree.
    expect(within(work).getByText("Work").getAttribute("dir")).toBe("auto");
    // The folder it is already in, and the one an account writes into rather than files into.
    expect(screen.queryByRole("menuitem", { name: "Inbox" })).toBeNull();
    expect(screen.queryByRole("menuitem", { name: "Sent" })).toBeNull();
  });

  it("files the message in the folder that was chosen", async () => {
    const onFiled = vi.fn();
    message.mockResolvedValue(DETAILS);
    render(
      <I18nProvider initial="en">
        <MessageReader messageId="42" folderId="1" onFiled={onFiled} />
      </I18nProvider>,
    );
    await screen.findByRole("heading", { level: 1 });

    fireEvent.pointerDown(screen.getByRole("button", { name: "Move to…" }), {
      button: 0,
      ctrlKey: false,
    });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Work" }));

    await waitFor(() => expect(moving).toHaveBeenCalledWith("42", "1", "8"));
    await waitFor(() =>
      expect(onFiled).toHaveBeenCalledWith({
        messageId: "42",
        folderId: "1",
        what: "moved",
      }),
    );
  });

  it("files a message where the account keeps finished mail, in the folder it is read in", async () => {
    // Which folder the archive is, is the core's business; what this sends is where the message is
    // now.
    message.mockResolvedValue(DETAILS);
    renderReader("42");
    await screen.findByText("Quarterly report");

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    expect(archiving).toHaveBeenCalledWith("42", "1");
  });

  it("throws a message away in the sense that can be taken back", async () => {
    // Not `permanently`: a person pressing the bin means the trash, and the difference is whether
    // they can have it back.
    message.mockResolvedValue(DETAILS);
    renderReader("42");
    await screen.findByText("Quarterly report");

    fireEvent.click(screen.getByRole("button", { name: "Move to trash" }));

    expect(deleting).toHaveBeenCalledWith("42", "1");
  });

  it("says so in the reader's language when there is nowhere to file it", async () => {
    archiving.mockRejectedValue({
      code: "folderCannotReceive",
      message: "This account has no archive folder.",
      path: null,
    });
    message.mockResolvedValue(DETAILS);
    renderReader("42", "ru");
    await screen.findByText("Quarterly report");

    fireEvent.click(screen.getByRole("button", { name: "В архив" }));

    expect(await screen.findByRole("alert")).toBeTruthy();
  });

  it("hands the filing to the workspace rather than saying anything about it here", async () => {
    // The line that says what happened lives under the list: by the time anybody reads it, this
    // pane has moved on to the next message and would be talking about mail nobody can see.
    message.mockResolvedValue(DETAILS);
    const filed = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageReader messageId="42" folderId="1" onFiled={filed} />
      </I18nProvider>,
    );
    await screen.findByText("Quarterly report");

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    await waitFor(() =>
      expect(filed).toHaveBeenCalledWith({
        messageId: "42",
        folderId: "1",
        what: "archived",
      }),
    );
    expect(screen.queryByText("Moved to the archive")).toBeNull();
    expect(screen.queryByRole("button", { name: "Undo" })).toBeNull();
  });

  it("files once however fast the button is clicked twice", async () => {
    // The buttons keep their place while the message under them changes, and it changes within a
    // frame of the first click. The second click would otherwise file a message nobody has seen.
    message.mockResolvedValue(DETAILS);
    const filed = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageReader messageId="42" folderId="1" onFiled={filed} />
      </I18nProvider>,
    );
    await screen.findByText("Quarterly report");

    const archive = screen.getByRole("button", { name: "Archive" });
    archiving.mockClear();
    fireEvent.click(archive);
    fireEvent.click(archive);

    await waitFor(() => expect(filed).toHaveBeenCalled());
    expect(archiving).toHaveBeenCalledTimes(1);
  });

  it("offers saving as an icon that still answers to a name", async () => {
    // An icon with no name is a button nobody using a screen reader can find, and a hint that
    // only appears on hover is no name at all. The word stays, as the button's own name.
    message.mockResolvedValue(DETAILS);
    renderReader("42");
    await screen.findByText("Quarterly report");

    const button = screen.getByRole("button", { name: "Save as file" });

    expect(button.textContent).toBe("");
    expect(button.querySelector("svg")).toBeTruthy();
  });

  it("asks to save by naming the message and nothing else", async () => {
    // A path from here is the one thing that must never happen: whatever gets a foothold in this
    // window would then be able to write wherever the person can.
    message.mockResolvedValue(DETAILS);
    renderReader("42");
    await screen.findByText("Quarterly report");

    fireEvent.click(screen.getByRole("button", { name: "Save as file" }));

    expect(save).toHaveBeenCalledWith("42");
    expect(save.mock.calls[0]).toHaveLength(1);
  });

  it("says what the file ended up being called", async () => {
    message.mockResolvedValue(DETAILS);
    renderReader("42");
    await screen.findByText("Quarterly report");

    fireEvent.click(screen.getByRole("button", { name: "Save as file" }));

    expect(
      await screen.findByText("Saved as Quarterly report.eml"),
    ).toBeTruthy();
  });

  it("says nothing when the person closes the panel", async () => {
    // Deciding not to save is an answer. Reporting it back would be telling somebody what they
    // just did.
    message.mockResolvedValue(DETAILS);
    save.mockResolvedValue({ name: null });
    renderReader("42");
    await screen.findByText("Quarterly report");

    fireEvent.click(screen.getByRole("button", { name: "Save as file" }));

    await screen.findByRole("button", { name: "Save as file" });
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByText(/Saved as/)).toBeNull();
  });

  it("shows a failure in the reader's language rather than the core's", async () => {
    message.mockResolvedValue(DETAILS);
    save.mockRejectedValue({
      code: "fileNotWritten",
      message: "The file could not be written where it was asked to go.",
      path: null,
    });
    renderReader("42", "ru");
    await screen.findByText("Quarterly report");

    fireEvent.click(screen.getByRole("button", { name: "Сохранить в файл" }));

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(
      screen.getByText("Файл не удалось сохранить в выбранное место"),
    ).toBeTruthy();
  });

  it("forgets what was saved when another message is opened", async () => {
    // The line says something about the message that was open, and this is a different message.
    message.mockResolvedValue(DETAILS);
    const { rerender } = render(
      <I18nProvider initial="en">
        <MessageReader messageId="42" folderId="1" />
      </I18nProvider>,
    );
    await screen.findByText("Quarterly report");
    fireEvent.click(screen.getByRole("button", { name: "Save as file" }));
    await screen.findByText("Saved as Quarterly report.eml");

    message.mockResolvedValue({
      ...DETAILS,
      messageId: "43",
      subject: "Another one",
    });
    rerender(
      <I18nProvider initial="en">
        <MessageReader messageId="43" folderId="1" />
      </I18nProvider>,
    );

    await screen.findByText("Another one");
    expect(screen.queryByText(/Saved as/)).toBeNull();
  });

  it("answers the message with everything an answer needs", async () => {
    message.mockResolvedValue(DETAILS);
    const { onReply } = renderReader("42");
    await screen.findByText("Quarterly report");

    fireEvent.click(screen.getByRole("button", { name: "Reply" }));

    // Who it goes to is asked for, not worked out here: taking the person's own addresses out of
    // an answer is one rule and it lives in the domain.
    await waitFor(() => expect(asking).toHaveBeenCalledWith("42", false));
    await waitFor(() =>
      expect(onReply).toHaveBeenCalledWith({
        to: "zeta@example.com",
        cc: "",
        subject: "Re: Quarterly report",
        answering: "quarterly@example.com",
        chain: ["kickoff@example.com", "agenda@example.com"],
        fromAccount: "acc-1",
        body: "",
      }),
    );
  });

  it("answers everybody as a different question, not the same one", async () => {
    asking.mockResolvedValue({
      to: ["zeta@example.com", "kappa@example.net"],
      cc: ["sigma@example.org"],
      quoted: "",
    });
    message.mockResolvedValue(DETAILS);
    const { onReply } = renderReader("42");
    await screen.findByText("Quarterly report");

    fireEvent.click(screen.getByRole("button", { name: "Reply to everybody" }));

    await waitFor(() => expect(asking).toHaveBeenCalledWith("42", true));
    await waitFor(() =>
      expect(onReply).toHaveBeenCalledWith(
        expect.objectContaining({
          to: "zeta@example.com, kappa@example.net",
          cc: "sigma@example.org",
        }),
      ),
    );
  });

  it("does not offer answering everybody on a message addressed to one person", async () => {
    // The two answers would be the same message, and offering both invites the one that adds
    // nothing.
    message.mockResolvedValue({ ...DETAILS, addressedToMore: false });
    renderReader("42");
    await screen.findByText("Quarterly report");

    expect(screen.getByRole("button", { name: "Reply" })).toBeTruthy();
    expect(
      screen.queryByRole("button", { name: "Reply to everybody" }),
    ).toBeNull();
  });

  it("says so when it cannot work out who the answer goes to", async () => {
    // Pressing a button and getting nothing is the failure a person cannot act on.
    asking.mockRejectedValue({
      code: "storageUnavailable",
      message: "The local index is unavailable.",
      path: null,
    });
    message.mockResolvedValue(DETAILS);
    const { onReply } = renderReader("42");
    await screen.findByText("Quarterly report");

    fireEvent.click(screen.getByRole("button", { name: "Reply" }));

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(onReply).not.toHaveBeenCalled();
  });

  it("asks once however fast the button is clicked twice", async () => {
    message.mockResolvedValue(DETAILS);
    renderReader("42");
    await screen.findByText("Quarterly report");

    const reply = screen.getByRole("button", { name: "Reply" });
    fireEvent.click(reply);
    fireEvent.click(reply);

    await waitFor(() => expect(asking).toHaveBeenCalled());
    expect(asking).toHaveBeenCalledTimes(1);
  });

  it("offers answering as an icon that still answers to a name", async () => {
    message.mockResolvedValue(DETAILS);
    renderReader("42", "ru");
    await screen.findByText("Quarterly report");

    // Not the key: a button labelled `reading.reply` is what a missing translation looks like on
    // the screen, and it reads that way to a screen reader too.
    expect(screen.getByRole("button", { name: "Ответить" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Ответить всем" })).toBeTruthy();
  });

  it("says nothing to a window that is not listening for an answer", async () => {
    message.mockResolvedValue(DETAILS);
    render(
      <I18nProvider initial="en">
        <MessageReader messageId="42" folderId="1" />
      </I18nProvider>,
    );
    await screen.findByText("Quarterly report");

    // The button is there whether or not anybody wants to hear about it, and pressing it must not
    // take the pane down with it.
    fireEvent.click(screen.getByRole("button", { name: "Reply" }));
    await waitFor(() => expect(asking).toHaveBeenCalled());
    expect(screen.getByText("Quarterly report")).toBeTruthy();
  });

  it("passes the message on with what it says and the files it carries", async () => {
    message.mockResolvedValue(DETAILS);
    const onForward = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageReader messageId="42" folderId="1" onForward={onForward} />
      </I18nProvider>,
    );
    await screen.findByText("Quarterly report");

    fireEvent.click(screen.getByRole("button", { name: "Forward" }));

    await waitFor(() => expect(onForward).toHaveBeenCalledTimes(1));
    expect(forwarding).toHaveBeenCalledWith("42", false);
    const forward = onForward.mock.calls[0]?.[0];
    expect(forward).toMatchObject({
      to: "",
      cc: "",
      subject: "Fwd: Quarterly report",
      answering: null,
      chain: [],
      fromAccount: "acc-1",
      files: [{ token: "t1", name: "report.pdf", bytes: "2048" }],
      leftBehind: [],
    });
    expect(forward.body).toContain(
      "---------- Forwarded message ----------\nFrom: Zeta Quorin <zeta@example.com>",
    );
    expect(forward.body).toContain("To: kappa@example.net");
    expect(forward.body.endsWith("\n\nThe figures for the quarter.")).toBe(true);
    expect(lettingGo).not.toHaveBeenCalled();
  });

  it("passes the message on when the window says the key was pressed", async () => {
    message.mockResolvedValue(DETAILS);
    const onForward = vi.fn();
    const reader = (asked: number) => (
      <I18nProvider initial="en">
        <MessageReader
          messageId="42"
          folderId="1"
          onForward={onForward}
          askedToForward={asked}
        />
      </I18nProvider>
    );
    const { rerender } = render(reader(0));
    await screen.findByText("Quarterly report");
    expect(forwarding).not.toHaveBeenCalled();

    rerender(reader(1));

    await waitFor(() => expect(onForward).toHaveBeenCalledTimes(1));
    expect(forwarding).toHaveBeenCalledWith("42", false);
  });

  it("asks once when the key is pressed again before the first answer", async () => {
    // A held key repeats, and a key never reaches the button that shuts while the core is
    // asked: two composers would open, the second over the first.
    forwarding.mockReturnValue(new Promise(() => {}));
    message.mockResolvedValue(DETAILS);
    const reader = (asked: number) => (
      <I18nProvider initial="en">
        <MessageReader
          messageId="42"
          folderId="1"
          onForward={vi.fn()}
          askedToForward={asked}
        />
      </I18nProvider>
    );
    const { rerender } = render(reader(0));
    await screen.findByText("Quarterly report");

    rerender(reader(1));
    await waitFor(() => expect(forwarding).toHaveBeenCalledTimes(1));
    rerender(reader(2));
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));

    expect(forwarding).toHaveBeenCalledTimes(1);
  });

  it("takes a key pressed while the next message is on its way as about nothing, not the one before", async () => {
    // The message on screen is the one a key is about. The answer of the one before used to wait
    // in the ref until the next was drawn, and a key pressed meanwhile answered a message nobody
    // was looking at any more.
    message.mockResolvedValueOnce(DETAILS);
    message.mockReturnValueOnce(new Promise(() => {}));
    const onForward = vi.fn();
    const onReply = vi.fn();
    const reader = (messageId: string, asked: number) => (
      <I18nProvider initial="en">
        <MessageReader
          messageId={messageId}
          folderId="1"
          onReply={onReply}
          askedToAnswer={asked}
          onForward={onForward}
          askedToForward={asked}
        />
      </I18nProvider>
    );
    const { rerender } = render(reader("42", 0));
    await screen.findByText("Quarterly report");

    rerender(reader("43", 0));
    await screen.findByText("Fetching the message…");
    rerender(reader("43", 1));
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));

    expect(forwarding).not.toHaveBeenCalled();
    expect(asking).not.toHaveBeenCalled();
  });

  it("answers everybody on the message when the window asks by key", async () => {
    message.mockResolvedValue(DETAILS);
    asking.mockReturnValue(new Promise(() => {}));
    const reader = (asked: number) => (
      <I18nProvider initial="en">
        <MessageReader
          messageId="42"
          folderId="1"
          onReply={() => {}}
          askedToAnswerAll={asked}
        />
      </I18nProvider>
    );
    const { rerender } = render(reader(0));
    await screen.findByText("Quarterly report");
    // Nothing on the first drawing: a count it was drawn with is not a key somebody pressed.
    expect(asking).not.toHaveBeenCalled();

    rerender(reader(1));

    await waitFor(() => expect(asking).toHaveBeenCalledWith("42", true));
  });

  it("throws the message away when the window asks by key", async () => {
    message.mockResolvedValue(DETAILS);
    deleting.mockReturnValue(new Promise(() => {}));
    const reader = (asked: number) => (
      <I18nProvider initial="en">
        <MessageReader messageId="42" folderId="1" askedToThrowAway={asked} />
      </I18nProvider>
    );
    const { rerender } = render(reader(0));
    await screen.findByText("Quarterly report");
    expect(deleting).not.toHaveBeenCalled();

    rerender(reader(1));

    await waitFor(() => expect(deleting).toHaveBeenCalledWith("42", "1"));
    expect(deleting).toHaveBeenCalledTimes(1);
  });

  it("says so when the message cannot be read to pass it on", async () => {
    forwarding.mockRejectedValue({
      code: "offline",
      message: "The mail provider cannot be reached.",
      path: null,
    });
    message.mockResolvedValue(DETAILS);
    const onForward = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageReader messageId="42" folderId="1" onForward={onForward} />
      </I18nProvider>,
    );
    await screen.findByText("Quarterly report");

    fireEvent.click(screen.getByRole("button", { name: "Forward" }));

    expect((await screen.findByRole("alert")).textContent).toBe(
      "The mail server could not be reached",
    );
    expect(onForward).not.toHaveBeenCalled();
    expect(
      screen.getByRole("button", { name: "Forward" }).hasAttribute("disabled"),
    ).toBe(false);
  });

  it("asks once however fast Forward is pressed twice", async () => {
    message.mockResolvedValue(DETAILS);
    renderReader("42");
    await screen.findByText("Quarterly report");

    const forward = screen.getByRole("button", { name: "Forward" });
    fireEvent.click(forward);
    fireEvent.click(forward);

    await waitFor(() => expect(forwarding).toHaveBeenCalled());
    expect(forwarding).toHaveBeenCalledTimes(1);
  });

  it("lets go of the files when nobody is listening for the message passed on", async () => {
    // The core holds them for a composer that will never open, until the application quits.
    forwarding.mockResolvedValue({
      to: [],
      cc: [],
      text: "",
      files: [
        { token: "t1", name: "report.pdf", bytes: "2048" },
        { token: "t2", name: "chart.png", bytes: "512" },
      ],
      leftBehind: [],
    });
    message.mockResolvedValue(DETAILS);
    render(
      <I18nProvider initial="en">
        <MessageReader messageId="42" folderId="1" />
      </I18nProvider>,
    );
    await screen.findByText("Quarterly report");

    fireEvent.click(screen.getByRole("button", { name: "Forward" }));

    await waitFor(() => expect(lettingGo).toHaveBeenCalledTimes(2));
    expect(lettingGo).toHaveBeenCalledWith("t1");
    expect(lettingGo).toHaveBeenCalledWith("t2");
  });

  it("lets go of the files when the reader has gone by the time they are held", async () => {
    let answer: (forward: ForwardDto) => void = () => {};
    forwarding.mockReturnValue(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    message.mockResolvedValue(DETAILS);
    const onForward = vi.fn();
    const { unmount } = render(
      <I18nProvider initial="en">
        <MessageReader messageId="42" folderId="1" onForward={onForward} />
      </I18nProvider>,
    );
    await screen.findByText("Quarterly report");
    fireEvent.click(screen.getByRole("button", { name: "Forward" }));
    await waitFor(() => expect(forwarding).toHaveBeenCalled());

    unmount();
    await act(async () =>
      answer({
        to: [],
        cc: [],
        text: "",
        files: [{ token: "t1", name: "report.pdf", bytes: "2048" }],
        leftBehind: [],
      }),
    );

    await waitFor(() => expect(lettingGo).toHaveBeenCalledWith("t1"));
    expect(onForward).not.toHaveBeenCalled();
  });

  /** Opens the menu of ways to pass a message on, as a pointer does. */
  async function waysToForward() {
    fireEvent.pointerDown(
      screen.getByRole("button", { name: "Ways to forward" }),
      { button: 0, ctrlKey: false },
    );
  }

  it("passes the message on whole from the menu beside Forward, as one file and nothing written", async () => {
    forwarding.mockResolvedValue({
      to: [],
      cc: [],
      text: "",
      files: [{ token: "t9", name: "Quarterly report.eml", bytes: "4096" }],
      leftBehind: [],
    });
    message.mockResolvedValue(DETAILS);
    const onForward = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageReader messageId="42" folderId="1" onForward={onForward} />
      </I18nProvider>,
    );
    await screen.findByText("Quarterly report");

    await waysToForward();
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Forward as attachment" }),
    );

    await waitFor(() => expect(onForward).toHaveBeenCalledTimes(1));
    expect(forwarding).toHaveBeenCalledWith("42", true);
    expect(onForward).toHaveBeenCalledWith({
      to: "",
      cc: "",
      subject: "Fwd: Quarterly report",
      body: "",
      answering: null,
      chain: [],
      fromAccount: "acc-1",
      files: [{ token: "t9", name: "Quarterly report.eml", bytes: "4096" }],
      leftBehind: [],
    });
  });

  it("lists every way to forward in the menu, passing on as text among them", async () => {
    message.mockResolvedValue(DETAILS);
    const onForward = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageReader messageId="42" folderId="1" onForward={onForward} />
      </I18nProvider>,
    );
    await screen.findByText("Quarterly report");

    await waysToForward();
    fireEvent.click(await screen.findByRole("menuitem", { name: "Forward" }));

    await waitFor(() => expect(onForward).toHaveBeenCalledTimes(1));
    expect(forwarding).toHaveBeenCalledWith("42", false);
    expect(onForward.mock.calls[0]?.[0].body).toContain(
      "---------- Forwarded message ----------",
    );
  });

  it("offers passing on as an icon that still answers to a name", async () => {
    message.mockResolvedValue(DETAILS);
    renderReader("42", "ru");
    await screen.findByText("Quarterly report");

    expect(screen.getByRole("button", { name: "Переслать" })).toBeTruthy();
  });
});

