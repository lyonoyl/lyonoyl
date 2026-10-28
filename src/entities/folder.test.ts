import { describe, expect, it } from "vitest";

import {
  folderBadge,
  unreadBadge,
  withReadChanges,
  type ReadChange,
} from "./folder";
import type { AccountNodeDto } from "@/shared/ipc/generated/AccountNodeDto";
import type { FolderRoleDto } from "@/shared/ipc/generated/FolderRoleDto";

function folder(
  role: FolderRoleDto,
  unread: number,
  total = 10,
  draftsHere = 0,
) {
  return { role, unread, total, draftsHere };
}

describe("what a folder shows beside its name", () => {
  it.each<FolderRoleDto>(["inbox", "custom", "junk", "flagged"])(
    "counts unread mail in %s, where mail arrives and waits",
    (role) => {
      expect(folderBadge(folder(role, 3))).toEqual({
        count: 3,
        label: "folder.unread",
      });
    },
  );

  it.each<FolderRoleDto>(["sent", "archive", "trash", "allMail"])(
    "shows nothing beside %s, however the server flagged what is in it",
    (role) => {
      expect(folderBadge(folder(role, 7))).toBeNull();
    },
  );

  it("counts drafts rather than unread ones, because a draft was written here", () => {
    expect(folderBadge(folder("drafts", 0, 4))).toEqual({
      count: 4,
      label: "folder.draftCount",
    });
  });

  it("counts the drafts on the server and the ones written here together, as the folder shows them", () => {
    expect(folderBadge(folder("drafts", 0, 1, 2))).toEqual({
      count: 3,
      label: "folder.draftCount",
    });
  });

  it("counts a draft written here when the server's folder is empty", () => {
    // The folder shows it, so a count of nought beside it would be a number about a different list.
    expect(folderBadge(folder("drafts", 0, 0, 1))).toEqual({
      count: 1,
      label: "folder.draftCount",
    });
  });

  it("says nothing about an empty draft folder", () => {
    expect(folderBadge(folder("drafts", 0, 0, 0))).toBeNull();
  });

  it("says nothing where everything has been read", () => {
    expect(folderBadge(folder("inbox", 0))).toBeNull();
  });

  it("gives an account its own row the same unread badge", () => {
    expect(unreadBadge(5)).toEqual({ count: 5, label: "folder.unread" });
    expect(unreadBadge(0)).toBeNull();
  });
});

describe("counts with what a person marked laid over them", () => {
  const counted = (): AccountNodeDto[] => [
    {
      place: 0,
      accountId: "acc-1",
      email: "zeta@example.com",
      displayName: "",
      collapsed: false,
      answered: true,
      unread: 3,
      folders: [
        {
          folderId: "1",
          name: "INBOX",
          role: "inbox",
          total: 9,
          unread: 3,
          draftsHere: 0,
          canReceive: true,
        },
        {
          folderId: "2",
          name: "Lists",
          role: "custom",
          total: 4,
          unread: 1,
          draftsHere: 0,
          canReceive: true,
        },
      ],
    },
  ];
  const change = (
    folderId: string,
    unread: -1 | 1,
    asked: number | null = 150,
  ): ReadChange => ({ folderId, unread, asked, askedAt: 0 });

  it("moves an inbox and the account it belongs to, in one go", () => {
    const [account] = withReadChanges(counted(), [change("1", -1)], 100);

    expect(account.folders[0].unread).toBe(2);
    expect(account.unread).toBe(2);
  });

  it("moves a folder that is not an inbox and leaves the account's inbox count alone", () => {
    const [account] = withReadChanges(counted(), [change("2", -1)], 100);

    expect(account.folders[1].unread).toBe(0);
    expect(account.unread).toBe(3);
  });

  it("is not laid over counts that already hold it", () => {
    // Counts read once the queue had got as far as the write hold it, whenever the read began: a
    // read already on its way when the change was written can come back with it. Laid over it
    // again, the number drops twice for one message.
    const [before] = withReadChanges(counted(), [change("1", -1, 150)], 149);
    const [holding] = withReadChanges(counted(), [change("1", -1, 150)], 150);
    const [after] = withReadChanges(counted(), [change("1", -1, 150)], 200);

    expect(before.folders[0].unread).toBe(2);
    expect(holding.folders[0].unread).toBe(3);
    expect(after.folders[0].unread).toBe(3);
  });

  it("is laid over whatever was read until the write answers", () => {
    // A read can hold it only once it is written, and the answer follows the writing by the
    // moment it takes to say so; dropped until then, the number would go back up under the dot.
    const [account] = withReadChanges(counted(), [change("1", -1, null)], 999);

    expect(account.folders[0].unread).toBe(2);
  });

  it("never counts below nothing", () => {
    const [account] = withReadChanges(
      counted(),
      [change("2", -1), change("2", -1)],
      100,
    );

    expect(account.folders[1].unread).toBe(0);
  });

  it("marks up as well as down", () => {
    const [account] = withReadChanges(counted(), [change("1", 1)], 100);

    expect(account.folders[0].unread).toBe(4);
    expect(account.unread).toBe(4);
  });

  it("hands back what it was given when nothing is laid over it", () => {
    const accounts = counted();

    expect(withReadChanges(accounts, [], 100)).toBe(accounts);
    expect(withReadChanges(accounts, [change("1", -1, 50)], 100)).toBe(
      accounts,
    );
  });
});
