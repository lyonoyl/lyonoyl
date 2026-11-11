import { describe, expect, it } from "vitest";

import { groupOf, inOrderOfNames } from "./group";

const named = (...names: string[]) => names.map((name) => ({ name }));
const names = (groups: { name: string }[]) => groups.map((group) => group.name);

describe("inOrderOfNames", () => {
  it("puts groups in the order of their names, whatever their case", () => {
    expect(names(inOrderOfNames(named("work", "Archive", "home"), "en"))).toEqual([
      "Archive",
      "home",
      "work",
    ]);
  });

  it("reads letters with accents where their language puts them", () => {
    // "École" among the names in E, not after every name in Z where its first byte would put it.
    expect(names(inOrderOfNames(named("Zeta", "École", "Delta"), "fr"))).toEqual([
      "Delta",
      "École",
      "Zeta",
    ]);
  });

  it("reads numbers as numbers", () => {
    expect(
      names(inOrderOfNames(named("Project 10", "Project 2", "Project 1"), "en")),
    ).toEqual(["Project 1", "Project 2", "Project 10"]);
  });

  it("leaves the list it was given as it was", () => {
    const given = named("b", "a");
    inOrderOfNames(given, "en");
    expect(names(given)).toEqual(["b", "a"]);
  });
});

describe("groupOf", () => {
  const groups = [
    { id: "g1", accounts: ["acc-1", "acc-2"] },
    { id: "g2", accounts: ["acc-3"] },
  ];

  it("is the group chosen in the sidebar", () => {
    expect(groupOf({ kind: "group", groupId: "g2" }, groups)).toBe("g2");
  });

  it("is the group of the mailbox or the folder chosen", () => {
    expect(groupOf({ kind: "account", accountId: "acc-2" }, groups)).toBe("g1");
    expect(
      groupOf(
        { kind: "folder", accountId: "acc-3", folderId: "f-1", role: "inbox" },
        groups,
      ),
    ).toBe("g2");
  });

  it("is no group for everything at once, a mailbox in none, or a group that is gone", () => {
    expect(groupOf({ kind: "everything" }, groups)).toBeNull();
    expect(groupOf({ kind: "account", accountId: "acc-9" }, groups)).toBeNull();
    expect(groupOf({ kind: "group", groupId: "gone" }, groups)).toBeNull();
    expect(groupOf(null, groups)).toBeNull();
  });
});
