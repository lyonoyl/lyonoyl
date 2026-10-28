import { describe, expect, it } from "vitest";

import {
  asFarAsMailboxes,
  BEFORE_SAYING_ANYTHING,
  ONCE_SAID,
  worthSaying,
} from "./steady";

describe("worthSaying", () => {
  it("says nothing about work that has only just begun", () => {
    // A round over nine quiet mailboxes can be shorter than the eye takes to move, and a line that
    // appears and goes in that time is read as a fault rather than as progress.
    expect(worthSaying({ busy: true, since: 0, saidAt: null }, 10)).toBe(false);
    expect(
      worthSaying(
        { busy: true, since: 0, saidAt: null },
        BEFORE_SAYING_ANYTHING - 1,
      ),
    ).toBe(false);
  });

  it("says it once the work has lasted long enough to be worth saying", () => {
    expect(
      worthSaying(
        { busy: true, since: 0, saidAt: null },
        BEFORE_SAYING_ANYTHING,
      ),
    ).toBe(true);
  });

  it("keeps saying it for a while after the work is done", () => {
    // Otherwise the end of a round blinks the line out a moment after it appeared.
    expect(worthSaying({ busy: false, since: null, saidAt: 1_000 }, 1_100)).toBe(
      true,
    );
    expect(
      worthSaying(
        { busy: false, since: null, saidAt: 1_000 },
        1_000 + ONCE_SAID - 1,
      ),
    ).toBe(true);
  });

  it("stops saying it once it has been up long enough", () => {
    expect(
      worthSaying({ busy: false, since: null, saidAt: 1_000 }, 1_000 + ONCE_SAID),
    ).toBe(false);
  });

  it("does not announce work after the fact that it never announced", () => {
    // Work that went by under the threshold is not worth mentioning once it is over: that would
    // be the same flicker with an extra step in it.
    expect(worthSaying({ busy: false, since: null, saidAt: null }, 9_999)).toBe(
      false,
    );
  });

  it("says nothing when there is no beginning to measure from", () => {
    expect(worthSaying({ busy: true, since: null, saidAt: null }, 9_999)).toBe(
      false,
    );
  });
});

describe("asFarAsMailboxes", () => {
  const nine = { done: 0, asked: 9, through: 0, of: 9000 };

  it("moves with the folders rather than waiting for a mailbox to finish", () => {
    // Every mailbox is read at the same moment as every other, so counting finished ones leaves
    // the number at nought for five seconds and then races it through every value it has.
    expect(asFarAsMailboxes({ ...nine, through: 0 })).toBe(0);
    expect(asFarAsMailboxes({ ...nine, through: 1000 })).toBe(1);
    expect(asFarAsMailboxes({ ...nine, through: 3000 })).toBe(3);
    expect(asFarAsMailboxes({ ...nine, through: 5000 })).toBe(5);
  });

  it("never reaches the total while anything is still being read", () => {
    // Nine of nine beside a mailbox still being read is the one reading a person would act on.
    expect(asFarAsMailboxes({ ...nine, through: 8833 })).toBe(8);
    expect(asFarAsMailboxes({ ...nine, through: 9000 })).toBe(8);
  });

  it("says the total once everything really has finished", () => {
    expect(
      asFarAsMailboxes({ done: 9, asked: 9, through: 9000, of: 9000 }),
    ).toBe(9);
  });

  it("falls back to finished mailboxes before any folder count arrives", () => {
    expect(
      asFarAsMailboxes({ done: 2, asked: 9, through: 0, of: 0 }),
    ).toBe(2);
  });

  it("says nothing about nothing", () => {
    expect(
      asFarAsMailboxes({ done: 0, asked: 0, through: 0, of: 0 }),
    ).toBe(0);
  });
});
