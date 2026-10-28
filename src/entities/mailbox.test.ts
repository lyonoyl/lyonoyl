import { describe, expect, it } from "vitest";

import { questionKey, scopeKey } from "./mailbox";

/**
 * Two keys, and the difference between them is the whole of it.
 *
 * `scopeKey` says which list is on the screen and must not move while somebody types, or React
 * remounts the list on every settled word and takes the scroll position and the loaded rows with
 * it. `questionKey` says what that list is asking and must move, or the list answers the first
 * text that settles and holds it until the box is emptied.
 *
 * Both of those have happened. Neither was pinned by anything: the constant key lives in one
 * branch of one function, and changing it left all of the suite green.
 */
describe("what identifies a list, and what identifies its question", () => {
  it("gives every search one list, however the words change", () => {
    expect(scopeKey({ kind: "looking", typed: "quarter" })).toBe(
      scopeKey({ kind: "looking", typed: "quarterly" }),
    );
    expect(scopeKey({ kind: "looking", typed: "" })).toBe(
      scopeKey({ kind: "looking", typed: "invoice" }),
    );
  });

  it("gives every search its own question", () => {
    expect(questionKey({ kind: "looking", typed: "quarter" })).not.toBe(
      questionKey({ kind: "looking", typed: "quarterly" }),
    );
  });

  it("keeps a search apart from a folder and an account", () => {
    const keys = [
      scopeKey({ kind: "folder", folderId: "7" }),
      scopeKey({ kind: "account", accountId: "acc-1" }),
      scopeKey({ kind: "everything" }),
      scopeKey({ kind: "looking", typed: "quarter" }),
    ];
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("asks the same question of a folder however it is reached", () => {
    // Only a search carries words. For everything else the two keys are the same string, so
    // nothing about a folder is re-asked for a reason that does not exist.
    for (const scope of [
      { kind: "folder", folderId: "7" },
      { kind: "account", accountId: "acc-1" },
      { kind: "everything" },
    ] as const) {
      expect(questionKey(scope)).toBe(scopeKey(scope));
    }
  });
});
