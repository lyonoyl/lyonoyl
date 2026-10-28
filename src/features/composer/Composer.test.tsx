import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { StrictMode, useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { NOTHING_TO_ANSWER, type Answer } from "@/entities/message";
import { ScreenPane } from "@/app/ScreenPane";
import { I18nProvider } from "@/shared/i18n";
import type { AttachingDto } from "@/shared/ipc/generated/AttachingDto";
import type { MailAccountSummaryDto } from "@/shared/ipc/generated/MailAccountSummaryDto";
import { Composer } from "./Composer";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    sendMessage: vi.fn(),
    attachFile: vi.fn(),
    forgetAttachment: vi.fn(),
    keepDraft: vi.fn(),
    latestDraft: vi.fn(),
    discardDraft: vi.fn(),
    reopenDraft: vi.fn(),
    noteDraft: vi.fn(),
    forgetNotedDraft: vi.fn(),
  };
});

const {
  sendMessage,
  keepDraft,
  latestDraft,
  discardDraft,
  attachFile,
  forgetAttachment,
  reopenDraft,
  noteDraft,
  forgetNotedDraft,
} = await import("@/shared/ipc/client");
const sending = vi.mocked(sendMessage);
const keeping = vi.mocked(keepDraft);
const lastKept = vi.mocked(latestDraft);
const throwingAway = vi.mocked(discardDraft);
const attach = vi.mocked(attachFile);
const forget = vi.mocked(forgetAttachment);
const reopening = vi.mocked(reopenDraft);
const noting = vi.mocked(noteDraft);
const forgetting = vi.mocked(forgetNotedDraft);
/** The name a composer goes by in the core: made at random as it opens, for nobody to predict. */
const A_COMPOSER = expect.stringMatching(/^[0-9a-f]{32}$/);

function account(
  id: string,
  email: string,
  canSend: boolean,
): MailAccountSummaryDto {
  return { id, email, displayName: "", provider: "imap", canSend };
}

/**
 * In the pane it ships in, because the way out of this screen is the pane's cross.
 *
 * The composer has no leaving control of its own: leaving writes down what was typed in the last
 * second and refuses while the message is going out, and the cross reaches that through `leaveRef`.
 * Rendered bare, these tests would prove nothing about the button a person actually presses.
 */
function renderComposer(accounts: MailAccountSummaryDto[], answer?: Answer) {
  const onWritten = vi.fn();
  const onCancel = vi.fn();
  const onKept = vi.fn();
  const leave: { current: ((then?: () => void) => void) | null } = {
    current: null,
  };
  const shown = (answer?: Answer) => (
    <I18nProvider initial="en">
      <ScreenPane
        paneRef={{ current: null }}
        closeLabel="Close this screen"
        onClose={() => leave.current?.()}
      >
        <Composer
          accounts={accounts}
          answer={answer}
          onWritten={onWritten}
          onCancel={onCancel}
          onKept={onKept}
          leaveRef={leave}
        />
      </ScreenPane>
    </I18nProvider>
  );
  const { unmount, rerender } = render(shown(answer));
  /**
   * The same window handed another answer, which is what the screen does when it keys two openings
   * alike: the compose button pressed over a draft with no address and no subject, say.
   */
  const handedAnother = (next: Answer) => rerender(shown(next));
  /** The way out as the screen takes it to open another message in this one's place. */
  const leaveFor = (then: () => void) => act(() => leave.current?.(then));
  return { onWritten, onCancel, onKept, unmount, handedAnother, leaveFor };
}

function type(label: RegExp, value: string) {
  fireEvent.change(screen.getByLabelText(label), { target: { value } });
}

/**
 * Waits for a draft opened in the window to have its files back, which is when it may be sent.
 *
 * Before then the Send button is not there to press: a message sent from a draft whose files had
 * not arrived would go without them, and the draft they are in would be thrown away with it.
 */
async function opened() {
  await waitFor(() =>
    expect(
      (screen.getByRole("button", { name: /^Send$/ }) as HTMLButtonElement)
        .disabled,
    ).toBe(false),
  );
}

beforeEach(() => {
  attach.mockReset();
  attach.mockResolvedValue(null);
  forget.mockReset();
  forget.mockResolvedValue(undefined);
  vi.useRealTimers();
  keeping.mockReset();
  keeping.mockResolvedValue(1);
  lastKept.mockReset();
  lastKept.mockResolvedValue(null);
  throwingAway.mockReset();
  throwingAway.mockResolvedValue(undefined);
  sending.mockReset();
  sending.mockResolvedValue("<one@example.com>");
  reopening.mockReset();
  reopening.mockResolvedValue([]);
  noting.mockReset();
  noting.mockResolvedValue(undefined);
  forgetting.mockReset();
  forgetting.mockResolvedValue(undefined);
});

describe("writing a message", () => {
  it("hands the message over under the account it is written from", async () => {
    const { onWritten } = renderComposer([
      account("acc-1", "zeta@example.com", true),
    ]);

    type(/^To$/, "kappa@example.net");
    type(/^Subject$/, "About Tuesday");
    type(/^Message$/, "Shall we?");
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(sending).toHaveBeenCalledTimes(1));
    expect(sending).toHaveBeenCalledWith("acc-1", {
      from: "zeta@example.com",
      to: ["kappa@example.net"],
      cc: [],
      bcc: [],
      subject: "About Tuesday",
      body: "Shall we?",
      answering: null,
      chain: [],
      files: [],
      writing: A_COMPOSER,
      kept: null,
    });
    await waitFor(() => expect(onWritten).toHaveBeenCalled());
  });

  it("draws no frame round the message while it is written", () => {
    // The caret says where the typing goes; a frame round the whole of the message says nothing.
    renderComposer([account("acc-1", "zeta@example.com", true)]);

    const field = screen.getByLabelText(/^message$/i);
    field.focus();

    expect(
      field.className
        .split(/\s+/)
        .some((name) => /(^|:)ring(-|$)/.test(name)),
    ).toBe(false);
    expect(field.className.split(/\s+/)).toContain("outline-none");
  });

  it("keeps what is written in its own direction, and the addresses left to right", () => {
    // A subject answered in English keeps its question mark at the end in an Arabic window, every
    // line of the text runs the way its own language runs, and an address reads one way everywhere.
    renderComposer([account("acc-1", "zeta@example.com", true)]);

    // Empty, a field runs the window's way, so the caret starts where the window's lines do.
    expect(screen.getByLabelText(/^subject$/i).getAttribute("dir")).toBeNull();
    expect(screen.getByLabelText(/^message$/i).getAttribute("dir")).toBeNull();

    type(/^subject$/i, "Re: Lunch tomorrow?");
    type(/^message$/i, "See you then");
    expect(screen.getByLabelText(/^subject$/i).getAttribute("dir")).toBe(
      "auto",
    );
    expect(screen.getByLabelText(/^message$/i).getAttribute("dir")).toBe(
      "auto",
    );
    expect(screen.getByLabelText(/^to$/i).getAttribute("dir")).toBe("ltr");
  });

  it("reads several addresses out of one line", async () => {
    // What people paste: commas, and the spaces that come with them.
    renderComposer([account("acc-1", "zeta@example.com", true)]);

    type(/^To$/, "kappa@example.net,  sigma@example.org ,");
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(sending).toHaveBeenCalled());
    expect(sending.mock.calls[0][1].to).toEqual([
      "kappa@example.net",
      "sigma@example.org",
    ]);
  });

  it("will not send a message with nobody to send it to", () => {
    renderComposer([account("acc-1", "zeta@example.com", true)]);

    type(/^Subject$/, "About Tuesday");

    expect(
      (screen.getByRole("button", { name: "Send" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });

  it("counts a blind copy as somebody to send it to", async () => {
    renderComposer([account("acc-1", "zeta@example.com", true)]);

    fireEvent.click(screen.getByRole("button", { name: "Add copies" }));
    type(/^Blind copy to$/, "kappa@example.net");
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(sending).toHaveBeenCalled());
    expect(sending.mock.calls[0][1].bcc).toEqual(["kappa@example.net"]);
  });

  it("offers only the accounts that have somewhere to send from", () => {
    // An account with no sending server would take the message and leave it in a queue nobody
    // watches, which is worse than saying so.
    renderComposer([
      account("acc-1", "reads-only@example.com", false),
      account("acc-2", "zeta@example.com", true),
    ]);

    // One account that can send means no picker at all — there is nothing to pick between.
    expect(screen.queryByLabelText(/^From$/)).toBeNull();
    expect(screen.getByRole("button", { name: "Send" })).toBeTruthy();
  });

  it("lists exactly the accounts that can send, when there is a choice", () => {
    renderComposer([
      account("acc-1", "reads-only@example.com", false),
      account("acc-2", "zeta@example.com", true),
      account("acc-3", "work@example.com", true),
    ]);

    const offered = Array.from(
      (screen.getByLabelText(/^From$/) as HTMLSelectElement).options,
      (option) => option.textContent,
    );
    expect(offered).toEqual(["zeta@example.com", "work@example.com"]);
    // A name somebody chose keeps its own direction.
    expect(
      Array.from(
        (screen.getByLabelText(/^From$/) as HTMLSelectElement).options,
        (option) => option.getAttribute("dir"),
      ),
    ).toEqual(["auto", "auto"]);
  });

  it("says so rather than offering a form nobody can send from", () => {
    renderComposer([account("acc-1", "reads-only@example.com", false)]);

    expect(screen.getByText("No account can send yet")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
  });

  it("says what went wrong and keeps what was written", async () => {
    // A message that could not be handed over must not be a message somebody has to write again.
    sending.mockRejectedValue({
      code: "invalidRequest",
      message: "That is not an address a message can be sent to.",
      path: null,
    });
    renderComposer([account("acc-1", "zeta@example.com", true)]);

    // One the window cannot tell is wrong -- it is shaped like an address, and only the server
    // knows there is nobody there. What this checks is what happens when the server says so.
    type(/^To$/, "nobody@nowhere.invalid");
    type(/^Message$/, "Shall we?");
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(
      (screen.getByLabelText(/^Message$/) as HTMLTextAreaElement).value,
    ).toBe("Shall we?");
  });

  it("opens an answer addressed and titled, with the conversation it belongs to", async () => {
    const answer: Answer = {
      to: "zeta@example.com",
      cc: "",
      subject: "Re: About Tuesday",
      answering: "first@example.com",
      chain: ["older@example.com", "first@example.com"],
      fromAccount: "acc-1",
      body: "",
    };
    renderComposer([account("acc-1", "kappa@example.net", true)], answer);

    expect((screen.getByLabelText(/^To$/) as HTMLInputElement).value).toBe(
      "zeta@example.com",
    );
    expect((screen.getByLabelText(/^Subject$/) as HTMLInputElement).value).toBe(
      "Re: About Tuesday",
    );

    type(/^Message$/, "Tuesday suits");
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(sending).toHaveBeenCalled());
    // Carried through untouched: what becomes of the two of them is RFC 5322 §3.6.4, and that
    // rule lives in the domain rather than being applied a second time on the way there.
    expect(sending).toHaveBeenCalledWith("acc-1", {
      from: "kappa@example.net",
      to: ["zeta@example.com"],
      cc: [],
      bcc: [],
      subject: "Re: About Tuesday",
      body: "Tuesday suits",
      answering: "first@example.com",
      chain: ["older@example.com", "first@example.com"],
      files: [],
      writing: A_COMPOSER,
      kept: null,
    });
  });

  it("answers a message that arrived without a name of its own", async () => {
    // Nothing for `In-Reply-To` to point at. It is still a message somebody can answer; it just
    // cannot be answered into a thread, and neither header is written.
    renderComposer([account("acc-1", "kappa@example.net", true)], {
      to: "zeta@example.com",
      cc: "",
      subject: "Re: About Tuesday",
      answering: null,
      chain: [],
      fromAccount: "acc-1",
      body: "",
    });

    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(sending).toHaveBeenCalled());
    expect(sending.mock.calls[0]?.[1]).toMatchObject({
      answering: null,
      chain: [],
      files: [],
    });
  });

  it("keeps what the person changed rather than the answer it opened with", async () => {
    // The fields are filled in, not fixed: an answer sent to somebody else, or retitled, is an
    // ordinary thing to want.
    renderComposer([account("acc-1", "kappa@example.net", true)], {
      to: "zeta@example.com",
      cc: "",
      subject: "Re: About Tuesday",
      answering: "first@example.com",
      chain: ["first@example.com"],
      fromAccount: "acc-1",
      body: "",
    });

    type(/^To$/, "sigma@example.org");
    type(/^Subject$/, "About Wednesday instead");
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(sending).toHaveBeenCalled());
    expect(sending).toHaveBeenCalledWith("acc-1", {
      from: "kappa@example.net",
      to: ["sigma@example.org"],
      cc: [],
      bcc: [],
      subject: "About Wednesday instead",
      body: "",
      // Still the same conversation: who it is addressed to is not what a thread is made of.
      answering: "first@example.com",
      chain: ["first@example.com"],
      files: [],
      writing: A_COMPOSER,
      kept: null,
    });
  });
  it("answers from the account the message arrived in, not the first one on the list", async () => {
    // Answering from another account puts the wrong person in the thread and files the copy in
    // the wrong Sent, and neither is visible to the person writing until somebody replies.
    renderComposer(
      [
        account("acc-1", "zeta@example.com", true),
        account("acc-2", "kappa@example.net", true),
      ],
      {
        to: "sigma@example.org",
        cc: "",
        subject: "Re: About Tuesday",
        answering: "first@example.com",
        chain: ["first@example.com"],
        fromAccount: "acc-2",
        body: "",
      },
    );

    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(sending).toHaveBeenCalled());
    expect(sending.mock.calls[0]?.[0]).toBe("acc-2");
    expect(sending.mock.calls[0]?.[1]).toMatchObject({
      from: "kappa@example.net",
    });
  });

  it("falls back to an account that can send when the one it came to cannot", async () => {
    // The account was removed, or lost the server it sends through. An answer from nowhere is not
    // an answer; starting from a sensible default is something the person can see and change.
    renderComposer([account("acc-1", "zeta@example.com", true)], {
      to: "sigma@example.org",
      cc: "",
      subject: "Re: About Tuesday",
      answering: "first@example.com",
      chain: ["first@example.com"],
      fromAccount: "acc-gone",
      body: "",
    });

    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(sending).toHaveBeenCalled());
    expect(sending.mock.calls[0]?.[0]).toBe("acc-1");
  });
  it("writes down what is in the window a moment after the typing stops", async () => {
    // Every keystroke would be a write to the disk this application waits for. Only at the end
    // would be a message lost by the one thing that ends a window without warning.
    renderComposer([account("acc-1", "zeta@example.com", true)]);

    type(/^To$/, "kappa@example.net");
    type(/^Message$/, "about Tuesday");

    expect(keeping).not.toHaveBeenCalled();

    await waitFor(
      () =>
        expect(keeping).toHaveBeenCalledWith(
          "acc-1",
          null,
          expect.objectContaining({
            recipients: "kappa@example.net",
            body: "about Tuesday",
          }),
        ),
      { timeout: 2000 },
    );
  });

  it("opens on nothing, whatever was written down before", async () => {
    // It used to bring back the newest draft whenever the window opened blank, so every press of
    // Compose started inside the last thing anybody had saved -- an address and a subject from
    // another day, in a window opened to write something else. What is saved lives in the drafts
    // folder now, and is opened from there.
    renderComposer([account("acc-1", "zeta@example.com", true)]);

    await screen.findByLabelText(/^To$/);

    expect((screen.getByLabelText(/^To$/) as HTMLInputElement).value).toBe("");
    expect((screen.getByLabelText(/^Subject$/) as HTMLInputElement).value).toBe(
      "",
    );
    expect(lastKept, "asked for one nobody opened").not.toHaveBeenCalled();
  });

  it("opens a draft that was written down, and replaces it rather than adding one", async () => {
    keeping.mockResolvedValue(4);
    const { onCancel } = renderComposer(
      [account("acc-1", "zeta@example.com", true)],
      {
        ...NOTHING_TO_ANSWER,
        draftId: 4,
        to: "kappa@example.net",
        bcc: "quiet@example.com",
        subject: "About Tuesday",
        body: "half a thought",
      },
    );

    expect((screen.getByLabelText(/^To$/) as HTMLInputElement).value).toBe(
      "kappa@example.net",
    );
    expect(
      (screen.getByLabelText(/^Blind copy to$/) as HTMLInputElement).value,
      "the copies nobody sees came back too",
    ).toBe("quiet@example.com");

    await opened();
    type(/^Message$/, "half a thought, continued");
    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Save as draft" }));

    // Under the number it already had: kept without it, the drafts folder would fill with one
    // more copy of the same message every time somebody opened it. Kept by the leaving itself,
    // before the window is told it may go: the timer's own keep is still waiting then, and a window
    // taken off the screen never runs it.
    await waitFor(() => expect(onCancel).toHaveBeenCalled());
    expect(keeping).toHaveBeenCalledWith(
      "acc-1",
      4,
      expect.objectContaining({ body: "half a thought, continued" }),
    );
  });

  it("brings an unfinished answer back still inside its conversation", async () => {
    // A draft reopened from the drafts folder is still inside its conversation. Brought back as a
    // new message, the next keystroke would write that over the stored draft, and the message
    // would go out with no `In-Reply-To` -- a new conversation for whoever receives it, and one
    // that cannot be undone.
    sending.mockResolvedValue("<answer@example.com>");

    renderComposer([account("acc-1", "zeta@example.com", true)], {
      ...NOTHING_TO_ANSWER,
      draftId: 9,
      to: "kappa@example.net",
      subject: "Re: About Tuesday",
      body: "the half I had written",
      answering: "<tuesday@example.com>",
      chain: ["<kickoff@example.com>", "<tuesday@example.com>"],
    });

    expect((screen.getByLabelText(/^Subject$/) as HTMLInputElement).value).toBe(
      "Re: About Tuesday",
    );

    await opened();
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(sending).toHaveBeenCalled());
    expect(sending.mock.calls[0]?.[1]).toEqual(
      expect.objectContaining({
        answering: "<tuesday@example.com>",
        chain: ["<kickoff@example.com>", "<tuesday@example.com>"],
        files: [],
      }),
    );
  });

  it("keeps an answer in its conversation while it is being written down", async () => {
    // Kept as well as sent: the draft row is what the next window is rebuilt from, so a keep that
    // forgot the conversation would destroy it on disk a moment after it was brought back.
    keeping.mockResolvedValue(9);

    renderComposer([account("acc-1", "zeta@example.com", true)], {
      ...NOTHING_TO_ANSWER,
      draftId: 9,
      to: "kappa@example.net",
      subject: "Re: About Tuesday",
      body: "the half I had written",
      answering: "<tuesday@example.com>",
      chain: ["<tuesday@example.com>"],
    });

    fireEvent.change(screen.getByLabelText(/^Message$/), {
      target: { value: "and the rest of it" },
    });

    await waitFor(
      () =>
        expect(keeping).toHaveBeenCalledWith(
          expect.anything(),
          expect.anything(),
          expect.objectContaining({
            answering: "<tuesday@example.com>",
            chain: ["<tuesday@example.com>"],
          }),
        ),
      { timeout: 2000 },
    );
  });

  it("does not bring anything back into an answer", async () => {
    // Somebody who pressed reply is answering that message. Filling their window with something
    // else they were writing would be answering for them.
    lastKept.mockResolvedValue({
      id: 4,
      recipients: "somebody@example.net",
      copies: "",
      blind: "",
      subject: "Something else",
      body: "half a thought",
      answering: null,
      chain: [],
    });

    renderComposer([account("acc-1", "zeta@example.com", true)], {
      to: "zeta@example.com",
      cc: "",
      subject: "Re: About Tuesday",
      body: "",
      answering: "<first@example.com>",
      chain: [],
      fromAccount: "acc-1",
    });

    await waitFor(() =>
      expect(
        (screen.getByLabelText(/^Subject$/) as HTMLInputElement).value,
      ).toBe("Re: About Tuesday"),
    );
    expect(lastKept).not.toHaveBeenCalled();
  });

  it("hands the outbox the draft the message was kept as, and throws nothing away itself", async () => {
    // The core throws the draft away in the same step as the outbox takes the message. Thrown
    // away from here once the message had gone, it would outlive an application ending in
    // between, an older copy of the message for somebody to send again.
    const { onWritten } = renderComposer(
      [account("acc-1", "zeta@example.com", true)],
      {
        ...NOTHING_TO_ANSWER,
        draftId: 4,
        to: "kappa@example.net",
        subject: "About Tuesday",
        body: "finished",
      },
    );

    await opened();
    fireEvent.click(screen.getByRole("button", { name: /^Send$/ }));

    await waitFor(() => expect(onWritten).toHaveBeenCalled());
    expect(sending).toHaveBeenCalledWith(
      "acc-1",
      expect.objectContaining({ body: "finished", kept: 4 }),
    );
    expect(throwingAway).not.toHaveBeenCalled();
  });

  it("says it was written only once the outbox has it, so the count read next has lost the draft", async () => {
    // Told first, the window would read the sidebar again while the draft is still there, and
    // the count beside Drafts would go on counting a message that has already been sent.
    let taken: (name: string) => void = () => {};
    sending.mockReturnValue(
      new Promise<string>((resolve) => {
        taken = resolve;
      }),
    );
    const { onWritten } = renderComposer(
      [account("acc-1", "zeta@example.com", true)],
      {
        ...NOTHING_TO_ANSWER,
        draftId: 4,
        to: "kappa@example.net",
        subject: "About Tuesday",
        body: "finished",
      },
    );

    await opened();
    fireEvent.click(screen.getByRole("button", { name: /^Send$/ }));
    await waitFor(() => expect(sending).toHaveBeenCalled());
    await act(async () => {});
    expect(onWritten).not.toHaveBeenCalled();

    await act(async () => taken("<one@example.com>"));

    await waitFor(() => expect(onWritten).toHaveBeenCalled());
  });

  it("waits for a keep already on its way before sending, and hands over the number it landed as", async () => {
    // Sent while the keep the timer started was still out, the message went with no draft to throw
    // away -- and the keep then landed, leaving a draft of a message that had already gone.
    let answer: (id: number) => void = () => {};
    keeping.mockReturnValueOnce(
      new Promise<number>((resolve) => {
        answer = resolve;
      }),
    );
    const { onWritten } = renderComposer([
      account("acc-1", "zeta@example.com", true),
    ]);
    type(/^To$/, "kappa@example.net");
    type(/^Message$/, "about Tuesday");
    await waitFor(() => expect(keeping).toHaveBeenCalledTimes(1), {
      timeout: 2000,
    });

    fireEvent.click(screen.getByRole("button", { name: /^Send$/ }));
    await act(async () => answer(9));

    await waitFor(() => expect(onWritten).toHaveBeenCalled());
    expect(sending).toHaveBeenCalledWith(
      "acc-1",
      expect.objectContaining({ body: "about Tuesday", kept: 9 }),
    );
    expect(throwingAway).not.toHaveBeenCalled();
    expect(keeping).toHaveBeenCalledTimes(1);
  });

  it("goes on writing into the same draft when sending failed", async () => {
    // The draft outlives a send that did not happen, so the next keep has to be that draft again
    // rather than a second one beside it.
    sending.mockRejectedValue({
      code: "invalidRequest",
      message: "",
      path: null,
    });
    keeping.mockResolvedValue(4);
    renderComposer([account("acc-1", "zeta@example.com", true)], {
      ...NOTHING_TO_ANSWER,
      draftId: 4,
      to: "kappa@example.net",
      subject: "About Tuesday",
      body: "finished",
    });

    await opened();
    fireEvent.click(screen.getByRole("button", { name: /^Send$/ }));
    await screen.findByRole("alert");
    type(/^Message$/, "finished, and then some");

    await waitFor(() => expect(keeping).toHaveBeenCalled(), { timeout: 2000 });
    expect(keeping).toHaveBeenLastCalledWith(
      "acc-1",
      4,
      expect.objectContaining({ body: "finished, and then some" }),
    );
    expect(throwingAway).not.toHaveBeenCalled();
  });

  it("says when what is being written is first written down, and not again", async () => {
    // Which is when the count beside Drafts has one more to count.
    keeping.mockResolvedValue(9);
    const { onKept } = renderComposer([
      account("acc-1", "zeta@example.com", true),
    ]);

    type(/^Message$/, "about Tuesday");
    await waitFor(() => expect(onKept).toHaveBeenCalledTimes(1), {
      timeout: 2000,
    });

    type(/^Message$/, "about Tuesday, and Wednesday");
    await waitFor(() => expect(keeping).toHaveBeenCalledTimes(2), {
      timeout: 2000,
    });
    expect(onKept).toHaveBeenCalledTimes(1);
  });

  it("says nothing new is kept when a draft opened from the folder is written down again", async () => {
    keeping.mockResolvedValue(4);
    const { onKept } = renderComposer(
      [account("acc-1", "zeta@example.com", true)],
      {
        ...NOTHING_TO_ANSWER,
        draftId: 4,
        to: "kappa@example.net",
        subject: "About Tuesday",
        body: "half a thought",
      },
    );

    type(/^Message$/, "half a thought, continued");
    await waitFor(() => expect(keeping).toHaveBeenCalled(), { timeout: 2000 });
    expect(onKept).not.toHaveBeenCalled();
  });

  it("writes down what is in the window before it closes, and waits", async () => {
    // The last thing typed is inside the wait the timer is serving out. Closing without this
    // loses whatever was written in the second before it, which is the whole point of keeping it.
    let letItFinish: (id: number) => void = () => {};
    keeping.mockImplementation(
      () =>
        new Promise<number>((settle) => {
          letItFinish = settle;
        }),
    );

    const { onCancel } = renderComposer([
      account("acc-1", "zeta@example.com", true),
    ]);
    type(/^Message$/, "one last thought");

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    // Asked first: leaving with something written is a decision, and both answers are ones people
    // mean.
    fireEvent.click(screen.getByRole("button", { name: "Save as draft" }));

    await waitFor(() =>
      expect(keeping).toHaveBeenCalledWith(
        "acc-1",
        null,
        expect.objectContaining({ body: "one last thought" }),
      ),
    );
    expect(onCancel).not.toHaveBeenCalled();

    letItFinish(1);

    await waitFor(() => expect(onCancel).toHaveBeenCalled());
  });

  it("stays open and says so when what is in it could not be written down", async () => {
    // Closing over a failed keep is losing the text silently, which is the one thing this is for.
    keeping.mockRejectedValue({
      code: "storageUnavailable",
      message: "",
      path: null,
    });

    const { onCancel } = renderComposer([
      account("acc-1", "zeta@example.com", true),
    ]);
    type(/^Message$/, "one last thought");

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Save as draft" }));

    await waitFor(() => expect(screen.getByRole("alert")).toBeTruthy());
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("loses nothing to keeps refused for want of room, and says so on the way out like any failed keep", async () => {
    // An account whose drafts hold as much as they may refuses every keep, as a request the core
    // will not take. The timer's keep says nothing, as no failed keep does while somebody writes;
    // what is on screen stays; and leaving with it kept says why it cannot, and stays open.
    keeping.mockRejectedValue({
      code: "invalidRequest",
      message: "",
      path: null,
    });
    const { onCancel } = renderComposer([
      account("acc-1", "zeta@example.com", true),
    ]);
    type(/^To$/, "kappa@example.net");
    type(/^Message$/, "about Tuesday");

    await waitFor(() => expect(keeping).toHaveBeenCalledTimes(1), {
      timeout: 2000,
    });
    await act(async () => {});
    expect(
      screen.queryByRole("alert"),
      "a keep the timer made interrupted the writing",
    ).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Save as draft" }));

    expect((await screen.findByRole("alert")).textContent).toBe(
      "The app made a request the native core rejected",
    );
    expect(onCancel).not.toHaveBeenCalled();
    expect(
      (screen.getByLabelText(/^Message$/) as HTMLTextAreaElement).value,
    ).toBe("about Tuesday");
    expect((screen.getByLabelText(/^To$/) as HTMLInputElement).value).toBe(
      "kappa@example.net",
    );
  });
});

describe("Composer, the addresses in the text", () => {
  /** The copy of the text drawn under the field, where the lines under the addresses are. */
  function under(): HTMLElement {
    const field = screen.getByLabelText(/^Message$/);
    const copy = field.previousElementSibling;
    if (!(copy instanceof HTMLElement)) throw new Error("nothing is drawn under the field");
    return copy;
  }

  it("underlines an address the moment it is typed, and nothing that is not one", () => {
    renderComposer([account("acc-1", "zeta@example.com", true)]);

    type(/^Message$/, "See https://example.com/a, or example.org, and www.example.net.");

    expect(
      [...under().querySelectorAll("[data-link]")].map((mark) => mark.textContent),
    ).toEqual(["https://example.com/a", "www.example.net"]);
    // The copy holds the text as it is in the field, every character in its place.
    expect(under().textContent).toBe(
      "See https://example.com/a, or example.org, and www.example.net.\u200B",
    );
  });

  it("moves the line with the text as it is changed", () => {
    renderComposer([account("acc-1", "zeta@example.com", true)]);

    type(/^Message$/, "https://example.com/a");
    expect(under().querySelectorAll("[data-link]")).toHaveLength(1);

    type(/^Message$/, "xhttps://example.com/a");
    expect(under().querySelectorAll("[data-link]")).toHaveLength(0);
  });

  it("is drawn for nobody but the eye, and leaves the field as the thing typed into", () => {
    renderComposer([account("acc-1", "zeta@example.com", true)]);
    type(/^Message$/, "https://example.com/a");

    const copy = under();
    expect(copy.getAttribute("aria-hidden")).toBe("true");
    expect(copy.className.split(/\s+/)).toEqual(
      expect.arrayContaining(["pointer-events-none", "text-transparent"]),
    );
    expect((screen.getByLabelText(/^Message$/) as HTMLTextAreaElement).value).toBe(
      "https://example.com/a",
    );
  });

  it("scrolls with the field", () => {
    renderComposer([account("acc-1", "zeta@example.com", true)]);
    type(/^Message$/, "line\n".repeat(200) + "https://example.com/a");
    const field = screen.getByLabelText(/^Message$/);

    field.scrollTop = 120;
    fireEvent.scroll(field);

    expect(under().scrollTop).toBe(120);
  });
});

describe("Composer, attaching a file", () => {
  const CHOSEN = { token: "t1", name: "invoice.pdf", bytes: "2500000" };

  const write = () =>
    renderComposer([account("acc-1", "zeta@example.com", true)]);

  it("lists what was chosen, and sends it by its token", async () => {
    // The bytes never cross: the file was chosen in the core and stayed there. A photograph
    // travelling out to be listed and back to be sent is a copy of somebody's file in a webview.
    attach.mockResolvedValue(CHOSEN);
    sending.mockResolvedValue("<m@example.com>");

    write();
    fireEvent.click(
      await screen.findByRole("button", { name: /attach a file/i }),
    );

    expect(await screen.findByText("invoice.pdf")).toBeTruthy();
    expect(screen.getByText("2.5 MB")).toBeTruthy();
    // A file's name keeps its own direction.
    expect(screen.getByText("invoice.pdf").getAttribute("dir")).toBe("auto");

    fireEvent.change(screen.getByLabelText(/^To$/), {
      target: { value: "kappa@example.net" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^Send$/ }));

    await waitFor(() =>
      expect(sending).toHaveBeenCalledWith(
        expect.anything(),
        expect.objectContaining({ files: ["t1"] }),
      ),
    );
  });

  it("forgets it in the core as well as on screen", async () => {
    // Dropped here and kept there, the bytes are memory nothing will ever release.
    attach.mockResolvedValue(CHOSEN);

    write();
    fireEvent.click(
      await screen.findByRole("button", { name: /attach a file/i }),
    );
    fireEvent.click(
      await screen.findByRole("button", { name: /remove invoice\.pdf/i }),
    );

    await waitFor(() => expect(forget).toHaveBeenCalledWith("t1"));
    expect(screen.queryByText("invoice.pdf")).toBeNull();
  });

  it("treats closing the dialog as an answer rather than a failure", async () => {
    attach.mockResolvedValue(null);

    write();
    fireEvent.click(
      await screen.findByRole("button", { name: /attach a file/i }),
    );

    await waitFor(() => expect(attach).toHaveBeenCalled());
    // Flushed, because the answer arrives in a promise: asserting on the moment the call was made
    // is asserting before anything could have gone wrong.
    await act(async () => {});

    expect(screen.queryByRole("alert")).toBeNull();
    // Both halves, and the positive one first. Asserting only that no row appeared is an
    // assertion a crashed render also satisfies — everything is absent then, including the thing
    // being looked for.
    expect(screen.getByRole("button", { name: /attach a file/i })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /^remove/i })).toBeNull();
  });

  it("says so when the file is too large", async () => {
    // Otherwise the button looks like it did nothing, which is the same thing a person sees when
    // they close the dialog — and the two mean opposite things.
    attach.mockRejectedValue({
      code: "fileTooLarge",
      message: "",
      path: null,
    });

    write();
    fireEvent.click(
      await screen.findByRole("button", { name: /attach a file/i }),
    );

    expect(await screen.findByText(/too large to attach/i)).toBeTruthy();
  });

  it("says a file could not be read, rather than calling it too large", async () => {
    // Two failures, two codes: a file that is gone or not permitted is not a file that is too big.
    attach.mockRejectedValue({
      code: "fileNotRead",
      message: "",
      path: null,
    });

    write();
    fireEvent.click(
      await screen.findByRole("button", { name: /attach a file/i }),
    );

    expect(await screen.findByText(/could not be read/i)).toBeTruthy();
    expect(screen.queryByText(/too large/i)).toBeNull();
  });

  it("asks for one file at a time, so each is weighed beside the one before it", async () => {
    // Asked for again while the first is still being read, the second would be weighed beside
    // the files on screen without the first: two that fit apart and not together would both be
    // held, and every keep after them refused.
    let chosen: (file: typeof CHOSEN) => void = () => {};
    attach
      .mockReturnValueOnce(
        new Promise((resolve) => {
          chosen = resolve;
        }),
      )
      .mockResolvedValueOnce(null);
    write();
    const attaching = () =>
      screen.getByRole("button", {
        name: /attach a file/i,
      }) as HTMLButtonElement;

    fireEvent.click(attaching());
    expect(
      attaching().getAttribute("aria-disabled"),
      "nothing says a second file cannot be asked for while the first is being read",
    ).toBe("true");
    // Said and refused rather than disabled: a button disabled under the focus drops it to the
    // document, and the next Tab after the panel starts again at the top of the window.
    expect(
      attaching().disabled,
      "the button was disabled under the focus of whoever pressed it",
    ).toBe(false);
    fireEvent.click(attaching());
    expect(
      attach,
      "a second file was asked for while the first was being read",
    ).toHaveBeenCalledTimes(1);

    await act(async () => chosen(CHOSEN));
    expect(await screen.findByText("invoice.pdf")).toBeTruthy();
    expect(attaching().getAttribute("aria-disabled")).toBeNull();
    fireEvent.click(attaching());

    await waitFor(() => expect(attach).toHaveBeenCalledTimes(2));
    expect(attach.mock.calls).toEqual([[[]], [["t1"]]]);
  });

  it("sends nothing while a chosen file is still being read, and sends it once it is in", async () => {
    // Sent while the file is on its way in, the message would leave without it and say nothing.
    let chosen: (file: typeof CHOSEN) => void = () => {};
    attach.mockReturnValueOnce(
      new Promise((resolve) => {
        chosen = resolve;
      }),
    );
    sending.mockResolvedValue("<m@example.com>");
    write();
    fireEvent.change(await screen.findByLabelText(/^To$/), {
      target: { value: "kappa@example.net" },
    });
    const sendButton = () =>
      screen.getByRole("button", { name: /^Send$/ }) as HTMLButtonElement;

    fireEvent.click(screen.getByRole("button", { name: /attach a file/i }));
    expect(
      sendButton().disabled,
      "Send offered while a file is being read",
    ).toBe(true);
    // Submitted past the button, the way Enter in a field submits the form.
    fireEvent.submit(sendButton().closest("form")!);
    await act(async () => {});
    expect(
      sending,
      "the message left while its file was still being read",
    ).not.toHaveBeenCalled();

    await act(async () => chosen(CHOSEN));
    expect(await screen.findByText("invoice.pdf")).toBeTruthy();
    fireEvent.click(sendButton());

    await waitFor(() =>
      expect(sending).toHaveBeenCalledWith(
        expect.anything(),
        expect.objectContaining({ files: ["t1"] }),
      ),
    );
  });

  it("lets go of a file still being read when the window goes, and keeps the draft without it", async () => {
    // Leaving is not refused while a file is read: a read that never ends would hold the window
    // open for good. Read in after the window went, the file is shown by nothing, and held in the
    // core it would be held for nobody until the application quits.
    let chosen: (file: typeof CHOSEN) => void = () => {};
    attach.mockReturnValueOnce(
      new Promise((resolve) => {
        chosen = resolve;
      }),
    );
    const { onCancel, unmount } = write();
    type(/^Message$/, "the invoice is attached");
    fireEvent.click(screen.getByRole("button", { name: /attach a file/i }));

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Save as draft" }));
    await waitFor(() => expect(onCancel).toHaveBeenCalled());
    unmount();

    expect(
      keeping.mock.calls,
      "the draft was kept with a file that was not on screen",
    ).toEqual([
      [
        "acc-1",
        null,
        expect.objectContaining({
          body: "the invoice is attached",
          files: [],
        }),
      ],
    ]);
    await act(async () => chosen(CHOSEN));
    await waitFor(() =>
      expect(
        forget,
        "the file read in after the window went is held for nobody",
      ).toHaveBeenCalledWith("t1"),
    );
  });

  it("lets go of a file read in as the window goes, though it was never drawn", async () => {
    // Read in between the screen being told the window is going and the render that takes the
    // window away, the file is never drawn. A way out letting go of what was drawn holds it for
    // nobody until the application quits.
    let chosen: (file: typeof CHOSEN) => void = () => {};
    attach.mockReturnValueOnce(
      new Promise((resolve) => {
        chosen = resolve;
      }),
    );
    const leave: { current: ((then?: () => void) => void) | null } = {
      current: null,
    };
    /** The screen as it ships: told the window is going, it takes it away. */
    function Writing() {
      const [open, setOpen] = useState(true);
      if (!open) return null;
      return (
        <ScreenPane
          paneRef={{ current: null }}
          closeLabel="Close this screen"
          onClose={() => leave.current?.()}
        >
          <Composer
            accounts={[account("acc-1", "zeta@example.com", true)]}
            onWritten={() => {}}
            onCancel={() => {
              // The read ends as the screen takes the window away.
              chosen(CHOSEN);
              setOpen(false);
            }}
            leaveRef={leave}
          />
        </ScreenPane>
      );
    }
    render(
      <I18nProvider initial="en">
        <Writing />
      </I18nProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: /attach a file/i }));

    // Nothing typed, so it goes without asking.
    await act(async () => {
      fireEvent.click(
        screen.getByRole("button", { name: "Close this screen" }),
      );
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    expect(
      screen.queryByRole("button", { name: /attach a file/i }),
      "the window did not go",
    ).toBeNull();
    await waitFor(() =>
      expect(
        forget,
        "the file read in as the window went is held for nobody",
      ).toHaveBeenCalledWith("t1"),
    );
  });

  it("keeps a file read in once leaving could not be made, since the window stayed", async () => {
    // Only a window that has gone lets go of what arrives. One whose keep was refused on the way
    // out is still being written, and the file belongs on screen and in the next keep.
    let chosen: (file: typeof CHOSEN) => void = () => {};
    attach.mockReturnValueOnce(
      new Promise((resolve) => {
        chosen = resolve;
      }),
    );
    keeping.mockRejectedValueOnce({
      code: "storageUnavailable",
      message: "",
      path: null,
    });
    const { onCancel } = write();
    type(/^Message$/, "the invoice is attached");
    fireEvent.click(screen.getByRole("button", { name: /attach a file/i }));
    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Save as draft" }));
    expect(await screen.findByRole("alert")).toBeTruthy();

    await act(async () => chosen(CHOSEN));

    expect(await screen.findByText("invoice.pdf")).toBeTruthy();
    await waitFor(
      () =>
        expect(keeping).toHaveBeenLastCalledWith(
          "acc-1",
          null,
          expect.objectContaining({ files: ["t1"] }),
        ),
      { timeout: 2000 },
    );
    expect(
      forget,
      "a file was let go of by a window that stayed",
    ).not.toHaveBeenCalled();
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("keeps a file chosen in a window mounted twice over, as development mounts it", async () => {
    // In development React mounts a component, takes it away and mounts it again, and the way
    // out runs in between. A window that took that for going would let go of every file it was
    // handed as it arrived.
    attach.mockResolvedValue(CHOSEN);
    render(
      <StrictMode>
        <I18nProvider initial="en">
          <Composer
            accounts={[account("acc-1", "zeta@example.com", true)]}
            onWritten={() => {}}
            onCancel={() => {}}
          />
        </I18nProvider>
      </StrictMode>,
    );

    fireEvent.click(screen.getByRole("button", { name: /attach a file/i }));

    expect(await screen.findByText("invoice.pdf")).toBeTruthy();
    await act(async () => {});
    expect(
      forget,
      "a file was let go of by a window still open",
    ).not.toHaveBeenCalled();
  });

  it("lets another file be asked for once the one before was refused, or the dialog closed", async () => {
    attach
      .mockRejectedValueOnce({ code: "fileTooLarge", message: "", path: null })
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce(CHOSEN);
    write();
    const attaching = () =>
      screen.getByRole("button", {
        name: /attach a file/i,
      }) as HTMLButtonElement;

    fireEvent.click(attaching());
    expect(await screen.findByText(/too large to attach/i)).toBeTruthy();
    expect(
      attaching().getAttribute("aria-disabled"),
      "no file can be asked for after one was refused",
    ).toBeNull();

    fireEvent.click(attaching());
    await waitFor(() => expect(attach).toHaveBeenCalledTimes(2));
    await act(async () => {});
    expect(
      attaching().getAttribute("aria-disabled"),
      "no file can be asked for after the dialog was closed",
    ).toBeNull();

    fireEvent.click(attaching());
    expect(await screen.findByText("invoice.pdf")).toBeTruthy();
  });
});

describe("Composer, a message passed on", () => {
  const INVOICE = { token: "f1", name: "invoice.pdf", bytes: "2500000" };
  const CHART = { token: "f2", name: "chart.png", bytes: "900" };
  /** A message passed on, as the reader opens the composer with it. */
  const FORWARD: Answer = {
    ...NOTHING_TO_ANSWER,
    subject: "Fwd: About Tuesday",
    body: "\n\n---------- Forwarded message ----------\n\nShall we?",
    fromAccount: "acc-1",
    files: [INVOICE, CHART],
    leftBehind: [],
  };

  const open = (forward: Answer = FORWARD) =>
    renderComposer([account("acc-1", "zeta@example.com", true)], forward);

  it("lists the files it carries and sends them by their tokens, asking the core for nothing", async () => {
    // Already held in the core when the window opens: nothing is chosen, and nothing is read back.
    open();

    expect(screen.getByText("invoice.pdf")).toBeTruthy();
    expect(screen.getByText("2.5 MB")).toBeTruthy();
    expect(screen.getByText("chart.png")).toBeTruthy();
    type(/^To$/, "sigma@example.org");
    fireEvent.click(screen.getByRole("button", { name: /^Send$/ }));

    await waitFor(() =>
      expect(sending).toHaveBeenCalledWith(
        "acc-1",
        expect.objectContaining({
          to: ["sigma@example.org"],
          subject: "Fwd: About Tuesday",
          answering: null,
          chain: [],
          files: ["f1", "f2"],
          kept: null,
        }),
      ),
    );
    expect(reopening).not.toHaveBeenCalled();
    expect(attach).not.toHaveBeenCalled();
  });

  it("keeps its files with it when it is written down", async () => {
    open();

    await waitFor(
      () =>
        expect(keeping).toHaveBeenCalledWith(
          "acc-1",
          null,
          expect.objectContaining({ files: ["f1", "f2"] }),
        ),
      { timeout: 2000 },
    );
  });

  it("takes one of its files off like any other, and lets go of it in the core", async () => {
    open();

    fireEvent.click(screen.getByRole("button", { name: "Remove chart.png" }));

    expect(screen.queryByText("chart.png")).toBeNull();
    expect(forget).toHaveBeenCalledWith("f2");
    await waitFor(
      () =>
        expect(keeping).toHaveBeenLastCalledWith(
          "acc-1",
          null,
          expect.objectContaining({ files: ["f1"] }),
        ),
      { timeout: 2000 },
    );
  });

  it("weighs a file chosen beside the ones it carries", async () => {
    // Weighed without them, a file that fits alone would be held beside files it does not fit
    // with, and no keep after it could be written down.
    open();

    fireEvent.click(screen.getByRole("button", { name: /attach a file/i }));

    await waitFor(() => expect(attach).toHaveBeenCalledWith(["f1", "f2"]));
  });

  it("lets go of its files once it is gone", async () => {
    const { unmount } = open();
    await waitFor(() => expect(keeping).toHaveBeenCalled(), { timeout: 2000 });

    unmount();

    await waitFor(() => expect(forget).toHaveBeenCalledWith("f1"));
    expect(forget).toHaveBeenCalledWith("f2");
  });

  it("says each file too heavy to go with it on a line of its own", async () => {
    // They are not among the files on screen, and nothing else says the message carried them.
    open({ ...FORWARD, files: [INVOICE], leftBehind: ["video.mov", "talk.mp4"] });

    const said = screen.getByRole("alert");
    expect(
      Array.from(said.querySelectorAll("p"), (line) => line.textContent),
    ).toEqual([
      "video.mov is not attached because the message would be too large",
      "talk.mp4 is not attached because the message would be too large",
    ]);
    expect(screen.getByText("invoice.pdf")).toBeTruthy();
  });

  it("says nothing about files left behind when none were", () => {
    open();

    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("Composer, the files a draft carries", () => {
  const INVOICE = { token: "r1", name: "invoice.pdf", bytes: "2500000" };
  const PHOTO = { token: "r2", name: "photo.jpg", bytes: "900" };
  /** A draft written down earlier, opened from the drafts folder. */
  const DRAFT: Answer = {
    ...NOTHING_TO_ANSWER,
    draftId: 4,
    to: "kappa@example.net",
    subject: "About Tuesday",
    body: "half a thought",
  };

  const open = (draft: Answer = DRAFT) =>
    renderComposer([account("acc-1", "zeta@example.com", true)], draft);

  /** Lets a keep's timer run out, which is what writing nothing has to outlast. */
  const longerThanAKeepWaits = () =>
    act(() => new Promise((resolve) => setTimeout(resolve, 1000)));

  it("keeps a message carrying nothing but a file, naming the file by its token", async () => {
    // A file and nothing typed is still a message somebody started. The token is what crosses: the
    // bytes are in the core already, and the draft takes them from there.
    attach.mockResolvedValue({
      token: "t1",
      name: "invoice.pdf",
      bytes: "2500000",
    });
    renderComposer([account("acc-1", "zeta@example.com", true)]);

    fireEvent.click(
      await screen.findByRole("button", { name: /attach a file/i }),
    );

    await waitFor(
      () =>
        expect(keeping).toHaveBeenCalledWith(
          "acc-1",
          null,
          expect.objectContaining({ files: ["t1"] }),
        ),
      { timeout: 2000 },
    );
  });

  it("lists the files a draft opened again carried, and sends them by their tokens", async () => {
    reopening.mockResolvedValue([INVOICE]);
    open();

    expect(await screen.findByText("invoice.pdf")).toBeTruthy();
    expect(screen.getByText("2.5 MB")).toBeTruthy();
    expect(reopening).toHaveBeenCalledWith(4);

    await opened();
    fireEvent.click(screen.getByRole("button", { name: /^Send$/ }));

    await waitFor(() =>
      expect(sending).toHaveBeenCalledWith(
        "acc-1",
        expect.objectContaining({ files: ["r1"], kept: 4 }),
      ),
    );
  });

  it("keeps a draft opened again without a file taken off it", async () => {
    keeping.mockResolvedValue(4);
    reopening.mockResolvedValue([INVOICE, PHOTO]);
    open();

    fireEvent.click(
      await screen.findByRole("button", { name: /remove invoice\.pdf/i }),
    );

    await waitFor(
      () =>
        expect(keeping).toHaveBeenLastCalledWith(
          "acc-1",
          4,
          expect.objectContaining({ files: ["r2"] }),
        ),
      { timeout: 2000 },
    );
    expect(forget).toHaveBeenCalledWith("r1");
  });

  it("writes nothing over a draft opened again until its files are back", async () => {
    // A keep naming none of them takes every one of them off the draft.
    let back: (files: (typeof INVOICE)[]) => void = () => {};
    reopening.mockReturnValue(
      new Promise((resolve) => {
        back = resolve;
      }),
    );
    keeping.mockResolvedValue(4);
    open();

    await longerThanAKeepWaits();

    expect(keeping).not.toHaveBeenCalled();
    expect(
      (screen.getByRole("button", { name: /^Send$/ }) as HTMLButtonElement)
        .disabled,
      "a message could be sent without the files its draft carries",
    ).toBe(true);

    await act(async () => back([INVOICE]));
    type(/^Message$/, "half a thought, continued");

    await waitFor(
      () =>
        expect(keeping).toHaveBeenCalledWith(
          "acc-1",
          4,
          expect.objectContaining({
            body: "half a thought, continued",
            files: ["r1"],
          }),
        ),
      { timeout: 2000 },
    );
  });

  it("takes nothing typed until a draft opened again has its files back", async () => {
    // Typed before then, it could not be written down, and leaving would lose it without a word.
    let back: (files: (typeof INVOICE)[]) => void = () => {};
    reopening.mockReturnValue(
      new Promise((resolve) => {
        back = resolve;
      }),
    );
    open();

    const fields = [/^To$/, /^Subject$/, /^Message$/].map(
      (label) => screen.getByLabelText(label) as HTMLInputElement,
    );
    const attaching = screen.getByRole("button", {
      name: /attach a file/i,
    }) as HTMLButtonElement;
    expect(
      [...fields, attaching].map((control) => control.disabled),
      "the window took changes it could not keep",
    ).toEqual([true, true, true, true]);

    await act(async () => back([INVOICE]));

    expect(
      [...fields, attaching].map((control) => control.disabled),
    ).toEqual([false, false, false, false]);
  });

  it("takes nothing typed into a draft whose files could not be had", async () => {
    // For as long as the window is open: nothing in it can be written down, and the draft stays on
    // the disk as it was.
    reopening.mockRejectedValue({
      code: "storageUnavailable",
      message: "",
      path: null,
    });
    open();

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(
      [/^To$/, /^Subject$/, /^Message$/].map(
        (label) => (screen.getByLabelText(label) as HTMLInputElement).disabled,
      ),
      "the window took changes it could not keep",
    ).toEqual([true, true, true]);
  });

  it("asks before leaving a draft whose files are not back yet, and leaves it as it was", async () => {
    // Nothing on the screen yet, and a file in the draft: leaving it unasked would throw the draft
    // away, and the file with it.
    reopening.mockReturnValue(new Promise(() => {}));
    const { onCancel } = open({ ...NOTHING_TO_ANSWER, draftId: 9 });

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "Save as draft" }),
    );

    await waitFor(() => expect(onCancel).toHaveBeenCalled());
    expect(
      throwingAway,
      "the draft was thrown away unasked",
    ).not.toHaveBeenCalled();
    expect(keeping, "the draft was written over").not.toHaveBeenCalled();
  });

  it("asks before leaving a draft whose files could not be had, and leaves it as it was", async () => {
    reopening.mockRejectedValue({
      code: "storageUnavailable",
      message: "",
      path: null,
    });
    const { onCancel } = open({ ...NOTHING_TO_ANSWER, draftId: 9 });
    expect(await screen.findByRole("alert")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "Save as draft" }),
    );

    await waitFor(() => expect(onCancel).toHaveBeenCalled());
    expect(
      throwingAway,
      "the draft was thrown away unasked",
    ).not.toHaveBeenCalled();
    expect(keeping, "the draft was written over").not.toHaveBeenCalled();
  });

  it("stays the draft it was opened on when it is handed a new message before its files are back", async () => {
    // What the compose button hands a window open on a draft with no address and no subject. Taken
    // as a new message, the window would keep under the draft's number with none of its files, and
    // take every one of them off it.
    let back: (files: (typeof INVOICE)[]) => void = () => {};
    reopening.mockReturnValue(
      new Promise((resolve) => {
        back = resolve;
      }),
    );
    keeping.mockResolvedValue(9);
    const { handedAnother } = open({
      ...NOTHING_TO_ANSWER,
      draftId: 9,
      body: "the text so far",
    });

    handedAnother(NOTHING_TO_ANSWER);
    await act(async () => back([INVOICE]));

    expect(await screen.findByText("invoice.pdf")).toBeTruthy();
    await waitFor(
      () =>
        expect(keeping).toHaveBeenCalledWith(
          "acc-1",
          9,
          expect.objectContaining({
            body: "the text so far",
            files: ["r1"],
          }),
        ),
      { timeout: 2000 },
    );
    expect(
      keeping.mock.calls.filter(([, , draft]) => draft.files.length === 0),
      "the draft was kept without its files",
    ).toEqual([]);
  });

  it("stays the draft it was opened on when it is handed a new message after its files could not be had", async () => {
    reopening.mockRejectedValue({
      code: "storageUnavailable",
      message: "",
      path: null,
    });
    const { handedAnother } = open({
      ...NOTHING_TO_ANSWER,
      draftId: 9,
      body: "the text so far",
    });
    expect(await screen.findByRole("alert")).toBeTruthy();

    handedAnother(NOTHING_TO_ANSWER);
    await longerThanAKeepWaits();

    expect(
      keeping,
      "the draft was written over without its files",
    ).not.toHaveBeenCalled();
  });

  it("names every file on screen when it asks for another, the draft's own among them", async () => {
    // What one message may carry is weighed in the core, which holds the files; the window says
    // which of them go with this message.
    reopening.mockResolvedValue([INVOICE]);
    attach
      .mockResolvedValueOnce({ token: "t1", name: "notes.txt", bytes: "40" })
      .mockResolvedValueOnce(null);
    open();
    await opened();

    fireEvent.click(screen.getByRole("button", { name: /attach a file/i }));
    expect(await screen.findByText("notes.txt")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /attach a file/i }));

    await waitFor(() => expect(attach).toHaveBeenCalledTimes(2));
    expect(attach.mock.calls).toEqual([[["r1"]], [["r1", "t1"]]]);
  });

  it("leaves a draft as it was when it is closed before its files are back, and lets go of them when they come", async () => {
    let back: (files: (typeof INVOICE)[]) => void = () => {};
    reopening.mockReturnValue(
      new Promise((resolve) => {
        back = resolve;
      }),
    );
    const { onCancel, unmount } = open();

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Save as draft" }));
    await waitFor(() => expect(onCancel).toHaveBeenCalled());
    unmount();

    expect(
      keeping,
      "the draft was written over without its files",
    ).not.toHaveBeenCalled();

    await act(async () => back([INVOICE]));
    await waitFor(() => expect(forget).toHaveBeenCalledWith("r1"));
  });

  it("says so when the files of a draft cannot be had, and writes nothing over it", async () => {
    reopening.mockRejectedValue({
      code: "storageUnavailable",
      message: "",
      path: null,
    });
    open();

    expect(await screen.findByRole("alert")).toBeTruthy();
    type(/^Message$/, "half a thought, continued");
    await longerThanAKeepWaits();

    expect(keeping).not.toHaveBeenCalled();
    expect(
      (screen.getByRole("button", { name: /^Send$/ }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });

  it("lets go of the files it was holding once it is gone, after the last keep has them", async () => {
    // Held in the core for a window that has gone, a draft opened and closed ten times would be
    // its photographs held ten times over until the application quits.
    let letItFinish: (id: number) => void = () => {};
    keeping.mockImplementation(
      () =>
        new Promise<number>((settle) => {
          letItFinish = settle;
        }),
    );
    reopening.mockResolvedValue([INVOICE]);
    const { unmount } = open();
    await screen.findByText("invoice.pdf");
    await waitFor(() => expect(keeping).toHaveBeenCalled(), { timeout: 2000 });

    unmount();
    await act(async () => {});
    expect(forget, "let go of before the draft had it").not.toHaveBeenCalled();

    await act(async () => letItFinish(4));
    await waitFor(() => expect(forget).toHaveBeenCalledWith("r1"));
  });

  it("throws a message away even when what is in it could not be written down", async () => {
    // Holding more text than a message may carry, it is refused at every keep; throwing it away is
    // the one answer that does not need it kept.
    keeping.mockResolvedValueOnce(7).mockRejectedValue({
      code: "invalidRequest",
      message: "",
      path: null,
    });
    const { onCancel } = renderComposer([
      account("acc-1", "zeta@example.com", true),
    ]);
    type(/^Message$/, "never mind");
    await waitFor(() => expect(keeping).toHaveBeenCalledTimes(1), {
      timeout: 2000,
    });

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));

    await waitFor(() => expect(throwingAway).toHaveBeenCalledWith(7));
    await waitFor(() => expect(onCancel).toHaveBeenCalled());
  });

  it("asks before leaving a draft opened again that carries nothing but files", async () => {
    // Nothing typed, and a file in it: leaving it unasked throws the draft away, and the file with
    // it.
    reopening.mockResolvedValue([INVOICE]);
    open({ ...NOTHING_TO_ANSWER, draftId: 9 });
    expect(await screen.findByText("invoice.pdf")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));

    expect(
      screen.queryByRole("button", { name: "Save as draft" }),
      "it left without asking",
    ).not.toBeNull();
    await act(async () => {});
    expect(
      throwingAway,
      "the draft was thrown away unasked",
    ).not.toHaveBeenCalled();
  });

  it("asks before leaving a message carrying nothing but a file", async () => {
    attach.mockResolvedValue({
      token: "t1",
      name: "invoice.pdf",
      bytes: "2500000",
    });
    keeping.mockResolvedValue(7);
    renderComposer([account("acc-1", "zeta@example.com", true)]);
    fireEvent.click(
      await screen.findByRole("button", { name: /attach a file/i }),
    );
    await waitFor(() => expect(keeping).toHaveBeenCalled(), { timeout: 2000 });

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));

    expect(
      screen.queryByRole("button", { name: "Save as draft" }),
      "it left without asking",
    ).not.toBeNull();
    await act(async () => {});
    expect(
      throwingAway,
      "the message was thrown away unasked",
    ).not.toHaveBeenCalled();
  });

  it("writes nothing down again while a message is being thrown away", async () => {
    // The wait the last keystroke started runs out while the draft is being thrown away. A keep
    // then finds no number to keep under, and writes the message down again as a draft of its
    // own, the file with it.
    attach.mockResolvedValue({
      token: "t1",
      name: "invoice.pdf",
      bytes: "2500000",
    });
    keeping.mockResolvedValue(7);
    throwingAway.mockReturnValue(new Promise(() => {}));
    renderComposer([account("acc-1", "zeta@example.com", true)]);
    fireEvent.click(
      await screen.findByRole("button", { name: /attach a file/i }),
    );
    expect(await screen.findByText("invoice.pdf")).toBeTruthy();
    type(/^Message$/, "never mind");

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(throwingAway).toHaveBeenCalledWith(7));
    await longerThanAKeepWaits();

    expect(
      keeping.mock.calls,
      "the message thrown away was written down again",
    ).toEqual([
      [
        "acc-1",
        null,
        expect.objectContaining({ body: "never mind", files: ["t1"] }),
      ],
    ]);
  });

  it("writes nothing down when the files of a draft being thrown away come back meanwhile", async () => {
    // Back while the draft is being thrown away, they would start the wait before a keep, and the
    // keep would write the draft down again as one of its own, every file with it.
    let back: (files: (typeof INVOICE)[]) => void = () => {};
    reopening.mockReturnValue(
      new Promise((resolve) => {
        back = resolve;
      }),
    );
    throwingAway.mockReturnValue(new Promise(() => {}));
    keeping.mockResolvedValue(9);
    open({ ...NOTHING_TO_ANSWER, draftId: 9 });

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(await screen.findByRole("button", { name: "Discard" }));
    await waitFor(() => expect(throwingAway).toHaveBeenCalledWith(9));
    await act(async () => back([INVOICE]));
    await longerThanAKeepWaits();

    expect(
      keeping,
      "the draft thrown away was written down again",
    ).not.toHaveBeenCalled();
  });
});

describe("leaving a message half written", () => {
  it("goes without a word when nothing was written", async () => {
    // Nothing to decide about, so nothing to ask. A question over an empty form is a press a
    // person has to answer for having opened a window.
    const { onCancel } = renderComposer([
      account("acc-1", "zeta@example.com", true),
    ]);

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));

    await waitFor(() => expect(onCancel).toHaveBeenCalled());
    expect(keeping).not.toHaveBeenCalled();
  });

  it("throws away what was written when that is the answer", async () => {
    // It used to keep every one of them, so Drafts filled with messages nobody meant to write.
    keeping.mockResolvedValue(7);
    const { onCancel } = renderComposer([
      account("acc-1", "zeta@example.com", true),
    ]);
    type(/^Message$/, "never mind");

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));

    // Written down first, because the last thing typed is inside a wait that has not run yet --
    // and then the one the writing produced is the one thrown away.
    await waitFor(() => expect(throwingAway).toHaveBeenCalledWith(7));
    await waitFor(() => expect(onCancel).toHaveBeenCalled());
  });

  it("goes on writing into the same draft when it could not be thrown away", async () => {
    // Still on the disk, so still the draft this window writes into. A keep under no number
    // would write the message down a second time beside it.
    keeping.mockResolvedValue(7);
    throwingAway.mockRejectedValue({
      code: "storageUnavailable",
      message: "",
      path: null,
    });
    const { onCancel } = renderComposer([
      account("acc-1", "zeta@example.com", true),
    ]);
    type(/^Message$/, "never mind");

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    expect(await screen.findByRole("alert")).toBeTruthy();
    type(/^Message$/, "never mind, then");

    await waitFor(
      () =>
        expect(keeping).toHaveBeenLastCalledWith(
          "acc-1",
          7,
          expect.objectContaining({ body: "never mind, then" }),
        ),
      { timeout: 2000 },
    );
    expect(
      keeping.mock.calls.filter(([, under]) => under === null),
      "the message was written down a second time beside its draft",
    ).toHaveLength(1);
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("keeps one draft when it is closed while a keep is still on its way", async () => {
    // Kept on the way out under no number while the timer's keep was still out, the message was
    // written down twice, and the folder held a copy nobody made on purpose.
    let answer: (id: number) => void = () => {};
    keeping.mockReturnValueOnce(
      new Promise<number>((resolve) => {
        answer = resolve;
      }),
    );
    keeping.mockResolvedValue(9);
    const { onCancel } = renderComposer([
      account("acc-1", "zeta@example.com", true),
    ]);
    type(/^Message$/, "about Tuesday");
    await waitFor(() => expect(keeping).toHaveBeenCalledTimes(1), {
      timeout: 2000,
    });

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Save as draft" }));
    await act(async () => answer(9));

    await waitFor(() => expect(onCancel).toHaveBeenCalled());
    expect(keeping).toHaveBeenCalledTimes(2);
    expect(keeping).toHaveBeenLastCalledWith("acc-1", 9, expect.anything());
  });

  it("takes nothing typed while it is on its way out", async () => {
    // Typed after Save as draft, while the keep is on its way, it is in nothing that keep writes,
    // and the window closes over it without a word. Sent then, a message goes while it is being
    // kept, or thrown away.
    let refuse: (reason: unknown) => void = () => {};
    keeping.mockReturnValue(
      new Promise<number>((_, reject) => {
        refuse = reject;
      }),
    );
    const { onCancel } = renderComposer([
      account("acc-1", "zeta@example.com", true),
    ]);
    type(/^To$/, "kappa@example.net");
    type(/^Message$/, "one last thought");
    const message = () =>
      screen.getByLabelText(/^Message$/) as HTMLTextAreaElement;
    const send = () =>
      screen.getByRole("button", { name: /^Send$/ }) as HTMLButtonElement;

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Save as draft" }));
    await waitFor(() => expect(keeping).toHaveBeenCalled());

    expect(message().disabled, "the message took changes on its way out").toBe(
      true,
    );
    expect(send().disabled, "the message could be sent on its way out").toBe(
      true,
    );

    await act(async () =>
      refuse({ code: "storageUnavailable", message: "", path: null }),
    );

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(onCancel).not.toHaveBeenCalled();
    expect(
      message().disabled,
      "the window stayed over what it could not keep, and took nothing more",
    ).toBe(false);
    expect(send().disabled).toBe(false);
    expect(message().value).toBe("one last thought");
  });

  it("stays where it was when the question is itself cancelled", () => {
    const { onCancel } = renderComposer([
      account("acc-1", "zeta@example.com", true),
    ]);
    type(/^Message$/, "still thinking");

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(onCancel).not.toHaveBeenCalled();
    expect(
      (screen.getByLabelText(/^Message$/) as HTMLTextAreaElement).value,
    ).toBe("still thinking");
  });

  it("goes on to what the screen opens in its place only once it has gone", async () => {
    // A new message asked for over this one is handed to the way out, to open once this window
    // has gone. Opened any earlier, it would stand over a question nobody answered, or over a
    // keep that has not landed and may yet fail.
    let letItFinish: (id: number) => void = () => {};
    keeping.mockImplementation(
      () =>
        new Promise<number>((settle) => {
          letItFinish = settle;
        }),
    );
    const { onCancel, leaveFor } = renderComposer([
      account("acc-1", "zeta@example.com", true),
    ]);
    type(/^Message$/, "one last thought");
    const next = vi.fn();

    leaveFor(next);
    expect(
      next,
      "it went on before the question was answered",
    ).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Save as draft" }));
    await waitFor(() => expect(keeping).toHaveBeenCalled());
    expect(next, "it went on before the keep landed").not.toHaveBeenCalled();

    await act(async () => letItFinish(1));

    await waitFor(() => expect(next).toHaveBeenCalledTimes(1));
    expect(onCancel).toHaveBeenCalledTimes(1);
    // The screen is told the window has gone before it opens another in its place: told after,
    // it would close the one it had just opened.
    expect(onCancel.mock.invocationCallOrder[0]).toBeLessThan(
      next.mock.invocationCallOrder[0],
    );
  });

  it("opens nothing in its place by a later way out once the question was put away", async () => {
    const { onCancel, leaveFor } = renderComposer([
      account("acc-1", "zeta@example.com", true),
    ]);
    type(/^Message$/, "still thinking");
    const next = vi.fn();

    leaveFor(next);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Save as draft" }));

    await waitFor(() => expect(onCancel).toHaveBeenCalledTimes(1));
    expect(
      next,
      "a message somebody stopped asking for opened",
    ).not.toHaveBeenCalled();
  });

  it("opens nothing in its place by a later way out once a keep it could not make kept it open", async () => {
    keeping.mockRejectedValue({
      code: "invalidRequest",
      message: "",
      path: null,
    });
    const { onCancel, leaveFor } = renderComposer([
      account("acc-1", "zeta@example.com", true),
    ]);
    type(/^Message$/, "still thinking");
    const next = vi.fn();

    leaveFor(next);
    fireEvent.click(screen.getByRole("button", { name: "Save as draft" }));
    expect(await screen.findByRole("alert")).toBeTruthy();
    keeping.mockResolvedValue(1);
    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Save as draft" }));

    await waitFor(() => expect(onCancel).toHaveBeenCalledTimes(1));
    expect(
      next,
      "a message somebody stopped asking for opened",
    ).not.toHaveBeenCalled();
  });
});

describe("who a message may be sent to", () => {
  it("will not send to something that is not an address", () => {
    // Any non-empty text used to do: a half-typed name went to the server, which refused it after
    // the wait, and what came back was a failure about a message that could never have been sent.
    renderComposer([account("acc-1", "zeta@example.com", true)]);

    type(/^To$/, "kappa");

    expect(
      (screen.getByRole("button", { name: /^Send$/ }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);

    type(/^To$/, "kappa@example.net");

    expect(
      (screen.getByRole("button", { name: /^Send$/ }) as HTMLButtonElement)
        .disabled,
    ).toBe(false);
  });

  it("puts the copy fields away again, and what was typed in them with it", () => {
    // Two fields that appear on a press and cannot be put away are two empty fields in every
    // message somebody wrote after pressing it once -- and a recipient nobody can see.
    renderComposer([account("acc-1", "zeta@example.com", true)]);

    fireEvent.click(screen.getByRole("button", { name: "Add copies" }));
    type(/^Copy to$/, "kappa@example.net");

    fireEvent.click(
      screen.getByRole("button", { name: "Remove the copy fields" }),
    );

    expect(screen.queryByLabelText(/^Copy to$/)).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Add copies" }));
    expect((screen.getByLabelText(/^Copy to$/) as HTMLInputElement).value).toBe(
      "",
    );
  });
});

describe("Composer, what the core holds for when the application ends", () => {
  // Quitting takes none of the ways out: the keep the timer is waiting to make never runs, and the
  // window cannot be asked anything once the end has begun. What the core writes down then is the
  // newest note this window gave it, so the note has to follow what is on screen, and go when the
  // message stops being written.
  const ZETA = account("acc-1", "zeta@example.com", true);
  /** The name the newest note was given, which is the composer's own. */
  const lastNamed = () => noting.mock.lastCall?.[2].writing;
  /** Where in the order of every call to the core the first call to `mock` came. */
  const first = (mock: { mock: { invocationCallOrder: number[] } }) =>
    mock.mock.invocationCallOrder[0] ?? Number.NaN;
  /** Where in that order the last call to `mock` came. */
  const last = (mock: { mock: { invocationCallOrder: number[] } }) =>
    mock.mock.invocationCallOrder[mock.mock.invocationCallOrder.length - 1] ??
    Number.NaN;
  const sendButton = () => screen.getByRole("button", { name: /^Send$/ });

  it("notes what is on screen as it changes, under the number it is kept under", async () => {
    keeping.mockResolvedValue(9);
    renderComposer([ZETA]);

    type(/^To$/, "kappa@example.net");
    type(/^Message$/, "about Tuesday");

    // At once, and not a moment after the typing stops: the key that quits comes after the last
    // change, and a note still waiting for the typing to stop would still be waiting then.
    expect(keeping, "the keep was made before the typing stopped").not.toHaveBeenCalled();
    expect(noting).toHaveBeenLastCalledWith(
      "acc-1",
      null,
      expect.objectContaining({
        recipients: "kappa@example.net",
        body: "about Tuesday",
        files: [],
      }),
    );

    await waitFor(() => expect(keeping).toHaveBeenCalledTimes(1), {
      timeout: 2000,
    });
    await act(async () => {});
    type(/^Message$/, "about Tuesday, and Wednesday");

    expect(noting).toHaveBeenLastCalledWith(
      "acc-1",
      9,
      expect.objectContaining({ body: "about Tuesday, and Wednesday" }),
    );
  });

  it("notes and keeps under a name of its own, which the next composer does not share", async () => {
    // What ties a note to the keeps of the same message in the core, so the end writes it down
    // once, and what keeps one composer from letting go of another's note.
    const { unmount } = renderComposer([ZETA]);
    type(/^Message$/, "about Tuesday");
    await waitFor(() => expect(keeping).toHaveBeenCalledTimes(1), {
      timeout: 2000,
    });
    const named = lastNamed();
    unmount();

    renderComposer([ZETA]);
    type(/^Message$/, "about Wednesday");

    expect(named).toMatch(/^[0-9a-f]{32}$/);
    expect(keeping.mock.calls[0]?.[2].writing).toBe(named);
    expect(lastNamed()).toMatch(/^[0-9a-f]{32}$/);
    expect(lastNamed(), "two composers went by one name").not.toBe(named);
  });

  it("notes a draft opened again under its own number, and only once its files are back", async () => {
    // Before then nothing can be kept over it: a keep naming none of its files takes every one of
    // them off it, and so would the end.
    let back: (files: AttachingDto[]) => void = () => {};
    reopening.mockReturnValue(
      new Promise((resolve) => {
        back = resolve;
      }),
    );
    renderComposer([ZETA], {
      ...NOTHING_TO_ANSWER,
      draftId: 4,
      to: "kappa@example.net",
      subject: "About Tuesday",
      body: "half a thought",
    });
    await act(async () => {});

    expect(noting, "noted before its files were back").not.toHaveBeenCalled();

    await act(async () =>
      back([{ token: "r1", name: "invoice.pdf", bytes: "2500000" }]),
    );

    expect(noting).toHaveBeenLastCalledWith(
      "acc-1",
      4,
      expect.objectContaining({ body: "half a thought", files: ["r1"] }),
    );
  });

  it("notes nothing the keep would not write down, and lets go of a note once it is that", async () => {
    // A message never kept with nothing in it is no draft at all, and neither is one with no
    // account to send it from.
    renderComposer([ZETA]);
    await screen.findByLabelText(/^To$/);
    expect(noting, "an empty message was noted").not.toHaveBeenCalled();

    type(/^Subject$/, "About Tuesday");
    const named = lastNamed();
    type(/^Subject$/, "");

    expect(noting).toHaveBeenCalledTimes(1);
    expect(forgetting).toHaveBeenCalledWith(named);
  });

  it("notes nothing from a window with no account to send from", async () => {
    renderComposer([account("acc-1", "zeta@example.com", false)], {
      ...NOTHING_TO_ANSWER,
      to: "kappa@example.net",
      subject: "Re: About Tuesday",
      body: "> Shall we?",
    });
    await act(async () => {});

    expect(noting).not.toHaveBeenCalled();
  });

  it("notes nothing while the message goes out, and leaves the note held until the outbox has it", async () => {
    // The core lets go of it in the same step as the outbox takes the message, by the name the
    // message carries. Let go of here first, an application ending while the message was handed
    // over would find nothing to write down, and the message would be in neither place.
    let landed: (id: number) => void = () => {};
    keeping.mockReturnValueOnce(
      new Promise<number>((resolve) => {
        landed = resolve;
      }),
    );
    let taken: (name: string) => void = () => {};
    sending.mockReturnValueOnce(
      new Promise<string>((resolve) => {
        taken = resolve;
      }),
    );
    const { onWritten } = renderComposer([ZETA]);
    type(/^To$/, "kappa@example.net");
    type(/^Message$/, "about Tuesday");
    await waitFor(() => expect(keeping).toHaveBeenCalledTimes(1), {
      timeout: 2000,
    });
    const named = lastNamed();
    const notes = noting.mock.calls.length;

    fireEvent.click(sendButton());
    await act(async () => landed(9));
    await waitFor(() => expect(sending).toHaveBeenCalled());

    expect(sending.mock.calls[0]?.[1].writing).toBe(named);
    expect(
      forgetting,
      "let go of before the outbox had the message",
    ).not.toHaveBeenCalled();

    await act(async () => taken("<one@example.com>"));
    await waitFor(() => expect(onWritten).toHaveBeenCalled());

    expect(noting.mock.calls.length, "noted while it was going out").toBe(notes);
  });

  it("keeps the note of a message that could not be sent, and goes on noting it", async () => {
    // Refused, it is still being written: the core still holds its note, and what is typed next
    // is noted like anything else.
    sending.mockRejectedValue({
      code: "invalidRequest",
      message: "",
      path: null,
    });
    renderComposer([ZETA]);
    type(/^To$/, "kappa@example.net");
    type(/^Message$/, "about Tuesday");

    fireEvent.click(sendButton());
    await screen.findByRole("alert");
    type(/^Message$/, "about Tuesday, or Wednesday");

    expect(forgetting).not.toHaveBeenCalled();
    expect(noting).toHaveBeenLastCalledWith(
      "acc-1",
      null,
      expect.objectContaining({ body: "about Tuesday, or Wednesday" }),
    );
  });

  it("lets go of the note as soon as the message is thrown away, before the draft goes", async () => {
    // Written by the end in the moment between, a message somebody threw away would come back.
    keeping.mockResolvedValue(4);
    renderComposer([ZETA], {
      ...NOTHING_TO_ANSWER,
      draftId: 4,
      to: "kappa@example.net",
      subject: "About Tuesday",
      body: "half a thought",
    });
    await opened();
    type(/^Message$/, "half a thought, never mind");
    const named = lastNamed();

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(throwingAway).toHaveBeenCalledWith(4));

    expect(forgetting).toHaveBeenCalledWith(named);
    expect(first(forgetting)).toBeLessThan(first(throwingAway));
    expect(
      last(noting),
      "noted again while it was being thrown away",
    ).toBeLessThan(first(forgetting));
  });

  it("lets go of the note once the way out has kept what is on screen, and not before", async () => {
    // Until that keep has landed, the note is the only place the last of the typing is.
    let landed: (id: number) => void = () => {};
    keeping.mockImplementation(
      () =>
        new Promise<number>((resolve) => {
          landed = resolve;
        }),
    );
    const { onCancel } = renderComposer([ZETA]);
    type(/^Message$/, "one last thought");

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Save as draft" }));
    await waitFor(() => expect(keeping).toHaveBeenCalled());
    expect(
      forgetting,
      "let go of while the keep was on its way",
    ).not.toHaveBeenCalled();

    await act(async () => landed(1));
    await waitFor(() => expect(onCancel).toHaveBeenCalled());

    expect(forgetting).toHaveBeenCalledTimes(1);
    expect(first(forgetting)).toBeLessThan(first(onCancel));
  });

  it("notes what is on screen again when it could not be thrown away, since it is still being written", async () => {
    // Let go of as the throwing away began, and the draft is still there: without a note, the
    // end would keep nothing typed since its last keep.
    keeping.mockResolvedValue(4);
    throwingAway.mockRejectedValue({
      code: "storageUnavailable",
      message: "",
      path: null,
    });
    const { onCancel } = renderComposer([ZETA], {
      ...NOTHING_TO_ANSWER,
      draftId: 4,
      to: "kappa@example.net",
      subject: "About Tuesday",
      body: "half a thought",
    });
    await opened();
    type(/^Message$/, "half a thought, never mind");

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    await screen.findByRole("alert");

    // Noted once the window is drawn again, which is after the alert is on screen.
    await waitFor(() =>
      expect(last(noting)).toBeGreaterThan(
        first(forgetting),
      ),
    );
    expect(onCancel).not.toHaveBeenCalled();
    expect(noting).toHaveBeenLastCalledWith(
      "acc-1",
      4,
      expect.objectContaining({ body: "half a thought, never mind" }),
    );
  });

  it("lets go of the note when it is taken off the screen", async () => {
    const { unmount } = renderComposer([ZETA]);
    type(/^Message$/, "about Tuesday");
    const named = lastNamed();

    unmount();

    expect(forgetting).toHaveBeenCalledWith(named);
  });
});
