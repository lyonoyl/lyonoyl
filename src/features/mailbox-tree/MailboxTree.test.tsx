import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ReadChange } from "@/entities/folder";
import type { Mailbox } from "@/entities/mailbox";
import { I18nProvider } from "@/shared/i18n";
import type { AccountNodeDto } from "@/shared/ipc/generated/AccountNodeDto";
import type { MailboxTreeDto } from "@/shared/ipc/generated/MailboxTreeDto";
import type { SyncReportDto } from "@/shared/ipc/generated/SyncReportDto";
import { applyNativeFeel } from "@/app/native-feel";
import { movedWithin } from "./AccountMenu";
import { MailboxTree } from "./MailboxTree";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    getMailboxTree: vi.fn(),
    listAccountGroups: vi.fn(),
    setAccountCollapsed: vi.fn(),
    setAccountGroupCollapsed: vi.fn(),
    placeAccountInGroup: vi.fn(),
    reorderAccounts: vi.fn(),
    arrangeSidebar: vi.fn(),
    syncAccount: vi.fn(),
    createAccountGroup: vi.fn(),
    removeAccountGroup: vi.fn(),
  };
});

const client = await import("@/shared/ipc/client");
const tree = vi.mocked(client.getMailboxTree);
const persist = vi.mocked(client.setAccountCollapsed);
const arrangement = vi.mocked(client.listAccountGroups);
const foldGroup = vi.mocked(client.setAccountGroupCollapsed);
const sync = vi.mocked(client.syncAccount);
const regroup = vi.mocked(client.placeAccountInGroup);
const reorder = vi.mocked(client.reorderAccounts);
const arrangeTop = vi.mocked(client.arrangeSidebar);
const makeGroup = vi.mocked(client.createAccountGroup);
const dropGroup = vi.mocked(client.removeAccountGroup);

/** The sidebar as the core answers it, read before anything was asked for. */
function sidebar(accounts: AccountNodeDto[]): MailboxTreeDto {
  return { accounts, askedThrough: 0 };
}

function account(
  id: string,
  email: string,
  folders: AccountNodeDto["folders"],
  extra: Partial<AccountNodeDto> = {},
): AccountNodeDto {
  return {
    accountId: id,
    email,
    displayName: "",
    collapsed: false,
    place: 0,
    answered: true,
    unread: 0,
    folders,
    ...extra,
  };
}

const INBOX = {
  folderId: "1",
  name: "INBOX",
  role: "inbox" as const,
  total: 9,
  unread: 2,
  draftsHere: 0,
  canReceive: true,
};
const ARCHIVE = {
  folderId: "2",
  name: "Archive",
  role: "archive" as const,
  total: 4,
  unread: 0,
  draftsHere: 0,
  canReceive: true,
};
const OTHER_INBOX = {
  folderId: "9",
  name: "INBOX",
  role: "inbox" as const,
  total: 3,
  unread: 1,
  draftsHere: 0,
  canReceive: true,
};
const SENT = {
  folderId: "3",
  name: "Sent",
  role: "sent" as const,
  total: 6,
  unread: 2,
  draftsHere: 0,
  canReceive: false,
};
const DRAFTS = {
  folderId: "4",
  name: "Drafts",
  role: "drafts" as const,
  total: 3,
  unread: 0,
  draftsHere: 0,
  canReceive: false,
};

function renderTree(selected: Mailbox | null = null, language = "en") {
  const onSelect = vi.fn();
  const onAddAccount = vi.fn();
  const onReconnect = vi.fn();
  render(
    <I18nProvider initial={language}>
      <MailboxTree
        selected={selected}
        onSelect={onSelect}
        onAddAccount={onAddAccount}
        onReconnect={onReconnect}
      />
    </I18nProvider>,
  );
  return { onSelect, onAddAccount, onReconnect };
}

beforeEach(() => {
  tree.mockReset();
  persist.mockReset();
  persist.mockResolvedValue(undefined);
  // Reset rather than left standing: a call count that survives into the next test turns "asked
  // once" into a claim about the order the tests happened to run in.
  arrangement.mockReset();
  arrangement.mockResolvedValue([]);
  foldGroup.mockReset();
  foldGroup.mockResolvedValue(undefined);
  regroup.mockReset();
  regroup.mockResolvedValue(undefined);
  reorder.mockReset();
  reorder.mockResolvedValue(undefined);
  arrangeTop.mockReset();
  arrangeTop.mockResolvedValue(undefined);
  makeGroup.mockReset();
  makeGroup.mockResolvedValue("g1");
  dropGroup.mockReset();
  dropGroup.mockResolvedValue(undefined);
  sync.mockReset();
  sync.mockResolvedValue({
    folders: 0,
    arrived: 0,
    foldersFailed: 0,
    foldersIncomplete: 0,
  });
});

describe("MailboxTree", () => {
  it("says an account's mail could not be read instead of saying it is being read", async () => {
    // The two look identical — an account with nothing under it — and mean the opposite. Left
    // as "reading folders" it is a message that never comes true.
    const quiet = {
      accountId: "acc-2",
      email: "quiet@example.com",
      displayName: "Quiet",
      collapsed: false,
      place: 0,
      answered: false,
      unread: 0,
      folders: [],
    };

    tree.mockResolvedValue(sidebar([quiet]));
    renderTree();

    const said = await screen.findByRole("alert");
    expect(said.textContent).toContain("could not be read");
    expect(screen.queryByText(/Reading folders/i)).toBeNull();
  });

  it("says so when the press found every account already being read", async () => {
    // A round already under way answers "being checked right now", which is not a failure. When that
    // is the answer for every account the press did nothing at all -- and saying nothing about it
    // is a button that looks broken, which is exactly what it looked like.
    tree.mockResolvedValue(
      sidebar([account("acc-1", "one@example.com", [INBOX])]),
    );
    sync.mockRejectedValue({
      code: "alreadyReading",
      message: "",
      path: null,
    });
    renderTree();

    fireEvent.click(
      await screen.findByRole("button", { name: "Get new mail" }),
    );

    const said = await screen.findByRole("alert");
    expect(said.textContent).toContain("being checked right now");
  });

  it("says nothing when one account was already being read and another ran", async () => {
    // Then the press did something, and a notice about the one that was busy would be a failure
    // reported for a round that worked.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [INBOX]),
      ]),
    );
    sync.mockImplementation((accountId: string) => {
      if (accountId === "acc-1") {
        return Promise.reject({
          code: "alreadyReading",
          message: "",
          path: null,
        });
      }
      return Promise.resolve({
        folders: 1,
        arrived: 0,
        foldersFailed: 0,
        foldersIncomplete: 0,
      });
    });
    renderTree();

    fireEvent.click(
      await screen.findByRole("button", { name: "Get new mail" }),
    );

    await waitFor(() => expect(sync).toHaveBeenCalledTimes(2));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows one account as its folders alone", async () => {
    // A tree with a single root is a row of wasted space and a fold that does nothing.
    tree.mockResolvedValue(
      sidebar([account("acc-1", "user@example.com", [INBOX, ARCHIVE])]),
    );
    renderTree();

    expect(await screen.findByRole("button", { name: /Inbox/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Archive/ })).toBeTruthy();
    expect(
      screen.queryByRole("button", { name: "user@example.com" }),
    ).toBeNull();
    expect(screen.queryByRole("button", { name: "All inboxes" })).toBeNull();
  });

  it("scrolls the mailboxes without drawing a bar, and never sideways", async () => {
    // Nine mailboxes with their folders are taller than any window, so the tree has to scroll --
    // by wheel, by trackpad and by the focus moving through it, which only a scroller does. What
    // goes is the bar, and a gutter the rows stop short of. Asserted as the rule, because this
    // runs without layout and nothing here can scroll.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX, ARCHIVE]),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
      ]),
    );
    renderTree();

    await screen.findByRole("button", { name: "one@example.com" });
    const classes = screen
      .getByRole("navigation", { name: "Mailboxes" })
      .className.split(/\s+/);

    expect(classes).toContain("no-gutter");
    expect(classes, "still a scroller up and down").toContain("overflow-y-auto");
    expect(classes, "never a scroller sideways").toContain("overflow-x-hidden");
    // Hidden up and down would take the wheel and the focus with the bar.
    expect(classes).not.toContain("overflow-hidden");
    expect(classes).not.toContain("overflow-y-hidden");
  });

  it("shows every account once there is more than one", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
      ]),
    );
    renderTree();

    expect(
      await screen.findByRole("button", { name: "one@example.com" }),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "two@example.com" }),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: /All inboxes/ })).toBeTruthy();
  });

  it("opens everything at once when several accounts are present", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
      ]),
    );
    const { onSelect } = renderTree(null);

    await waitFor(() =>
      expect(onSelect).toHaveBeenCalledWith({ kind: "everything" }),
    );
  });

  it("opens the inbox when there is only one account", async () => {
    tree.mockResolvedValue(
      sidebar([account("acc-1", "user@example.com", [INBOX, ARCHIVE])]),
    );
    const { onSelect } = renderTree(null);

    await waitFor(() =>
      expect(onSelect).toHaveBeenCalledWith({
        kind: "folder",
        accountId: "acc-1",
        folderId: "1",
        // Carried with the selection: the window asks one question of it — is this the trash —
        // and looking that up again would be a second place deciding what the trash is.
        role: "inbox",
      }),
    );
  });

  it("opens the account itself when it has no folders yet", async () => {
    tree.mockResolvedValue(
      sidebar([account("acc-1", "fresh@example.com", [])]),
    );
    const { onSelect } = renderTree(null);

    await waitFor(() =>
      expect(onSelect).toHaveBeenCalledWith({
        kind: "account",
        accountId: "acc-1",
      }),
    );
  });

  it("selects a folder together with the account it belongs to", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
      ]),
    );
    const { onSelect } = renderTree({ kind: "everything" });

    const folders = await screen.findAllByRole("button", { name: /Inbox/ });
    fireEvent.click(folders[1]);

    expect(onSelect).toHaveBeenCalledWith({
      kind: "folder",
      accountId: "acc-2",
      folderId: "9",
      role: "inbox",
    });
  });

  it("selects the whole account when its name is pressed", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
      ]),
    );
    const { onSelect } = renderTree({ kind: "everything" });

    fireEvent.click(
      await screen.findByRole("button", { name: "two@example.com" }),
    );

    expect(onSelect).toHaveBeenCalledWith({
      kind: "account",
      accountId: "acc-2",
    });
  });

  it("selects everything from the row above the accounts", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
      ]),
    );
    const { onSelect } = renderTree({ kind: "account", accountId: "acc-1" });

    fireEvent.click(await screen.findByRole("button", { name: /All inboxes/ }));

    expect(onSelect).toHaveBeenCalledWith({ kind: "everything" });
  });

  it("folds an account away and remembers it", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
      ]),
    );
    renderTree({ kind: "everything" });

    const [fold] = await screen.findAllByRole("button", {
      name: /hide the folders of/i,
    });
    fireEvent.click(fold);

    await waitFor(() => expect(persist).toHaveBeenCalledWith("acc-1", true));
    expect(screen.queryByRole("button", { name: /Archive/ })).toBeNull();
  });

  it("opens an account that was folded away when it was last used", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX], { collapsed: true }),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
      ]),
    );
    renderTree({ kind: "everything" });

    // Only the second account's inbox is on screen; the first one's folders are folded away.
    const folders = await screen.findAllByRole("button", { name: /Inbox/ });
    expect(folders).toHaveLength(1);
    expect(
      screen.getByRole("button", { name: /show the folders of/i }),
    ).toBeTruthy();
  });

  it("shows what was actually stored when folding away could not be kept", async () => {
    persist.mockRejectedValue(new Error("the profile is unavailable"));
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
      ]),
    );
    renderTree({ kind: "everything" });

    const [fold] = await screen.findAllByRole("button", {
      name: /hide the folders of/i,
    });
    fireEvent.click(fold);

    await waitFor(() =>
      expect(
        screen.getAllByRole("button", { name: /hide the folders of/i }),
      ).toHaveLength(2),
    );
  });

  it("offers to add an account from the plus", async () => {
    tree.mockResolvedValue(
      sidebar([account("acc-1", "user@example.com", [INBOX])]),
    );
    const { onAddAccount } = renderTree({ kind: "everything" });

    fireEvent.pointerDown(
      await screen.findByRole("button", { name: "Add an account or a group" }),
      { button: 0, ctrlKey: false },
    );
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Add account" }),
    );

    await waitFor(() => expect(onAddAccount).toHaveBeenCalled());
  });

  it("reads its groups again once one is made from the plus, and tells the window", async () => {
    tree.mockResolvedValue(
      sidebar([account("acc-1", "user@example.com", [INBOX])]),
    );
    const onArrangementChanged = vi.fn();
    render(
      <I18nProvider initial="en">
        <MailboxTree
          selected={null}
          onSelect={vi.fn()}
          onAddAccount={vi.fn()}
          onReconnect={vi.fn()}
          onArrangementChanged={onArrangementChanged}
        />
      </I18nProvider>,
    );
    await waitFor(() => expect(arrangement).toHaveBeenCalled());
    const readBefore = arrangement.mock.calls.length;
    arrangement.mockResolvedValue([
      { id: "g1", name: "Work", place: 0, collapsed: false, accounts: [] },
    ]);

    fireEvent.pointerDown(
      screen.getByRole("button", { name: "Add an account or a group" }),
      { button: 0, ctrlKey: false },
    );
    fireEvent.click(await screen.findByRole("menuitem", { name: "Add group" }));
    const field = await screen.findByRole("textbox", { name: "Group name" });
    fireEvent.change(field, { target: { value: "Work" } });
    fireEvent.submit(field.closest("form")!);

    await waitFor(() =>
      expect(arrangement.mock.calls.length).toBeGreaterThan(readBefore),
    );
    expect(makeGroup).toHaveBeenCalledWith("Work");
    expect(onArrangementChanged).toHaveBeenCalledTimes(1);
    expect(await screen.findByText("Work")).toBeTruthy();
  });

  it("names folders with a role in the reader's language", async () => {
    tree.mockResolvedValue(
      sidebar([account("acc-1", "user@example.com", [INBOX, ARCHIVE])]),
    );
    renderTree({ kind: "everything" }, "ru");

    expect(
      await screen.findByRole("button", { name: /Входящие/ }),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: /Архив/ })).toBeTruthy();
  });

  it("keeps a folder the user made under its own name", async () => {
    const projects = {
      folderId: "7",
      name: "Projects 2026",
      role: "custom" as const,
      total: 1,
      unread: 0,
      draftsHere: 0,
      canReceive: true,
    };
    tree.mockResolvedValue(
      sidebar([account("acc-1", "user@example.com", [projects])]),
    );
    renderTree({ kind: "everything" }, "ru");

    expect(
      await screen.findByRole("button", { name: /Projects 2026/ }),
    ).toBeTruthy();
  });

  it("asks the server of an account that has no folders yet", async () => {
    // An account is added in one action and filled in another; nobody should have to press
    // anything in between.
    tree.mockResolvedValue(
      sidebar([account("acc-1", "fresh@example.com", [])]),
    );
    renderTree({ kind: "everything" });

    await waitFor(() => expect(sync).toHaveBeenCalledWith("acc-1"));
  });

  it("does not ask about an account that already has its folders", async () => {
    tree.mockResolvedValue(
      sidebar([account("acc-1", "user@example.com", [INBOX])]),
    );
    renderTree({ kind: "everything" });

    await screen.findByRole("button", { name: /Inbox/ });
    expect(sync).not.toHaveBeenCalled();
  });

  it("asks about an empty account once, not on every reload", async () => {
    // A server that genuinely has no mailboxes answers with none, the tree is read again, and the
    // account is still empty. Without a record of having asked, that is a loop that never stops.
    tree.mockResolvedValue(
      sidebar([account("acc-1", "empty@example.com", [])]),
    );
    renderTree({ kind: "everything" });

    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(tree).toHaveBeenCalledTimes(2));
    // Give the effect every chance to fire again on the reloaded, still-empty tree.
    await new Promise((resume) => setTimeout(resume, 50));

    expect(sync).toHaveBeenCalledTimes(1);
    expect(tree).toHaveBeenCalledTimes(2);
  });

  it("keeps the sidebar when a server cannot be reached, and says which account", async () => {
    sync.mockRejectedValue({ code: "offline", message: "", path: null });
    tree.mockResolvedValue(
      sidebar([account("acc-1", "fresh@example.com", [])]),
    );
    renderTree({ kind: "everything" });

    const said = (await screen.findByRole("alert")).textContent ?? "";
    expect(said).toContain("The mail server could not be reached");
    expect(
      said,
      "a failure nobody can attribute is a failure nobody can act on",
    ).toContain("fresh@example.com");
    expect(
      screen.getByRole("button", { name: "Add an account or a group" }),
    ).toBeTruthy();
  });

  it("offers to type the password again for an account that has none", async () => {
    // The one failure that is a thing to do rather than a thing to read.
    sync.mockRejectedValue({
      code: "credentialsMissing",
      message: "",
      path: null,
    });
    tree.mockResolvedValue(sidebar([account("acc-1", "gone@example.com", [])]));
    const { onReconnect } = renderTree({ kind: "everything" });

    const said = await screen.findByRole("alert");
    expect(said.textContent).toContain("gone@example.com");

    fireEvent.click(
      screen.getByRole("button", { name: "Enter the password again" }),
    );

    expect(onReconnect).toHaveBeenCalledWith("acc-1");
  });

  it("offers the same way back when the server refuses the password", async () => {
    // A password that stopped working is the ordinary case — it is changed, or expired, or was
    // typed wrong once — and it arrives as a different code from a password that is gone. Both are
    // the same thing to do, so both get the button; without this one, the commoner of the two is a
    // sentence the reader cannot act on.
    sync.mockRejectedValue({
      code: "unauthorized",
      message: "",
      path: null,
    });
    tree.mockResolvedValue(
      sidebar([account("acc-1", "stale@example.com", [])]),
    );
    const { onReconnect } = renderTree({ kind: "everything" });

    const said = await screen.findByRole("alert");
    expect(said.textContent).toContain("stale@example.com");

    fireEvent.click(
      screen.getByRole("button", { name: "Enter the password again" }),
    );

    expect(onReconnect).toHaveBeenCalledWith("acc-1");
  });

  it("asks every account when told to get new mail", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
      ]),
    );
    renderTree({ kind: "everything" });

    fireEvent.click(
      await screen.findByRole("button", { name: "Get new mail" }),
    );

    await waitFor(() => expect(sync).toHaveBeenCalledTimes(2));
    expect(sync.mock.calls.map((call) => call[0])).toEqual(["acc-1", "acc-2"]);
  });

  it("asks about every account at once rather than waiting for each", async () => {
    // The cap lives in the core now: `sync_account` waits its turn behind the same width a
    // background round uses. Asking one at a time here made one slow server hold up every account
    // behind it — nine mailboxes at five seconds each was fifty seconds of a window that looked
    // frozen, and this is the path it takes when it opens and when somebody presses refresh.
    let releaseFirst = () => {};
    sync.mockImplementationOnce(
      () =>
        new Promise<SyncReportDto>((resolve) => {
          releaseFirst = () =>
            resolve({
              folders: 1,
              arrived: 0,
              foldersFailed: 0,
              foldersIncomplete: 0,
            });
        }),
    );
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
      ]),
    );
    renderTree({ kind: "everything" });

    fireEvent.click(
      await screen.findByRole("button", { name: "Get new mail" }),
    );

    // Both, while the first is still unanswered.
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(2));
    expect(sync).toHaveBeenCalledWith("acc-1");
    expect(sync).toHaveBeenCalledWith("acc-2");

    releaseFirst();
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(2));
  });

  it("reads the tree again once the servers have answered", async () => {
    tree.mockResolvedValue(
      sidebar([account("acc-1", "one@example.com", [INBOX])]),
    );
    renderTree({ kind: "everything" });

    fireEvent.click(
      await screen.findByRole("button", { name: "Get new mail" }),
    );

    await waitFor(() => expect(tree).toHaveBeenCalledTimes(2));
  });

  it("reports a sidebar it could not read", async () => {
    tree.mockRejectedValue({
      code: "storageUnavailable",
      message: "",
      path: "/somewhere/profile.sqlite3",
    });
    renderTree();

    const said = (await screen.findByRole("alert")).textContent;
    expect(said).toContain("The local database could not be opened");
    // The recovery screen names a file that will not open. Only a newer build's file, which may be
    // one account's alone, is named here.
    expect(said).not.toContain("/somewhere/profile.sqlite3");
  });

  it("names the file a newer version wrote, which may be one account's alone", async () => {
    // The profile opened and no recovery screen is up, so the sidebar is the one place that can
    // say which file it is.
    tree.mockRejectedValue({
      code: "storageFromANewerBuild",
      message: "",
      path: "/somewhere/accounts/3.sqlite3",
    });
    renderTree();

    const said = (await screen.findByRole("alert")).textContent;
    expect(said).toContain(
      "A newer version of Lyonoyl\u00a0Mail wrote a file this version does not open",
    );
    expect(said).toContain("/somewhere/accounts/3.sqlite3");
    // A path reads left to right in every language.
    expect(
      screen.getByText("/somewhere/accounts/3.sqlite3").getAttribute("dir"),
    ).toBe("ltr");
    expect(screen.getByText("/somewhere/accounts/3.sqlite3").className).toContain(
      "[text-align:match-parent]",
    );
  });

  it("leaves a key that could not be had to the window's recovery screen", async () => {
    // The window puts its recovery screen up for it, saying where the key is and what to do. A
    // second account of it here is one nobody can act on.
    tree.mockRejectedValue({
      code: "storageKeyUnavailable",
      message: "",
      path: null,
    });
    renderTree();
    const add = { name: "Add an account or a group" };
    expect(screen.getByRole("button", add)).toBeTruthy();

    // Gone in the render that takes the failure in, which is the render to look at.
    await waitFor(() => expect(screen.queryByRole("button", add)).toBeNull());

    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("leaves Sent unbadged even when the server calls its messages unread", async () => {
    tree.mockResolvedValue(
      sidebar([account("acc-1", "one@example.com", [INBOX, SENT, DRAFTS])]),
    );
    renderTree();

    expect(await screen.findByLabelText("2 unread")).toBeTruthy();
    expect(screen.queryByLabelText("6 unread")).toBeNull();
    expect(screen.getByLabelText("3 drafts")).toBeTruthy();
  });

  it("does not ask an empty account for its mailboxes again on every pass", async () => {
    // The set of accounts already asked is the only thing stopping a server with no folders from
    // being asked over and over. Building the tree again on every background change threw it away,
    // which turned one connection per empty account into one per account per pass.
    // A fresh answer each time, the way the core gives one. Handing back the same array makes
    // React skip the render that would have asked again, and the test then passes for a reason
    // that has nothing to do with what it is named for.
    tree.mockImplementation(async () =>
      sidebar([account("acc-1", "one@example.com", [])]),
    );
    sync.mockResolvedValue({
      folders: 0,
      arrived: 0,
      foldersIncomplete: 0,
      foldersFailed: 0,
    });

    const view = render(
      <I18nProvider initial="en">
        <MailboxTree
          selected={null}
          onSelect={vi.fn()}
          onAddAccount={vi.fn()}
          onReconnect={vi.fn()}
          generation={0}
        />
      </I18nProvider>,
    );

    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));

    for (const generation of [1, 2, 3]) {
      view.rerender(
        <I18nProvider initial="en">
          <MailboxTree
            selected={null}
            onSelect={vi.fn()}
            onAddAccount={vi.fn()}
            onReconnect={vi.fn()}
            generation={generation}
          />
        </I18nProvider>,
      );
      await waitFor(() =>
        expect(tree.mock.calls.length).toBeGreaterThanOrEqual(generation + 1),
      );
    }

    expect(sync).toHaveBeenCalledTimes(1);
  });

  it("reads itself once at a time, and once more after a burst of changes", async () => {
    // Every change a person makes asks for the tree again, and a key held down is thirty of them a
    // second. Each one started while the last is still being answered is the core reading every
    // account for an answer nobody looks at: one on its way and one behind it is all there is.
    const answers: ((tree: MailboxTreeDto) => void)[] = [];
    tree.mockImplementation(
      () => new Promise<MailboxTreeDto>((answer) => answers.push(answer)),
    );

    const draw = (generation: number) => (
      <I18nProvider initial="en">
        <MailboxTree
          selected={null}
          onSelect={vi.fn()}
          onAddAccount={vi.fn()}
          onReconnect={vi.fn()}
          generation={generation}
        />
      </I18nProvider>
    );
    const { rerender } = render(draw(0));
    await waitFor(() => expect(tree).toHaveBeenCalledTimes(1));

    for (const generation of [1, 2, 3, 4, 5]) rerender(draw(generation));
    await new Promise((settle) => setTimeout(settle, 20));
    expect(tree).toHaveBeenCalledTimes(1);

    await act(async () =>
      answers[0](sidebar([account("acc-1", "one@example.com", [INBOX])])),
    );
    await waitFor(() => expect(tree).toHaveBeenCalledTimes(2));

    await act(async () =>
      answers[1](sidebar([account("acc-1", "one@example.com", [INBOX])])),
    );
    await new Promise((settle) => setTimeout(settle, 20));
    expect(tree).toHaveBeenCalledTimes(2);
  });

  it("asks again once the pass it collided with has changed something", async () => {
    // Being read already is what was asked for, not a refusal -- but the pass it collided with can
    // end having brought this account nothing, and then nobody has asked its server anything.
    // Left at that, the account sits empty in the sidebar for the rest of the session.
    tree.mockImplementation(async () =>
      sidebar([account("acc-1", "one@example.com", [])]),
    );
    sync.mockRejectedValueOnce({
      code: "alreadyReading",
      message: "",
      path: null,
    });
    sync.mockResolvedValue({
      folders: 6,
      arrived: 0,
      foldersIncomplete: 0,
      foldersFailed: 0,
    });

    const draw = (generation: number) => (
      <I18nProvider initial="en">
        <MailboxTree
          selected={null}
          onSelect={vi.fn()}
          onAddAccount={vi.fn()}
          onReconnect={vi.fn()}
          generation={generation}
        />
      </I18nProvider>
    );
    const { rerender } = render(draw(0));

    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));

    // The end of that pass is what reaches this window as a change.
    rerender(draw(1));

    // Twice, and by itself: the second attempt is the point.
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(2));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("does not read a busy mailbox as a new one", async () => {
    // An account being read holds its own index, so a sidebar drawn during a pass gets no folders
    // back for it. Read as "this one is new", that asked every account, which held every index,
    // which emptied the sidebar again — nine mailboxes re-read every seven seconds for as long as
    // the window was open. `answered` is the difference, and the core already said it.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [], { answered: false }),
        account("acc-2", "two@example.com", [INBOX]),
      ]),
    );
    renderTree();

    await screen.findByTitle("one@example.com");
    await new Promise((settle) => setTimeout(settle, 80));

    expect(sync).not.toHaveBeenCalled();
  });

  it("still asks about an account that genuinely has none", async () => {
    // The case the ask exists for: its file was read, and there is nothing in it yet.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [], { answered: true }),
        account("acc-2", "two@example.com", [INBOX]),
      ]),
    );
    renderTree();

    await waitFor(() => expect(sync).toHaveBeenCalledWith("acc-1"));
    expect(sync).not.toHaveBeenCalledWith("acc-2");
  });

  it("asks once, not in a loop, while the account is being read", async () => {
    // The pass a freshly added account gets is asked for twice -- once by the account being added,
    // once by this sidebar seeing an account with no folders -- and the second ask is answered
    // "being checked right now". Asking again on the spot is asking the same question of the same
    // pass: it answers the same way for as long as the pass runs, and every turn of it redraws the
    // sidebar and flashes a red line above it.
    tree.mockImplementation(async () =>
      sidebar([account("acc-1", "one@example.com", [])]),
    );
    sync.mockRejectedValue({
      code: "alreadyReading",
      message: "",
      path: null,
    });

    render(
      <I18nProvider initial="en">
        <MailboxTree
          selected={null}
          onSelect={vi.fn()}
          onAddAccount={vi.fn()}
          onReconnect={vi.fn()}
          generation={0}
        />
      </I18nProvider>,
    );

    await waitFor(() => expect(sync).toHaveBeenCalled());
    await new Promise((settle) => setTimeout(settle, 120));
    expect(sync).toHaveBeenCalledTimes(1);
  });

  it("says nothing when an ask nobody made found the account being read", async () => {
    // Nobody pressed anything, so there is no press to explain. What the sidebar wanted is
    // happening, and saying "being checked right now" in red says a new account failed.
    tree.mockImplementation(async () =>
      sidebar([account("acc-1", "one@example.com", [])]),
    );
    sync.mockRejectedValue({
      code: "alreadyReading",
      message: "",
      path: null,
    });

    render(
      <I18nProvider initial="en">
        <MailboxTree
          selected={null}
          onSelect={vi.fn()}
          onAddAccount={vi.fn()}
          onReconnect={vi.fn()}
          generation={0}
        />
      </I18nProvider>,
    );

    await waitFor(() => expect(sync).toHaveBeenCalled());
    await new Promise((settle) => setTimeout(settle, 120));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("says an account is being read rather than showing nothing under it", async () => {
    // A first pass over a hosted server takes the better part of a minute. An account with nothing
    // under it for that long reads as an account that failed.
    tree.mockResolvedValue(sidebar([account("acc-1", "one@example.com", [])]));
    sync.mockResolvedValue({
      folders: 0,
      arrived: 0,
      foldersIncomplete: 0,
      foldersFailed: 0,
    });

    render(
      <I18nProvider initial="en">
        <MailboxTree
          selected={null}
          onSelect={vi.fn()}
          onAddAccount={vi.fn()}
          onReconnect={vi.fn()}
        />
      </I18nProvider>,
    );

    expect(await screen.findByText("Reading folders…")).toBeTruthy();
  });
});

describe("MailboxTree, arranged into groups", () => {
  function group(
    id: string,
    name: string,
    accounts: string[],
    collapsed = false,
  ) {
    return { id, name, place: 0, collapsed, accounts };
  }

  it("gives every row the same surface, in a group or out of one", async () => {
    // What a person points at is the button, and it is the width of the sidebar. Indenting a group
    // with a padded container round its accounts took that width off their rows as well, so the
    // thing that lit up under the pointer began short of the edge and was narrower than every row
    // outside a group.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [SENT]),
      ]),
    );
    arrangement.mockResolvedValue([group("g1", "Work", ["acc-1"])]);

    renderTree();

    const inside = await screen.findByRole("button", { name: /Inbox/ });
    const outside = screen.getByRole("button", { name: /Sent/ });
    const nav = screen.getByRole("navigation", { name: "Mailboxes" });

    // Nothing between the tree and a row may carry a left padding: that is the shape of the bug,
    // and a class on a wrapper is how it comes back.
    for (const row of nav.querySelectorAll("button")) {
      for (
        let above = row.parentElement;
        above !== null && above !== nav;
        above = above.parentElement
      ) {
        expect(
          [...above.classList].filter((one) => /^p[xl]?-/.test(one)),
        ).toEqual([]);
      }
    }

    expect(inside.className).toBe(outside.className);
  });

  it("puts every folder at one level, whoever its account belongs to", async () => {
    // A folder is a folder. Letting the indent add up through the group put the mailboxes of a
    // grouped account a step to the right of everybody else's, so the sidebar had two ladders.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [SENT]),
      ]),
    );
    arrangement.mockResolvedValue([group("g1", "Work", ["acc-1"])]);

    renderTree();

    const grouped = await screen.findByRole("button", { name: /Inbox/ });
    const loose = screen.getByRole("button", { name: /Sent/ });

    const indentOf = (row: HTMLElement) =>
      [...row.classList].find((one) => one.startsWith("ps-"));

    expect(indentOf(grouped)).toBe(indentOf(loose));
    expect(indentOf(grouped)).toBeTruthy();
  });

  /** Opens the menu the way a person does: the second button, on the row itself. */
  function openTheMenuOn(row: HTMLElement) {
    fireEvent.contextMenu(row);
  }

  it("offers what can be done to an account on the row that names it", async () => {
    // The first button selects the account. A menu that took that gesture would leave no way to
    // open the mailbox it is a menu for, so it is on the second one.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [SENT]),
      ]),
    );
    renderTree();

    openTheMenuOn(await screen.findByTitle("one@example.com"));

    expect(
      await screen.findByRole("menuitem", { name: "Get new mail" }),
    ).toBeTruthy();
    expect(
      screen.getByRole("menuitem", { name: "Account settings…" }),
    ).toBeTruthy();
  });

  it("reads that one account when the menu asks for its mail", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [SENT]),
      ]),
    );
    sync.mockResolvedValue({
      folders: 1,
      arrived: 0,
      foldersIncomplete: 0,
      foldersFailed: 0,
    });
    renderTree();

    openTheMenuOn(await screen.findByTitle("two@example.com"));
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Get new mail" }),
    );

    // That account and no other: a menu on one row that read all of them would be a refresh
    // button in a place nobody would look for one.
    await waitFor(() => expect(sync).toHaveBeenCalledWith("acc-2"));
    expect(sync).not.toHaveBeenCalledWith("acc-1");
  });

  it("opens the account's own screen rather than removing it here", async () => {
    // Removing an account is one press further in, on the screen where the sentence about what
    // goes with it is already written. A second way to do it would be a second place for that
    // sentence to fall out of step.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [SENT]),
      ]),
    );
    const { onReconnect } = renderTree();

    openTheMenuOn(await screen.findByTitle("one@example.com"));
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Account settings…" }),
    );

    expect(onReconnect).toHaveBeenCalledWith("acc-1");
    expect(screen.queryByRole("menuitem", { name: /Remove/ })).toBeNull();
  });

  it("does not offer a group to move to when there are none", async () => {
    // A submenu whose one entry is "not in a group" cannot change anything.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [SENT]),
      ]),
    );
    renderTree();

    openTheMenuOn(await screen.findByTitle("one@example.com"));
    await screen.findByRole("menuitem", { name: "Get new mail" });

    expect(screen.queryByText("Move to group")).toBeNull();
  });

  it("moves an account into a group from the row", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [SENT]),
      ]),
    );
    arrangement.mockResolvedValue([group("g1", "Work", ["acc-2"])]);
    regroup.mockResolvedValue(undefined);
    renderTree();

    openTheMenuOn(await screen.findByTitle("one@example.com"));
    fireEvent.click(await screen.findByText("Move to group"));
    const work = await screen.findByRole("menuitemradio", { name: "Work" });
    // A name somebody chose keeps its own direction, as it does in the sidebar.
    expect(screen.getByText("Work", { selector: "[role=menuitemradio] span" }).getAttribute("dir")).toBe("auto");
    fireEvent.click(work);

    await waitFor(() => expect(regroup).toHaveBeenCalledWith("acc-1", "g1"));
  });

  it("takes an account out of every group", async () => {
    // The way back. Without it a mailbox put in a group by accident stays there.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [SENT]),
      ]),
    );
    arrangement.mockResolvedValue([group("g1", "Work", ["acc-1"])]);
    regroup.mockResolvedValue(undefined);
    renderTree();

    openTheMenuOn(await screen.findByTitle("one@example.com"));
    fireEvent.click(await screen.findByText("Move to group"));
    fireEvent.click(
      await screen.findByRole("menuitemradio", { name: "Not in a group" }),
    );

    await waitFor(() => expect(regroup).toHaveBeenCalledWith("acc-1", null));
  });

  it("moves an account down the sidebar from the menu", async () => {
    // A drag is a thing only a mouse can do. The order of somebody's mailboxes is not a setting
    // that should need one.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [SENT]),
        account("acc-3", "three@example.com", [ARCHIVE]),
      ]),
    );
    reorder.mockResolvedValue(undefined);
    renderTree();

    openTheMenuOn(await screen.findByTitle("one@example.com"));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Move down" }));

    await waitFor(() =>
      expect(arrangeTop).toHaveBeenCalledWith([
        { kind: "account", id: "acc-2" },
        { kind: "account", id: "acc-1" },
        { kind: "account", id: "acc-3" },
      ]),
    );
  });

  it("offers no way up from the top and none down from the bottom", async () => {
    // An entry that cannot do anything is worse than one that is not there: a person presses it,
    // nothing happens, and they have learned the menu lies.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [SENT]),
      ]),
    );
    renderTree();

    openTheMenuOn(await screen.findByTitle("one@example.com"));

    const up = await screen.findByRole("menuitem", { name: "Move up" });
    const down = screen.getByRole("menuitem", { name: "Move down" });
    expect(up.getAttribute("aria-disabled")).toBe("true");
    expect(down.getAttribute("aria-disabled")).not.toBe("true");
  });

  it("writes down the order a row was dragged into", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [SENT]),
        account("acc-3", "three@example.com", [ARCHIVE]),
      ]),
    );
    reorder.mockResolvedValue(undefined);
    renderTree();

    const carried = (await screen.findByTitle("three@example.com"))
      .parentElement!.parentElement!;
    const onto =
      screen.getByTitle("one@example.com").parentElement!.parentElement!;

    const carrying = { dataTransfer: { setData: vi.fn(), effectAllowed: "" } };
    fireEvent.dragStart(carried, carrying);
    fireEvent.dragOver(onto, { dataTransfer: { dropEffect: "" } });
    fireEvent.drop(onto);

    await waitFor(() =>
      expect(arrangeTop).toHaveBeenCalledWith([
        { kind: "account", id: "acc-3" },
        { kind: "account", id: "acc-1" },
        { kind: "account", id: "acc-2" },
      ]),
    );
  });

  it("can still be dragged with the window behaving like an application", async () => {
    // The window cancels every drag at the document, so that nothing in the interface can be
    // carried out of it. A test that draws the tree without that is a test of a window nobody
    // runs: this feature passed every one of them and could not be picked up at all.
    const undo = applyNativeFeel();
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [SENT]),
      ]),
    );
    reorder.mockResolvedValue(undefined);
    renderTree();

    const carried = (await screen.findByTitle("two@example.com")).parentElement!
      .parentElement!;
    const onto = screen.getByTitle("one@example.com").parentElement!
      .parentElement!;

    const began = fireEvent.dragStart(carried, {
      dataTransfer: { setData: vi.fn(), effectAllowed: "" },
    });
    expect(began, "the drag was cancelled before it began").toBe(true);

    fireEvent.dragOver(onto, { dataTransfer: { dropEffect: "" } });
    fireEvent.drop(onto);

    await waitFor(() =>
      expect(arrangeTop).toHaveBeenCalledWith([
        { kind: "account", id: "acc-2" },
        { kind: "account", id: "acc-1" },
      ]),
    );
    undo();
  });

  it("does not reorder across a group boundary", async () => {
    // Where an account belongs is the group menu's decision. One gesture may not make two.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [SENT]),
      ]),
    );
    arrangement.mockResolvedValue([group("g1", "Work", ["acc-1"])]);
    renderTree();

    const carried = (await screen.findByTitle("two@example.com")).parentElement!
      .parentElement!;
    const onto =
      screen.getByTitle("one@example.com").parentElement!.parentElement!;

    fireEvent.dragStart(carried, {
      dataTransfer: { setData: vi.fn(), effectAllowed: "" },
    });
    fireEvent.dragOver(onto, { dataTransfer: { dropEffect: "" } });
    fireEvent.drop(onto);

    // Given time to happen, and then shown not to have. A bare assertion here would pass before
    // the drop had a chance to write anything.
    await new Promise((settle) => setTimeout(settle, 60));
    expect(reorder).not.toHaveBeenCalled();
    expect(arrangeTop).not.toHaveBeenCalled();
  });

  /** The labels of what is drawn at the top of the sidebar and in it, in the order drawn. */
  function drawnInOrder(): string[] {
    return screen
      .getAllByRole("button")
      .map((button) => button.getAttribute("title") ?? button.textContent ?? "")
      .filter((label) => /Work|@example\.com/.test(label))
      .map((label) => (label.includes("Work") ? "Work" : label));
  }

  it("draws an account in no group above a group when that is where it was put", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX], { place: 0 }),
        account("acc-2", "two@example.com", [SENT], { place: 0 }),
      ]),
    );
    arrangement.mockResolvedValue([
      { ...group("g1", "Work", ["acc-2"]), place: 1 },
    ]);
    renderTree();

    await screen.findByRole("button", { name: /Work/ });

    expect(drawnInOrder()).toEqual([
      "one@example.com",
      "Work",
      "two@example.com",
    ]);
  });

  it("draws a group first where it and an account were given the same place", async () => {
    // What every sidebar showed before the two were counted together.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX], { place: 0 }),
        account("acc-2", "two@example.com", [SENT], { place: 0 }),
      ]),
    );
    arrangement.mockResolvedValue([group("g1", "Work", ["acc-2"])]);
    renderTree();

    await screen.findByRole("button", { name: /Work/ });

    expect(drawnInOrder()).toEqual([
      "Work",
      "two@example.com",
      "one@example.com",
    ]);
  });

  it("moves an account in no group past a group from the menu, and shows it there at once", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX], { place: 0 }),
        account("acc-2", "two@example.com", [SENT], { place: 1 }),
      ]),
    );
    arrangement.mockResolvedValue([
      { ...group("g1", "Work", ["acc-1"]), place: 0 },
    ]);
    renderTree();

    openTheMenuOn(await screen.findByTitle("two@example.com"));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Move up" }));

    await waitFor(() =>
      expect(arrangeTop).toHaveBeenCalledWith([
        { kind: "account", id: "acc-2" },
        { kind: "group", id: "g1" },
      ]),
    );
    expect(drawnInOrder()).toEqual([
      "two@example.com",
      "Work",
      "one@example.com",
    ]);
    expect(reorder).not.toHaveBeenCalled();
  });

  it("puts an account in no group where a group stands when it is dropped on the group", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX], { place: 0 }),
        account("acc-2", "two@example.com", [SENT], { place: 1 }),
      ]),
    );
    arrangement.mockResolvedValue([
      { ...group("g1", "Work", ["acc-1"]), place: 0 },
    ]);
    renderTree();

    const carried = (await screen.findByTitle("two@example.com")).parentElement!
      .parentElement!;
    const onto = await screen.findByRole("button", { name: /Work/ });

    fireEvent.dragStart(carried, {
      dataTransfer: { setData: vi.fn(), effectAllowed: "" },
    });
    fireEvent.dragOver(onto, { dataTransfer: { dropEffect: "" } });
    fireEvent.drop(onto);

    await waitFor(() =>
      expect(arrangeTop).toHaveBeenCalledWith([
        { kind: "account", id: "acc-2" },
        { kind: "group", id: "g1" },
      ]),
    );
  });

  it("does not let an account in a group be dropped on a group", async () => {
    // Out of a group and into another place is two decisions, and a drop makes one.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [SENT]),
      ]),
    );
    arrangement.mockResolvedValue([
      { ...group("g1", "Work", ["acc-1"]), place: 0 },
      { ...group("g2", "Home", ["acc-2"]), place: 1 },
    ]);
    renderTree();

    const carried = (await screen.findByTitle("one@example.com")).parentElement!
      .parentElement!;
    const onto = await screen.findByRole("button", { name: /Home/ });

    fireEvent.dragStart(carried, {
      dataTransfer: { setData: vi.fn(), effectAllowed: "" },
    });
    const offered = fireEvent.dragOver(onto, {
      dataTransfer: { dropEffect: "" },
    });
    fireEvent.drop(onto);

    // A drag over a place that takes nothing is not cancelled, which is how the window says no.
    expect(offered).toBe(true);
    await new Promise((settle) => setTimeout(settle, 60));
    expect(arrangeTop).not.toHaveBeenCalled();
    expect(reorder).not.toHaveBeenCalled();
  });

  it("draws a group over the mailboxes in it, and leaves the rest where they were", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
        account("acc-3", "three@example.com", []),
      ]),
    );
    arrangement.mockResolvedValue([group("g1", "Work", ["acc-1", "acc-2"])]);

    renderTree();

    await screen.findByRole("button", { name: /Work/ });

    // The mailboxes in the group are still drawn: a group is an arrangement, not a place mail
    // goes to hide.
    expect(screen.getByTitle("one@example.com")).toBeTruthy();
    expect(screen.getByTitle("two@example.com")).toBeTruthy();
    // And the one nobody grouped is still there, unmoved.
    expect(screen.getByTitle("three@example.com")).toBeTruthy();
  });

  it("shows the unread of everything in the group on the group itself", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX], { unread: 4 }),
        account("acc-2", "two@example.com", [OTHER_INBOX], { unread: 3 }),
        account("acc-3", "three@example.com", [], { unread: 90 }),
      ]),
    );
    arrangement.mockResolvedValue([group("g1", "Work", ["acc-1", "acc-2"])]);

    renderTree();

    const row = await screen.findByRole("button", { name: /Work/ });

    // Seven, not ninety-seven: the number on a group is the mail that group would show, and one
    // that counted the whole device would send somebody looking for four messages that are not
    // in it.
    expect(row.textContent).toContain("7");
  });

  it("moves the numbers in the frame a message is marked, before the sidebar reads again", async () => {
    // The dot on the row goes under the finger. A number beside the inbox, on the account and on
    // every inbox together that waits for the sidebar to be read again says two things at once.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX], { unread: 2 }),
        account("acc-2", "two@example.com", [OTHER_INBOX], { unread: 3 }),
      ]),
    );
    arrangement.mockResolvedValue([]);
    const props = {
      selected: null,
      onSelect: vi.fn(),
      onAddAccount: vi.fn(),
      onReconnect: vi.fn(),
    };
    const view = render(
      <I18nProvider initial="en">
        <MailboxTree {...props} />
      </I18nProvider>,
    );
    const everything = await screen.findByRole("button", {
      name: /All inboxes/,
    });
    expect(everything.textContent).toContain("5");

    view.rerender(
      <I18nProvider initial="en">
        <MailboxTree
          {...props}
          readChanges={[
            {
              folderId: INBOX.folderId,
              unread: -1,
              asked: null,
              askedAt: performance.now(),
            },
          ]}
        />
      </I18nProvider>,
    );

    expect(everything.textContent).toContain("4");
    expect(tree).toHaveBeenCalledTimes(1);
  });

  it("moves a number once for a change that a read already on its way came back holding", async () => {
    // A read set off before the change was asked for, which the core answers after writing it
    // down: its counts hold the change. Ordered by when the read began, the change would be laid
    // over them again, and the numbers would go 5, 4, 3, 4 for one message.
    tree.mockResolvedValueOnce(
      sidebar([
        account("acc-1", "one@example.com", [INBOX], { unread: 2 }),
        account("acc-2", "two@example.com", [OTHER_INBOX], { unread: 3 }),
      ]),
    );
    arrangement.mockResolvedValue([]);
    let answer: (tree: MailboxTreeDto) => void = () => {};
    tree.mockImplementation(
      () => new Promise<MailboxTreeDto>((resolve) => (answer = resolve)),
    );
    const draw = (generation: number, readChanges: ReadChange[]) => (
      <I18nProvider initial="en">
        <MailboxTree
          selected={null}
          onSelect={vi.fn()}
          onAddAccount={vi.fn()}
          onReconnect={vi.fn()}
          generation={generation}
          readChanges={readChanges}
        />
      </I18nProvider>
    );
    const view = render(draw(0, []));
    const everything = await screen.findByRole("button", {
      name: /All inboxes/,
    });
    expect(everything.textContent).toContain("5");

    // The read on its way first, and the change asked for after it began.
    view.rerender(draw(1, []));
    await waitFor(() => expect(tree).toHaveBeenCalledTimes(2));
    const marked: ReadChange = {
      folderId: INBOX.folderId,
      unread: -1,
      asked: null,
      askedAt: performance.now(),
    };
    view.rerender(draw(1, [marked]));
    expect(everything.textContent).toContain("4");

    // Written as the seventh thing asked for, and the read holds it.
    view.rerender(draw(1, [{ ...marked, asked: 7 }]));
    await act(async () =>
      answer({
        accounts: [
          account("acc-1", "one@example.com", [INBOX], { unread: 1 }),
          account("acc-2", "two@example.com", [OTHER_INBOX], { unread: 3 }),
        ],
        askedThrough: 7,
      }),
    );

    expect(everything.textContent).toContain("4");
    expect(everything.textContent).not.toContain("3");
  });

  it("asks for the group's mail when the group is pressed", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
      ]),
    );
    arrangement.mockResolvedValue([group("g1", "Work", ["acc-1"])]);

    const { onSelect } = renderTree();

    fireEvent.click(await screen.findByRole("button", { name: /Work/ }));

    expect(onSelect).toHaveBeenCalledWith({ kind: "group", groupId: "g1" });
  });

  it("hides the mailboxes of a folded group and keeps the group itself", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
      ]),
    );
    arrangement.mockResolvedValue([group("g1", "Work", ["acc-1"], true)]);

    renderTree();

    await screen.findByRole("button", { name: /Work/ });

    // Folding is the whole of what a group of twenty mailboxes is for. The one outside it stays.
    expect(screen.queryByTitle("one@example.com")).toBeNull();
    expect(screen.getByTitle("two@example.com")).toBeTruthy();
  });

  it("writes down a fold so the next start opens as it was left", async () => {
    tree.mockResolvedValue(
      sidebar([account("acc-1", "one@example.com", [INBOX])]),
    );
    arrangement.mockResolvedValue([group("g1", "Work", ["acc-1"])]);

    renderTree();

    fireEvent.click(
      await screen.findByRole("button", { name: /hide mailboxes/i }),
    );

    await waitFor(() => expect(foldGroup).toHaveBeenCalledWith("g1", true));
    // And on screen at once, rather than after the write comes back: a fold that waited for the
    // disk would be a triangle that turns a moment after the click.
    expect(screen.queryByTitle("one@example.com")).toBeNull();
  });

  it("says a group is empty rather than drawing it as a blank", async () => {
    tree.mockResolvedValue(
      sidebar([account("acc-1", "one@example.com", [INBOX])]),
    );
    arrangement.mockResolvedValue([group("g1", "Work", [])]);

    renderTree();

    // A gap under a name reads as mailboxes that failed to load.
    expect(await screen.findByText(/nothing in this group yet/i)).toBeTruthy();
  });

  it("draws a group naming a mailbox that is gone, without the mailbox", async () => {
    // Removing an account does not walk the arrangements it was in, so this is the ordinary
    // state after somebody removes a grouped mailbox — not a corruption to report.
    tree.mockResolvedValue(
      sidebar([account("acc-1", "one@example.com", [INBOX])]),
    );
    arrangement.mockResolvedValue([
      group("g1", "Work", ["acc-1", "acc-removed"]),
    ]);

    renderTree();

    expect(await screen.findByRole("button", { name: /Work/ })).toBeTruthy();
    expect(screen.getByTitle("one@example.com")).toBeTruthy();
  });

  it("shows every mailbox when the arrangement cannot be read", async () => {
    // A store that answers about mailboxes and not about groups is a sidebar with every mailbox
    // in it, which is what it looked like before anybody made a group. Showing nothing would be
    // an application that lost somebody's mail over a failed read of a list of names.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
      ]),
    );
    arrangement.mockRejectedValue({
      code: "storageUnavailable",
      message: "",
      path: null,
    });

    renderTree();

    expect(await screen.findByTitle("one@example.com")).toBeTruthy();
    expect(screen.getByTitle("two@example.com")).toBeTruthy();
  });
});

describe("MailboxTree, when a group stops existing", () => {
  it("leaves a group that is gone rather than reading an empty list under it", async () => {
    // Removed in the settings screen while it was the thing being read. Left alone, the list
    // shows nothing with nothing highlighted to say why, and the only way back is to notice that
    // and click something else.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
      ]),
    );
    arrangement.mockResolvedValue([]);

    const { onSelect } = renderTree({ kind: "group", groupId: "g-gone" });

    await waitFor(() =>
      expect(onSelect).toHaveBeenCalledWith({ kind: "everything" }),
    );
  });

  it("removes a group from its row's menu once its name is typed, and its mailboxes stay", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
      ]),
    );
    arrangement.mockResolvedValue([
      { id: "g1", name: "Work", place: 0, collapsed: false, accounts: ["acc-1"] },
    ]);
    const onArrangementChanged = vi.fn();
    render(
      <I18nProvider initial="en">
        <MailboxTree
          selected={null}
          onSelect={vi.fn()}
          onAddAccount={vi.fn()}
          onReconnect={vi.fn()}
          onArrangementChanged={onArrangementChanged}
        />
      </I18nProvider>,
    );

    fireEvent.contextMenu(await screen.findByRole("button", { name: /Work/ }));
    expect(
      await screen.findByRole("menu", { name: "Actions for the group Work" }),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("menuitem", { name: "Remove" }));
    const field = await screen.findByRole("textbox", { name: "Group name" });
    // The menu takes the focus back on its way out, a moment after it closes. Opened at the press,
    // the panel lost its field's focus to it.
    await waitFor(() => expect(document.activeElement).toBe(field));
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(document.activeElement).toBe(field);
    const remove = screen.getByRole("button", { name: "Remove" }) as HTMLButtonElement;
    expect(remove.disabled).toBe(true);
    // The group holds a mailbox, so the panel says where it goes.
    expect(
      screen.getByText("The mailboxes in this group stay in the sidebar where the group was, with all their mail"),
    ).toBeTruthy();
    expect(dropGroup).not.toHaveBeenCalled();
    arrangement.mockResolvedValue([]);

    fireEvent.change(field, { target: { value: "Work" } });
    fireEvent.click(remove);

    await waitFor(() => expect(dropGroup).toHaveBeenCalledWith("g1"));
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: /Work/ })).toBeNull(),
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByTitle("one@example.com")).toBeTruthy();
    expect(onArrangementChanged).toHaveBeenCalledTimes(1);
  });

  it("stays on a group that is still there", async () => {
    tree.mockResolvedValue(
      sidebar([account("acc-1", "one@example.com", [INBOX])]),
    );
    arrangement.mockResolvedValue([
      { id: "g1", name: "Work", place: 0, collapsed: false, accounts: ["acc-1"] },
    ]);

    const { onSelect } = renderTree({ kind: "group", groupId: "g1" });

    await screen.findByRole("button", { name: /Work/ });
    expect(onSelect).not.toHaveBeenCalled();
  });
});

describe("MailboxTree, when the arrangement cannot be read", () => {
  it("stays on the group being read rather than treating a failed read as none", async () => {
    // "The read failed" and "there are no groups" are one value apart, and reading the first as
    // the second throws somebody off the mail they were looking at because the store was busy.
    // Two, because one account is drawn as its folders alone and has no row to look for.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "one@example.com", [INBOX]),
        account("acc-2", "two@example.com", [OTHER_INBOX]),
      ]),
    );
    arrangement.mockRejectedValue({
      code: "storageUnavailable",
      message: "",
      path: null,
    });

    const { onSelect } = renderTree({ kind: "group", groupId: "g1" });

    await screen.findByTitle("one@example.com");
    expect(onSelect).not.toHaveBeenCalled();
  });
});

describe("movedWithin", () => {
  const run = ["a", "b", "c"];

  it("takes a row out and puts it back where it was dropped", () => {
    expect(movedWithin(run, "c", 0)).toEqual(["c", "a", "b"]);
    expect(movedWithin(run, "a", 2)).toEqual(["b", "c", "a"]);
  });

  it("answers with nothing where there is nowhere to go", () => {
    // The first row asked to go up and the last asked to go down. Answering with the same order
    // would be a write nobody asked for on every press.
    expect(movedWithin(run, "a", -1)).toBeNull();
    expect(movedWithin(run, "c", 3)).toBeNull();
    expect(movedWithin(run, "b", 1)).toBeNull();
  });

  it("answers with nothing for a row that is not in the run", () => {
    expect(movedWithin(run, "d", 0)).toBeNull();
  });

  it("keeps every row it was given", () => {
    // A reorder that dropped one would take a mailbox off the screen until the next start.
    const ordered = movedWithin(run, "b", 0);
    expect([...(ordered ?? [])].sort()).toEqual([...run].sort());
  });
});

describe("MailboxTree, folded into a rail", () => {
  function renderRail(selected: Mailbox | null = null, language = "en") {
    const onSelect = vi.fn();
    const onUnfold = vi.fn();
    render(
      <I18nProvider initial={language}>
        <MailboxTree
          selected={selected}
          onSelect={onSelect}
          onAddAccount={vi.fn()}
          onReconnect={vi.fn()}
          folded
          onUnfold={onUnfold}
        />
      </I18nProvider>,
    );
    return { onSelect, onUnfold };
  }

  /** A folder for every role there is, and the icon each is drawn as. */
  const EVERY_ROLE = [
    { role: "inbox", name: "Inbox", icon: "lucide-inbox" },
    { role: "sent", name: "Sent", icon: "lucide-send" },
    { role: "drafts", name: "Drafts", icon: "lucide-file-pen" },
    { role: "trash", name: "Trash", icon: "lucide-trash-2" },
    { role: "junk", name: "Junk", icon: "lucide-octagon-alert" },
    { role: "archive", name: "Archive", icon: "lucide-archive" },
    { role: "allMail", name: "All mail", icon: "lucide-mails" },
    { role: "flagged", name: "Flagged", icon: "lucide-flag" },
    // A folder somebody made, which the server names and this build has no icon for.
    { role: "custom", name: "Receipts", icon: "lucide-folder" },
  ] as const;

  it("draws every folder as the icon of what it is for, and one it does not know as a folder", async () => {
    tree.mockResolvedValue(
      sidebar([
        account(
          "acc-1",
          "zeta@example.com",
          EVERY_ROLE.map(({ role, name }, at) => ({
            folderId: String(at + 1),
            name,
            role,
            total: 0,
            unread: 0,
            draftsHere: 0,
            canReceive: true,
          })),
        ),
      ]),
    );
    renderRail();

    for (const { name, icon } of EVERY_ROLE) {
      const button = await screen.findByRole("button", { name });
      const drawn = button.querySelectorAll("svg");
      expect(drawn.length, name).toBe(1);
      expect(drawn[0]!.classList.contains(icon), `${name} as ${icon}`).toBe(
        true,
      );
    }
  });

  it("gets mail first, and leaves out what adds an account and what folds", async () => {
    tree.mockResolvedValue(
      sidebar([account("acc-1", "zeta@example.com", [INBOX, ARCHIVE])]),
    );
    renderRail();

    await screen.findByRole("button", { name: /^Inbox/ });
    const buttons = screen.getAllByRole("button");
    expect(buttons[0]).toBe(
      screen.getByRole("button", { name: "Get new mail" }),
    );
    expect(
      screen.queryByRole("button", { name: "Add an account or a group" }),
    ).toBeNull();
    expect(screen.queryByText("Mailboxes")).toBeNull();
  });

  it("gets mail for every account at once, as the heading's button does, and once", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "zeta@example.com", [INBOX]),
        account("acc-2", "kappa@example.net", [OTHER_INBOX]),
      ]),
    );
    let finish: () => void = () => {};
    sync.mockReturnValue(
      new Promise((resolve) => {
        finish = () =>
          resolve({
            folders: 0,
            arrived: 0,
            foldersFailed: 0,
            foldersIncomplete: 0,
          });
      }),
    );
    renderRail();

    const getMail = await screen.findByRole("button", { name: "Get new mail" });
    fireEvent.click(getMail);

    expect(sync).toHaveBeenCalledTimes(2);
    expect(sync).toHaveBeenCalledWith("acc-1");
    expect(sync).toHaveBeenCalledWith("acc-2");
    // Turning while it works, and a second press refused rather than asking again.
    const working = screen.getByRole("button", { name: "Checking the servers…" });
    expect(working.getAttribute("aria-busy")).toBe("true");
    expect(working.querySelector("svg")!.classList.contains("animate-spin")).toBe(
      true,
    );
    fireEvent.click(working);
    expect(sync).toHaveBeenCalledTimes(2);

    await act(async () => finish());
  });

  it("marks the mailbox being read, and says what is unread in the hint", async () => {
    tree.mockResolvedValue(
      sidebar([account("acc-1", "zeta@example.com", [INBOX, ARCHIVE])]),
    );
    renderRail({
      kind: "folder",
      accountId: "acc-1",
      folderId: INBOX.folderId,
      role: "inbox",
    });

    const inbox = await screen.findByRole("button", { name: "Inbox 2 unread" });
    expect(inbox.getAttribute("aria-current")).toBe("true");
    // A dot where the open sidebar has the number.
    expect(inbox.querySelector("span[aria-hidden=true].rounded-full")).toBeTruthy();

    const archive = screen.getByRole("button", { name: "Archive" });
    expect(archive.getAttribute("aria-current")).toBeNull();
    expect(archive.querySelector("span[aria-hidden=true].rounded-full")).toBeNull();

    act(() => inbox.focus());
    const hint = await screen.findByRole("tooltip");
    expect(hint.textContent).toContain("Inbox");
    expect(hint.textContent).toContain("2 unread");
  });

  it("chooses the mailbox an icon stands for, as the row does", async () => {
    tree.mockResolvedValue(
      sidebar([account("acc-1", "zeta@example.com", [INBOX, ARCHIVE])]),
    );
    const { onSelect } = renderRail({
      kind: "folder",
      accountId: "acc-1",
      folderId: INBOX.folderId,
      role: "inbox",
    });

    fireEvent.click(await screen.findByRole("button", { name: "Archive" }));

    expect(onSelect).toHaveBeenLastCalledWith({
      kind: "folder",
      accountId: "acc-1",
      folderId: ARCHIVE.folderId,
      role: "archive",
    });
  });

  it("offers All inboxes alone with several accounts, and leaves what was chosen chosen", async () => {
    // The same icons once for every account, with nothing to say whose they are, are icons nobody
    // can choose between. What was chosen with the sidebar open goes on being shown.
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "zeta@example.com", [INBOX, ARCHIVE], {
          displayName: "Zeta Quorin",
          unread: 2,
        }),
        account("acc-2", "kappa@example.net", [OTHER_INBOX], { unread: 1 }),
      ]),
    );
    const { onSelect } = renderRail({
      kind: "folder",
      accountId: "acc-1",
      folderId: ARCHIVE.folderId,
      role: "archive",
    });

    const everything = await screen.findByRole("button", {
      name: "All inboxes 3 unread",
    });
    expect(everything.querySelector("svg.lucide-inbox")).toBeTruthy();
    expect(everything.getAttribute("aria-current")).toBeNull();
    expect(screen.getAllByRole("button")).toEqual([
      screen.getByRole("button", { name: "Get new mail" }),
      everything,
    ]);
    expect(onSelect).not.toHaveBeenCalled();

    fireEvent.click(everything);
    expect(onSelect).toHaveBeenLastCalledWith({ kind: "everything" });
  });

  it("marks All inboxes while it is what is being read", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "zeta@example.com", [INBOX]),
        account("acc-2", "kappa@example.net", [OTHER_INBOX]),
      ]),
    );
    renderRail({ kind: "everything" });

    const everything = await screen.findByRole("button", {
      name: /^All inboxes/,
    });
    expect(everything.getAttribute("aria-current")).toBe("true");
  });

  it("draws the folders of one account in a group, and not the group", async () => {
    // One account is one account however it is arranged: its folders are what an icon can stand for.
    tree.mockResolvedValue(
      sidebar([account("acc-1", "zeta@example.com", [INBOX, ARCHIVE])]),
    );
    arrangement.mockResolvedValue([
      { id: "g-1", name: "Work", place: 0, collapsed: true, accounts: ["acc-1"] },
    ]);
    renderRail();

    expect(await screen.findByRole("button", { name: /^Inbox/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Archive" })).toBeTruthy();
    await waitFor(() => expect(arrangement).toHaveBeenCalled());
    expect(screen.queryByRole("button", { name: /Work/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /mailboxes|folders of/i })).toBeNull();
  });

  it("says an account whose mail could not be read with a mark, among several", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "zeta@example.com", [INBOX]),
        account("acc-2", "kappa@example.net", [], { answered: false }),
      ]),
    );
    const { onUnfold } = renderRail();

    const mark = within(await screen.findByRole("alert")).getByRole("button");
    expect(mark.textContent).toContain("could not be read");
    expect(mark.textContent).toContain("kappa@example.net");
    expect(screen.getByRole("button", { name: /^All inboxes/ })).toBeTruthy();

    fireEvent.click(mark);
    expect(onUnfold).toHaveBeenCalledTimes(1);
  });

  it("says a failed check with a mark whose press opens the sidebar", async () => {
    tree.mockResolvedValue(
      sidebar([account("acc-1", "zeta@example.com", [INBOX])]),
    );
    sync.mockRejectedValue({ code: "offline", message: "", path: null });
    const { onUnfold } = renderRail();

    fireEvent.click(await screen.findByRole("button", { name: "Get new mail" }));

    const said = await screen.findByRole("alert");
    const mark = within(said).getByRole("button");
    // The sentence the open sidebar says, with the account it is about under it.
    expect(mark.textContent).toContain("zeta@example.com");
    expect(mark.className).toContain("text-destructive");

    fireEvent.click(mark);
    expect(onUnfold).toHaveBeenCalledTimes(1);
  });

  it("names the account once in a password that was refused", async () => {
    tree.mockResolvedValue(
      sidebar([account("acc-1", "zeta@example.com", [INBOX])]),
    );
    sync.mockRejectedValue({ code: "unauthorized", message: "", path: null });
    renderRail();

    fireEvent.click(await screen.findByRole("button", { name: "Get new mail" }));

    const mark = within(await screen.findByRole("alert")).getByRole("button");
    expect(mark.textContent).toBe(
      "The server would not take the password for zeta@example.com",
    );
  });

  it("says an account whose mail could not be read with a mark", async () => {
    tree.mockResolvedValue(
      sidebar([
        account("acc-1", "zeta@example.com", [], { answered: false }),
      ]),
    );
    const { onUnfold } = renderRail();

    const mark = within(await screen.findByRole("alert")).getByRole("button");
    expect(mark.textContent).toContain("could not be read");
    expect(mark.textContent).toContain("zeta@example.com");

    fireEvent.click(mark);
    expect(onUnfold).toHaveBeenCalledTimes(1);
  });

  it("says nothing about an account whose folders are on their way", async () => {
    // A wait that ends by itself: a mark for it would be a mark that comes and goes on every start.
    tree.mockResolvedValue(
      sidebar([account("acc-1", "zeta@example.com", [], { answered: true })]),
    );
    renderRail();

    await screen.findByRole("button", { name: "Get new mail" });
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByText(/Reading folders/i)).toBeNull();
  });

  it("says a sidebar that could not be read with a mark", async () => {
    tree.mockRejectedValue({ code: "internal", message: "", path: null });
    const { onUnfold } = renderRail();

    const mark = within(await screen.findByRole("alert")).getByRole("button");
    fireEvent.click(mark);
    expect(onUnfold).toHaveBeenCalledTimes(1);
  });
});
