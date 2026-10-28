import { describe, expect, it } from "vitest";

import {
  answerSubject,
  answerTo,
  forwardOf,
  forwardSubject,
  forwardWholeOf,
} from "./message";
import { createTranslator } from "@/shared/i18n";
import type { ForwardDto } from "@/shared/ipc/generated/ForwardDto";
import type { MessageDetailsDto } from "@/shared/ipc/generated/MessageDetailsDto";

function details(over: Partial<MessageDetailsDto> = {}): MessageDetailsDto {
  return {
    messageId: "42",
    subject: "About Tuesday",
    sender: "Zeta Quorin",
    senderAddress: "zeta@example.com",
    receivedAt: 1_700_000_000,
    size: 2048,
    unread: false,
    bodyState: "headersOnly",
    accountId: "acc-1",
    addressedToMore: false,
    named: "first@example.com",
    chain: ["older@example.com"],
    placements: [],
    ...over,
  };
}

describe("what an answer is called", () => {
  it("says what it is answering", () => {
    expect(answerSubject("About Tuesday")).toBe("Re: About Tuesday");
  });

  it("does not stack a second Re: on a message that has already been round", () => {
    expect(answerSubject("Re: About Tuesday")).toBe("Re: About Tuesday");
  });

  it("collapses a whole stack of them, however it was written", () => {
    expect(answerSubject("Re: Re:   RE: About Tuesday")).toBe(
      "Re: About Tuesday",
    );
    expect(answerSubject("Re[2]: About Tuesday")).toBe("Re: About Tuesday");
  });

  it.each([
    ["Aw: About Tuesday", "German"],
    ["Antw: About Tuesday", "Dutch"],
    ["Sv: About Tuesday", "Swedish"],
    ["Ответ: About Tuesday", "Russian"],
  ])("collapses %s, which is the same prefix in %s", (written) => {
    // A thread that crossed one of those clients comes back with its prefix, and answering it
    // would otherwise start a second stack beside the first.
    expect(answerSubject(written)).toBe("Re: About Tuesday");
  });

  it("answers a message with no subject at all", () => {
    expect(answerSubject(null)).toBe("Re:");
    expect(answerSubject("   ")).toBe("Re:");
    // A subject that is nothing but the prefix is a subject that says nothing.
    expect(answerSubject("Re:")).toBe("Re:");
  });

  it("leaves a subject that only looks like a prefix alone", () => {
    expect(answerSubject("Reading week")).toBe("Re: Reading week");
    expect(answerSubject("Sverige")).toBe("Re: Sverige");
  });
});

/** Who the core said the answer goes to. Deciding that is its job, not this module's. */
function audience(to: string[], cc: string[] = [], quoted = "") {
  return { to, cc, quoted };
}

/// A translator and a date format standing in for the interface's own.
const words = ((key: string, values?: Record<string, unknown>) =>
  key === "compose.answering"
    ? `On ${values?.when}, ${values?.who} wrote:`
    : key === "compose.answeringUnknown"
      ? `On ${values?.when}, an unknown sender wrote:`
      : key) as never;
const dates = new Intl.DateTimeFormat("en", { dateStyle: "full" });

describe("what an answer carries", () => {
  it("takes the name and the chain from the message, and the audience from the core", () => {
    expect(
      answerTo(details(), audience(["zeta@example.com"]), words, dates),
    ).toEqual({
      to: "zeta@example.com",
      cc: "",
      subject: "Re: About Tuesday",
      answering: "first@example.com",
      chain: ["older@example.com"],
      fromAccount: "acc-1",
      body: "",
    });
  });

  it("writes a room out as one line each, which is what the fields hold", () => {
    expect(
      answerTo(
        details(),
        audience(
          ["zeta@example.com", "kappa@example.net"],
          ["sigma@example.org"],
        ),
        words,
        dates,
      ),
    ).toMatchObject({
      to: "zeta@example.com, kappa@example.net",
      cc: "sigma@example.org",
    });
  });

  it("carries no name for a message that arrived without one", () => {
    // Nothing to point `In-Reply-To` at. The message is still answerable — it just cannot be
    // answered into a thread, and Rust writes neither header rather than one pointing at nothing.
    expect(
      answerTo(
        details({ named: null, chain: [] }),
        audience(["zeta@example.com"]),
        words,
        dates,
      ),
    ).toEqual({
      to: "zeta@example.com",
      cc: "",
      subject: "Re: About Tuesday",
      answering: null,
      chain: [],
      fromAccount: "acc-1",
      body: "",
    });
  });

  it("starts an answer with room to write, then who is answered, then their message", () => {
    // Room first, and the cursor with it. An answer written under a quotation is one most people
    // never see the beginning of.
    const started = answerTo(
      details(),
      audience(["zeta@example.com"], [], "> what they wrote\n"),
      words,
      dates,
    ).body;

    expect(started.startsWith("\n\n")).toBe(true);
    expect(started).toContain("wrote:");
    expect(started.endsWith("> what they wrote\n")).toBe(true);
    expect(started.indexOf("wrote:")).toBeLessThan(
      started.indexOf("> what they wrote"),
    );
  });

  it("says an unknown sender wrote when the message names nobody, rather than putting a label where a name goes", () => {
    const started = answerTo(
      details({ sender: null, senderAddress: null }),
      audience(["zeta@example.com"], [], "> what they wrote\n"),
      words,
      dates,
    ).body;

    expect(started).toContain("an unknown sender wrote:");
    expect(started).not.toContain("list.unknownSender");
  });

  it("starts empty when there is nothing to quote", () => {
    // A message this device does not hold, or one with no text part. An answer that opened with a
    // line about a quotation that is not there would be saying something untrue.
    expect(
      answerTo(details(), audience(["zeta@example.com"]), words, dates).body,
    ).toBe("");
  });

  it("addresses nobody when the core could find nobody to address", () => {
    // Better an answer somebody finishes addressing than one that cannot be sent and does not say
    // why. A message stored before the address had a column of its own is the ordinary case.
    expect(
      answerTo(details({ senderAddress: null }), audience([]), words, dates).to,
    ).toBe("");
  });
});

describe("what a message passed on is called", () => {
  it("says it is passed on", () => {
    expect(forwardSubject("About Tuesday")).toBe("Fwd: About Tuesday");
  });

  it("does not stack a second Fwd: on a message passed on before, however it was written", () => {
    expect(forwardSubject("Fwd: About Tuesday")).toBe("Fwd: About Tuesday");
    expect(forwardSubject("FW: About Tuesday")).toBe("Fwd: About Tuesday");
    expect(forwardSubject("Fwd: fw:  FWD : About Tuesday")).toBe(
      "Fwd: About Tuesday",
    );
  });

  it("keeps an answer's prefix, which says something else", () => {
    expect(forwardSubject("Re: About Tuesday")).toBe("Fwd: Re: About Tuesday");
  });

  it("passes on a message with no subject at all", () => {
    expect(forwardSubject(null)).toBe("Fwd:");
    expect(forwardSubject("   ")).toBe("Fwd:");
    expect(forwardSubject("Fwd:")).toBe("Fwd:");
  });

  it("leaves a subject that only begins like the prefix alone", () => {
    expect(forwardSubject("Fwd from the field")).toBe(
      "Fwd: Fwd from the field",
    );
    expect(forwardSubject("Fw drafts")).toBe("Fwd: Fw drafts");
  });
});

/** What the core read of the message to pass it on, with nothing in it unless asked. */
function passedOn(over: Partial<ForwardDto> = {}): ForwardDto {
  return { to: [], cc: [], text: "", files: [], leftBehind: [], ...over };
}

describe("what a message passed on carries", () => {
  // The interface's own English, and dates in one zone wherever the test runs. The date line is
  // expected as this formatter writes it: how a machine's formatter spaces a time is not this
  // module's to say.
  const english = createTranslator("en");
  const utc = new Intl.DateTimeFormat("en", {
    dateStyle: "full",
    timeStyle: "short",
    timeZone: "UTC",
  });

  it("goes to nobody yet, answers nothing, and is written from the account the message came to", () => {
    // Who it goes to now is the person's to say, and it starts a conversation of its own with
    // whoever receives it rather than joining one they never saw.
    const forward = forwardOf(details(), passedOn(), english, utc);

    expect(forward).toMatchObject({
      to: "",
      cc: "",
      subject: "Fwd: About Tuesday",
      answering: null,
      chain: [],
      fromAccount: "acc-1",
    });
  });

  it("carries the files the core holds for it, and names the ones left behind", () => {
    const files = [
      { token: "t1", name: "invoice.pdf", bytes: "78" },
      { token: "t2", name: "photo.jpg", bytes: "331" },
    ];

    const forward = forwardOf(
      details(),
      passedOn({ files, leftBehind: ["video.mov"] }),
      english,
      utc,
    );

    expect(forward.files).toEqual(files);
    expect(forward.leftBehind).toEqual(["video.mov"]);
  });

  it("starts with room to write, then whose message it is, then the message", () => {
    const forward = forwardOf(
      details(),
      passedOn({
        to: ["kappa@example.net", "sigma@example.org"],
        cc: ["you@example.com"],
        text: "Shall we?\nZeta",
      }),
      english,
      utc,
    );

    expect(forward.body).toBe(
      [
        "",
        "",
        "---------- Forwarded message ----------",
        "From: Zeta Quorin <zeta@example.com>",
        `Date: ${utc.format(new Date(1_700_000_000 * 1000))}`,
        "Subject: About Tuesday",
        "To: kappa@example.net, sigma@example.org",
        "Cc: you@example.com",
        "",
        "Shall we?",
        "Zeta",
      ].join("\n"),
    );
  });

  it.each([
    [{ sender: null }, "From: zeta@example.com"],
    [{ senderAddress: null }, "From: Zeta Quorin"],
    [{ sender: "ZETA@example.com" }, "From: zeta@example.com"],
  ])("names the writer by what the message gave: %o", (over, line) => {
    const body = forwardOf(details(over), passedOn(), english, utc).body;

    expect(body.split("\n")).toContain(line);
  });

  it("marks off what the message wrote in a block that runs right to left, in the text that is sent", () => {
    // The block goes out inside the message and is read in somebody else's mail app. A name and
    // an address dropped into an Arabic line unmarked come out with their brackets turned round.
    const body = forwardOf(
      details(),
      passedOn({ to: ["kappa@example.net"] }),
      createTranslator("ar"),
      utc,
    ).body;

    expect(body).toContain("\u2068Zeta Quorin <zeta@example.com>\u2069");
    expect(body).toContain("\u2068kappa@example.net\u2069");
    expect(body).toContain("\u2068About Tuesday\u2069");
  });

  it("leaves out a line the message has nothing for, rather than writing it empty", () => {
    const body = forwardOf(
      details({ sender: null, senderAddress: null, subject: null }),
      passedOn({ text: "Shall we?" }),
      english,
      utc,
    ).body;

    expect(body).toBe(
      [
        "",
        "",
        "---------- Forwarded message ----------",
        `Date: ${utc.format(new Date(1_700_000_000 * 1000))}`,
        "",
        "Shall we?",
      ].join("\n"),
    );
  });
});

describe("what a message passed on whole carries", () => {
  it("is the message as one file under the subject it is passed on under, and nothing written", () => {
    // No lines say whose it is: the copy says that itself, and anything written here is what the
    // person adds above it.
    const file = { token: "t1", name: "About Tuesday.eml", bytes: "4096" };

    expect(
      forwardWholeOf(details(), passedOn({ files: [file], text: "ignored" })),
    ).toEqual({
      to: "",
      cc: "",
      subject: "Fwd: About Tuesday",
      body: "",
      answering: null,
      chain: [],
      fromAccount: "acc-1",
      files: [file],
      leftBehind: [],
    });
  });

  it("names a message too heavy to go whole as left behind", () => {
    expect(
      forwardWholeOf(
        details(),
        passedOn({ leftBehind: ["About Tuesday.eml"] }),
      ).leftBehind,
    ).toEqual(["About Tuesday.eml"]);
  });
});
