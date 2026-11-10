import { describe, expect, it } from "vitest";

import { afterDrop, END, landingOn, type Placing } from "./dropping";

const account = (id: string) => ({ kind: "account" as const, id });
const group = (id: string) => ({ kind: "group" as const, id });

describe("landingOn", () => {
  it("puts a mailbox into a group dropped on the middle of its row", () => {
    const work = { kind: "group" as const, id: "g1", open: true };

    expect(landingOn(work, account("acc-1"), 0.5)).toEqual({
      id: "g1",
      where: "into",
    });
    // A drag the window saw no pointer for lands where a person aims at a group.
    expect(landingOn(work, account("acc-1"), null)).toEqual({
      id: "g1",
      where: "into",
    });
  });

  it("puts a mailbox above a group from the top edge of its row", () => {
    const work = { kind: "group" as const, id: "g1", open: false };

    expect(landingOn(work, account("acc-1"), 0.1)).toEqual({
      id: "g1",
      where: "before",
    });
  });

  it("puts a mailbox below a folded group from the bottom edge, and into an open one", () => {
    // Below an open group's row is its first mailbox, so that edge is already inside it.
    expect(
      landingOn({ kind: "group", id: "g1", open: false }, account("a"), 0.9),
    ).toEqual({ id: "g1", where: "after" });
    expect(
      landingOn({ kind: "group", id: "g1", open: true }, account("a"), 0.9),
    ).toEqual({ id: "g1", where: "into" });
  });

  it("puts a mailbox above or below another by the half of the row the pointer is on", () => {
    const row = { kind: "account" as const, id: "acc-2", groupId: "g1" };

    expect(landingOn(row, account("acc-1"), 0.2)).toEqual({
      id: "acc-2",
      where: "before",
    });
    expect(landingOn(row, account("acc-1"), 0.8)).toEqual({
      id: "acc-2",
      where: "after",
    });
    expect(landingOn(row, account("acc-1"), null)).toEqual({
      id: "acc-2",
      where: "before",
    });
  });

  it("takes nothing on the row being carried", () => {
    expect(
      landingOn(
        { kind: "account", id: "acc-1", groupId: null },
        account("acc-1"),
        0.5,
      ),
    ).toBeNull();
    expect(
      landingOn({ kind: "group", id: "g1", open: true }, group("g1"), 0.5),
    ).toBeNull();
  });

  it("never puts a group inside another group", () => {
    // Not on a mailbox in one, and not into one from its row.
    expect(
      landingOn(
        { kind: "account", id: "acc-1", groupId: "g2" },
        group("g1"),
        0.5,
      ),
    ).toBeNull();
    expect(
      landingOn({ kind: "group", id: "g2", open: false }, group("g1"), 0.5),
    ).toEqual({ id: "g2", where: "after" });
  });

  it("puts a group only above an open group, whose row has its mailboxes below it", () => {
    expect(
      landingOn({ kind: "group", id: "g2", open: true }, group("g1"), 0.9),
    ).toEqual({ id: "g2", where: "before" });
  });

  it("puts whatever is carried last from the space below everything", () => {
    expect(landingOn({ kind: "end" }, account("acc-1"), null)).toEqual({
      id: END,
      where: "after",
    });
    expect(landingOn({ kind: "end" }, group("g1"), 0.5)).toEqual({
      id: END,
      where: "after",
    });
  });

  it("puts a group above or below a mailbox in no group", () => {
    const loose = { kind: "account" as const, id: "acc-1", groupId: null };

    expect(landingOn(loose, group("g1"), 0.1)).toEqual({
      id: "acc-1",
      where: "before",
    });
    expect(landingOn(loose, group("g1"), 0.9)).toEqual({
      id: "acc-1",
      where: "after",
    });
  });
});

describe("afterDrop", () => {
  /** Work holding one and two, a mailbox in none, then Home holding three. */
  const top: Placing[] = [
    { kind: "group", id: "work", accounts: ["one", "two"] },
    { kind: "account", id: "loose" },
    { kind: "group", id: "home", accounts: ["three"] },
  ];

  it("moves a mailbox among the others in its group", () => {
    expect(
      afterDrop(top, account("two"), { id: "one", where: "before" }),
    ).toEqual({ run: { ids: ["two", "one"], groupId: "work" } });
  });

  it("puts a mailbox dropped on a group into it, and leaves its place there to the group", () => {
    expect(
      afterDrop(top, account("loose"), { id: "home", where: "into" }),
    ).toEqual({ home: { accountId: "loose", groupId: "home" } });
  });

  it("changes nothing for a mailbox dropped into the group it is already in", () => {
    expect(
      afterDrop(top, account("one"), { id: "work", where: "into" }),
    ).toBeNull();
  });

  it("puts a mailbox from the top into a group at the place it was dropped", () => {
    expect(
      afterDrop(top, account("loose"), { id: "two", where: "before" }),
    ).toEqual({
      home: { accountId: "loose", groupId: "work" },
      run: { ids: ["one", "loose", "two"], groupId: "work" },
    });
  });

  it("takes a mailbox out of its group to the place it was dropped at the top", () => {
    expect(
      afterDrop(top, account("one"), { id: "loose", where: "after" }),
    ).toEqual({
      home: { accountId: "one", groupId: null },
      run: { ids: ["work", "loose", "one", "home"], groupId: null },
    });
  });

  it("takes a mailbox out of its group to just above the group", () => {
    expect(
      afterDrop(top, account("two"), { id: "work", where: "before" }),
    ).toEqual({
      home: { accountId: "two", groupId: null },
      run: { ids: ["two", "work", "loose", "home"], groupId: null },
    });
  });

  it("moves a mailbox from one group to a place in another", () => {
    expect(
      afterDrop(top, account("one"), { id: "three", where: "after" }),
    ).toEqual({
      home: { accountId: "one", groupId: "home" },
      run: { ids: ["three", "one"], groupId: "home" },
    });
  });

  it("moves a group with everything in it among the things at the top", () => {
    expect(
      afterDrop(top, group("home"), { id: "work", where: "before" }),
    ).toEqual({ run: { ids: ["home", "work", "loose"], groupId: null } });
  });

  it("puts no group into another", () => {
    expect(
      afterDrop(top, group("home"), { id: "work", where: "into" }),
    ).toBeNull();
  });

  it("changes nothing where a row is dropped back where it stands", () => {
    // Above the row below it is where it already is: a line there would promise a move that
    // does not happen.
    expect(
      afterDrop(top, account("one"), { id: "two", where: "before" }),
    ).toBeNull();
    expect(
      afterDrop(top, group("work"), { id: "loose", where: "before" }),
    ).toBeNull();
  });

  it("takes a mailbox out of its group to the very end, below an open group", () => {
    expect(afterDrop(top, account("three"), { id: END, where: "after" })).toEqual({
      home: { accountId: "three", groupId: null },
      run: { ids: ["work", "loose", "home", "three"], groupId: null },
    });
  });

  it("moves a group to the very end", () => {
    expect(afterDrop(top, group("work"), { id: END, where: "after" })).toEqual({
      run: { ids: ["loose", "home", "work"], groupId: null },
    });
  });

  it("changes nothing for the last thing dropped at the end", () => {
    expect(afterDrop(top, group("home"), { id: END, where: "after" })).toBeNull();
  });

  it("changes nothing for a row the arrangement does not hold", () => {
    expect(
      afterDrop(top, account("one"), { id: "gone", where: "before" }),
    ).toBeNull();
  });
});
