import { CircleAlert, Plus, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  folderBadge,
  folderName,
  unreadBadge,
  withReadChanges,
  type ReadChange,
} from "@/entities/folder";
import { isSameMailbox, type Mailbox } from "@/entities/mailbox";
import { describeError, useTranslation, type Translator } from "@/shared/i18n";
import {
  ContextMenu,
  ContextMenuTrigger,
} from "@/shared/ui/primitives/context-menu";
import {
  arrangeSidebar,
  getMailboxTree,
  isCommandError,
  listAccountGroups,
  reorderAccounts,
  setAccountCollapsed,
  setAccountGroupCollapsed,
  syncAccount,
} from "@/shared/ipc/client";
import { ColumnHeading } from "@/shared/ui/ColumnHeading";
import { AccountMenu, movedWithin, type AccountActions } from "./AccountMenu";
import { Folding, FOLDER, INDENT, Rail, RailRule, Row } from "./Row";
import { Button } from "@/shared/ui/primitives/button";
import { RailButton } from "@/shared/ui/RailButton";
import type { AccountGroupDto } from "@/shared/ipc/generated/AccountGroupDto";
import type { AccountNodeDto } from "@/shared/ipc/generated/AccountNodeDto";
import type { FolderDto } from "@/shared/ipc/generated/FolderDto";
import type { CommandError } from "@/shared/ipc/generated/CommandError";
import type { PlacedDto } from "@/shared/ipc/generated/PlacedDto";

/**
 * The sidebar.
 *
 * One account is shown as its folders alone: a tree with a single root is a row of wasted space
 * and a fold that does nothing. From the second account on, each gets a row of its own that can be
 * folded away, and a row above them all reaches every message on the device.
 */
/**
 * The folder an account opens on.
 *
 * By role rather than by position: a server lists its mailboxes in whatever order it likes, and
 * the one this is developed against lists the inbox last. Nothing when the account has no folder
 * marked as one, which is a server this build does not understand rather than an account without
 * an inbox -- the caller then opens the account instead of a folder.
 */
function inboxOf(account: AccountNodeDto): FolderDto | undefined {
  return account.folders.find((folder) => folder.role === "inbox");
}

/**
 * What a dragged row carries, under a type no other application knows.
 *
 * The drop decides from what this window is already holding, so nothing reads it back — it is here
 * because a drag with an empty payload never produces a drop at all.
 */
const CARRYING_AN_ACCOUNT = "application/x-lyonoyl-account";

/** No changes, as one value, so a tree given none does not lay them over its counts every draw. */
const NO_CHANGES: ReadChange[] = [];

export function MailboxTree({
  selected,
  onSelect,
  onAddAccount,
  onReconnect,
  openInboxOf = null,
  onInboxOpened,
  generation = 0,
  readChanges = NO_CHANGES,
  recovering = null,
  folded = false,
  onUnfold,
}: {
  /**
   * Drawn in the folded sidebar: the button that gets mail, and the mailboxes an icon can stand
   * for — the folders of the only account, or with several accounts All inboxes alone. Nothing
   * that adds an account, and no chevrons: the folds are the open sidebar's, and so is the plus.
   */
  folded?: boolean;
  /**
   * Opens the sidebar, for a mark in the folded one that stands for something said in words: the
   * words, and what can be done about them, are in the open sidebar.
   */
  onUnfold?: () => void;
  /** What the window's recovery screen is up for, when it is up. */
  recovering?:
    | { kind: "command"; error: CommandError }
    | { kind: "bridge" }
    | null;
  selected: Mailbox | null;
  onSelect: (mailbox: Mailbox) => void;
  onAddAccount: () => void;
  /** Asked for an account whose password is no longer in the store this device keeps it in. */
  onReconnect: (accountId: string) => void;
  /**
   * An account just connected, whose inbox is to be opened.
   *
   * Named by account rather than by folder because the caller does not know which folder that is:
   * a mailbox that has just arrived is described here, once this has read itself again.
   */
  openInboxOf?: string | null;
  onInboxOpened?: () => void;
  /** Bumped by the caller when the index changed, which is when this reads itself again. */
  generation?: number;
  /** Messages marked read or unread that the counts read last may not hold yet. */
  readChanges?: ReadChange[];
}) {
  const t = useTranslation();
  const [accounts, setAccounts] = useState<AccountNodeDto[]>([]);
  /** How far the queue had got when the counts on screen were read, which says what they hold. */
  const [readThrough, setReadThrough] = useState(0);
  /**
   * The arrangement, or `null` while it has not been read yet.
   *
   * Not an empty list for both: "nobody has made a group" and "we have not looked" are the same
   * value read one way, and a selection standing on a group was thrown away on every first draw
   * because of it.
   */
  const [groups, setGroups] = useState<AccountGroupDto[] | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  /** A server that could not be reached. The tree stays on screen; only this line changes. */
  const [syncFailure, setSyncFailure] = useState<{
    accountId: string;
    error: CommandError;
  } | null>(null);
  const [syncing, setSyncing] = useState(false);
  /** Which account is being carried by a drag, and which row it is over. */
  const [carried, setCarried] = useState<string | null>(null);
  const [over, setOver] = useState<string | null>(null);
  /** Accounts already asked for their mailboxes, so a server with none is not asked forever. */
  const asked = useRef(new Set<string>());
  /**
   * Accounts asked while a pass over them was already running.
   *
   * They count as asked, because what the ask wanted is happening — and asking again on the spot
   * is putting the same question to the same pass, which answers the same way until it ends. They
   * are forgotten when something changes, so an account whose pass brought nothing is asked again
   * rather than sitting empty for the session.
   */
  const collided = useRef(new Set<string>());
  const [reloads, setReloads] = useState(0);
  /**
   * Whether a read is on its way, and whether another was asked for while it was.
   *
   * One read at a time and one behind it. Every change a person makes asks for the tree again, and
   * a key held down over a folder is thirty of them a second: started each time, the core reads
   * every account thirty times a second to answer questions only the last of which is looked at.
   * The one behind starts when the one on its way lands, so the last change is always read.
   */
  const reading = useRef({ now: false, behind: false });
  const [readAgain, setReadAgain] = useState(0);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    if (reading.current.now) {
      reading.current.behind = true;
      return;
    }
    reading.current.now = true;
    getMailboxTree()
      .then((tree) => {
        if (!mounted.current) return;
        setAccounts(tree.accounts);
        setReadThrough(tree.askedThrough);
        setError(null);
      })
      .catch((reason: unknown) => {
        if (!mounted.current) return;
        setError(
          isCommandError(reason)
            ? reason
            : { code: "internal", message: "", path: null },
        );
      })
      .finally(() => {
        reading.current.now = false;
        if (!reading.current.behind || !mounted.current) return;
        reading.current.behind = false;
        setReadAgain((again) => again + 1);
      });
    // The arrangement is read beside the accounts and never instead of them: a store that
    // answers about mailboxes and not about groups is a sidebar with every mailbox in it, which
    // is what it looked like before anybody made a group. An arrangement nobody can read is not a
    // reason to show nothing.
    listAccountGroups()
      .then((arrangement) => {
        if (mounted.current) setGroups(arrangement);
      })
      .catch(() => {
        // Left unread rather than set to none. A read that failed says nothing about whether
        // somebody has groups, and writing it down as "none" is what would throw a person off
        // the group they were reading because the store was busy for a moment.
        if (mounted.current) setGroups(null);
      });
  }, [reloads, readAgain]);

  // Read again when a pass changed something, rather than built again. Built again — which is what
  // a changing key did — throws away the two things here that exist to last: the set of accounts
  // already asked for their mailboxes, whose whole point is that a server with none is not asked
  // over and over, and the line saying a server could not be reached, which vanished the moment any
  // other account reported anything.
  const firstGeneration = useRef(generation);
  useEffect(() => {
    if (generation === firstGeneration.current) return;
    firstGeneration.current = generation;
    for (const accountId of collided.current) asked.current.delete(accountId);
    collided.current.clear();
    setReloads((reload) => reload + 1);
  }, [generation]);

  /** Folded away or open, written down at once so the next start opens as it was left. */
  const foldGroup = useCallback((groupId: string, collapsed: boolean) => {
    setGroups((current) =>
      current === null
        ? current
        : current.map((group) =>
            group.id === groupId ? { ...group, collapsed } : group,
          ),
    );
    void setAccountGroupCollapsed(groupId, collapsed).catch(() => {
      // A fold that could not be written down is a fold that will not be there next time, which
      // is a small thing to be told about in a red panel. The sidebar keeps it for this session.
    });
  }, []);

  const failed = useCallback((accountId: string, reason: unknown) => {
    setSyncFailure({
      accountId,
      error: isCommandError(reason)
        ? reason
        : { code: "internal", message: "", path: null },
    });
  }, []);

  /**
   * Asks every account at once, because one slow server must not hold up the rest.
   *
   * Awaited one after another, nine mailboxes at five seconds each are fifty seconds of a window
   * that looks frozen. This is the path a press of refresh takes, and the one an account new to the
   * sidebar takes; every account is read as the application starts by the core's first round.
   *
   * `pressed` is whether a person asked for this, and it changes one answer: an account already
   * being read. A press that did nothing at all has to say so, or the button looks broken; an ask
   * this sidebar made of its own accord has nobody waiting on it, and a red line above the tree
   * over it says a new account failed when it is being read exactly as asked.
   */
  const sync = useCallback(
    async (targets: AccountNodeDto[], pressed: boolean) => {
      if (targets.length === 0) return;
      setSyncing(true);
      setSyncFailure(null);

      // Whether anything actually ran. A round already under way answers "being read right now",
      // which is not a failure -- but when that is the answer for every account, the press did
      // nothing at all, and a press that says nothing is a button that looks broken.
      let ranSomething = false;
      let alreadyReading: { accountId: string; reason: unknown } | null = null;

      try {
        for (const account of targets) asked.current.add(account.accountId);

        const answers = await Promise.allSettled(
          targets.map((account) => syncAccount(account.accountId)),
        );

        for (const [at, answer] of answers.entries()) {
          const account = targets[at];
          if (account === undefined) continue;

          if (answer.status === "fulfilled") {
            ranSomething = true;
            continue;
          }

          const reason: unknown = answer.reason;

          // Being read already is not a refusal and not a failure: what was asked for is
          // happening. It stays asked, and is let go of again the next time anything changes --
          // taking it out here asks the running pass the same question a hundred times a second
          // for as long as the pass lasts, and every turn redraws the tree and flashes the line
          // above it.
          if (isCommandError(reason) && reason.code === "alreadyReading") {
            collided.current.add(account.accountId);
            alreadyReading ??= { accountId: account.accountId, reason };
            continue;
          }

          failed(account.accountId, reason);
          ranSomething = true;
        }

        if (pressed && !ranSomething && alreadyReading !== null) {
          failed(alreadyReading.accountId, alreadyReading.reason);
        }
      } finally {
        // Whatever happened. A round that ended by throwing left the button turning for the rest
        // of the session, and the sidebar stopped asking about new accounts as well -- the same
        // flag guards both.
        setSyncing(false);
      }
      setReloads((count) => count + 1);
    },
    [failed],
  );

  // An account that has just been added has no folders until its server is asked. Only those are
  // asked automatically, and only once, so nothing here turns into a loop or a stampede.
  //
  // `answered` is what tells a new account from one whose file could not be read just now, and it
  // is the difference between asking once and asking for ever. An account being read holds its
  // own index, so a sidebar drawn during a pass gets no folders back for it — and read as "this
  // one is new", that became: ask every account, which holds every index, which empties the
  // sidebar again, which asks again. Nine mailboxes re-read every seven seconds, for as long as
  // the window was open.
  useEffect(() => {
    if (syncing) return;
    const fresh = accounts.filter(
      (account) =>
        account.folders.length === 0 &&
        account.answered &&
        !asked.current.has(account.accountId),
    );
    if (fresh.length > 0) void sync(fresh, false);
  }, [accounts, sync, syncing]);

  // A group removed while it was the thing being read. The row it was selected in is gone from
  // the sidebar, so leaving the selection on it is a list showing nothing with nothing highlighted
  // to say why — and no way back other than clicking something else.
  useEffect(() => {
    if (groups === null || selected?.kind !== "group") return;
    if (groups.some((group) => group.id === selected.groupId)) return;

    onSelect({ kind: "everything" });
  }, [groups, onSelect, selected]);

  // A mailbox that was just connected opens on its inbox. Waiting until this knows its folders,
  // because until then there is no inbox to name -- and a mailbox that shows nothing after being
  // connected is a person wondering whether it worked.
  useEffect(() => {
    if (openInboxOf === null) return;

    const added = accounts.find((one) => one.accountId === openInboxOf);
    if (added === undefined) return;

    const inbox = inboxOf(added);
    onSelect(
      inbox
        ? {
            kind: "folder",
            accountId: added.accountId,
            folderId: inbox.folderId,
            role: inbox.role,
          }
        : { kind: "account", accountId: added.accountId },
    );
    onInboxOpened?.();
  }, [accounts, onInboxOpened, onSelect, openInboxOf]);

  // Something is always open, so the list is never empty for no reason. With more than one account
  // that is everything at once; with one it is the inbox.
  useEffect(() => {
    if (selected !== null || accounts.length === 0) return;

    if (accounts.length > 1) {
      onSelect({ kind: "everything" });
      return;
    }

    const [only] = accounts;
    // By role, not by position. Dovecot lists INBOX last and every other server is free to, so
    // "the first folder" opened whatever the server happened to name first -- Archive, on the
    // mailbox this was written against.
    const inbox = inboxOf(only);
    onSelect(
      inbox
        ? {
            kind: "folder",
            accountId: only.accountId,
            folderId: inbox.folderId,
            role: inbox.role,
          }
        : { kind: "account", accountId: only.accountId },
    );
  }, [accounts, onSelect, selected]);

  const toggle = useCallback((accountId: string, collapsed: boolean) => {
    setAccounts((current) =>
      current.map((account) =>
        account.accountId === accountId ? { ...account, collapsed } : account,
      ),
    );

    setAccountCollapsed(accountId, collapsed).catch(() => {
      // The sidebar would then open tomorrow in a state nothing stored; showing what was actually
      // kept is better than showing what was asked for.
      setAccounts((current) =>
        current.map((account) =>
          account.accountId === accountId
            ? { ...account, collapsed: !collapsed }
            : account,
        ),
      );
    });
  }, []);

  // What is drawn: the counts as last read, with what a person marked since laid over them, so the
  // number beside a folder moves in the frame the dot on the row does.
  const counted = useMemo(
    () => withReadChanges(accounts, readChanges, readThrough),
    [accounts, readChanges, readThrough],
  );
  if (error) {
    // A key that could not be had comes only with storage that never opened, and the window puts
    // its recovery screen up for it, saying where the key is and what to do. Said again here, in
    // other words, it is a second account of the same thing beside the one that can be acted on.
    if (error.code === "storageKeyUnavailable") return null;
    // A newer build's profile is the recovery screen's to name, and it does. One account's file has
    // nowhere else — the profile opened, and a recovery screen up for something other than a newer
    // build says nothing about it — so it is named here.
    if (
      error.code === "storageFromANewerBuild" &&
      recovering?.kind === "command" &&
      recovering.error.code === "storageFromANewerBuild"
    ) {
      return null;
    }

    if (folded) {
      return (
        <div role="alert" className="flex flex-col items-center">
          <RailButton
            label={describeError(error, t)}
            tone="alert"
            onClick={() => onUnfold?.()}
          >
            <CircleAlert />
          </RailButton>
        </div>
      );
    }

    return (
      <div
        role="alert"
        className="flex flex-col gap-1 text-xs text-muted-foreground"
      >
        <p>{describeError(error, t)}</p>
        {error.code === "storageFromANewerBuild" && error.path && (
          <p
            dir="ltr"
            className="overflow-x-auto font-mono [text-align:match-parent]"
          >
            {error.path}
          </p>
        )}
      </div>
    );
  }

  // One account is still one account when it is the only one in a group: the fold above it would
  // do nothing and the row would be a heading over a heading.
  const single = accounts.length === 1 && (groups ?? []).length === 0;
  const arrangement = arrangedBy(groups ?? [], counted);

  /**
   * What is drawn beside this account, in the order it is drawn: the accounts in its group, or,
   * for an account in no group, everything at the top of the sidebar, where a group is one place.
   *
   * A drag and a menu entry both move a row inside its own run and never out of it: where an
   * account belongs is the group menu's decision, and one gesture may not make two.
   */
  const runHolding = (accountId: string): Run => {
    const group = (groups ?? []).find((one) =>
      one.accounts.includes(accountId),
    );
    if (group !== undefined) return { ids: group.accounts, atTheTop: false };

    return { ids: arrangement.map((entry) => entry.id), atTheTop: true };
  };

  /** Writes the new order down, and shows it at once so the row does not snap back on the way. */
  const settle = (run: Run, ordered: string[]) => {
    const places = new Map(ordered.map((id, place) => [id, place]));

    if (run.atTheTop) {
      const grouping = new Set((groups ?? []).map((group) => group.id));
      setAccounts((current) =>
        current.map((account) => ({
          ...account,
          place: places.get(account.accountId) ?? account.place,
        })),
      );
      setGroups((current) =>
        current === null
          ? current
          : current.map((group) => ({
              ...group,
              place: places.get(group.id) ?? group.place,
            })),
      );

      void arrangeSidebar(
        ordered.map(
          (id): PlacedDto =>
            grouping.has(id) ? { kind: "group", id } : { kind: "account", id },
        ),
      ).catch(() => {
        // The order that was actually written is the one to show, as below.
        setReloads((count) => count + 1);
      });
      return;
    }

    const inRun = new Set(run.ids);

    setAccounts((current) =>
      [...current].sort((one, other) => {
        if (!inRun.has(one.accountId) || !inRun.has(other.accountId)) return 0;
        return (
          (places.get(one.accountId) ?? 0) - (places.get(other.accountId) ?? 0)
        );
      }),
    );
    setGroups((current) =>
      current === null
        ? current
        : current.map((group) =>
            group.accounts.some((id) => inRun.has(id))
              ? { ...group, accounts: ordered }
              : group,
          ),
    );

    void reorderAccounts(ordered).catch(() => {
      // The order that was actually written is the one to show. Leaving the row where the drag
      // put it would be a sidebar that disagrees with itself at the next start.
      setReloads((count) => count + 1);
    });
  };

  const reordered = (accountId: string, by: -1 | 1) => {
    const run = runHolding(accountId);
    return {
      run,
      ordered: movedWithin(run.ids, accountId, run.ids.indexOf(accountId) + by),
    };
  };

  const actions: AccountActions = {
    groups: groups ?? [],
    sync: (account) => void sync([account], true),
    settings: onReconnect,
    regrouped: () => setReloads((count) => count + 1),
    move: (accountId, by) => {
      const { run, ordered } = reordered(accountId, by);
      if (ordered !== null) settle(run, ordered);
    },
    canMove: (accountId, by) => reordered(accountId, by).ordered !== null,
    drag: {
      carried,
      over,
      start: setCarried,
      hover: setOver,
      drop: (target) => {
        setOver(null);
        setCarried(null);
        if (carried === null || carried === target) return;

        // Only within one run, and the arithmetic is what says so: a row dropped on an account
        // in another group is not in this run, so there is no place in it to put the row and the
        // answer is nothing. A drag meaning two things at once — move it, and re-home it — is one
        // gesture too few for two decisions, and the second is said in the menu. A group is a
        // place in the run at the top of the sidebar, so a row from there dropped on one takes
        // its place, as it would an account's.
        const run = runHolding(carried);
        const ordered = movedWithin(run.ids, carried, run.ids.indexOf(target));
        if (ordered !== null) settle(run, ordered);
      },
      lands: (target) =>
        carried !== null &&
        carried !== target &&
        runHolding(carried).ids.includes(target),
      end: () => {
        setCarried(null);
        setOver(null);
      },
    },
  };

  /**
   * Every account asked at once, for the button in the heading and the one in the folded sidebar:
   * one press, one piece of work, whichever of them it was.
   */
  const getMail = () => {
    if (syncing) return;
    void sync(accounts, true);
  };

  return (
    <div
      className={`flex min-h-0 flex-col gap-1 ${folded ? "w-full items-center" : ""}`}
    >
      {folded ? (
        // Under Compose, where the heading stood: the heading is words, and the plus beside it
        // adds an account, which is something done with the sidebar open.
        accounts.length > 0 && (
          <>
            <RailButton
              label={syncing ? t("shell.refreshing") : t("shell.refresh")}
              busy={syncing}
              onClick={getMail}
            >
              <RefreshCw className={syncing ? "animate-spin" : undefined} />
            </RailButton>
            {/* What the heading did: the two buttons above are things to do, and the icons below
                are places to read. */}
            <RailRule />
          </>
        )
      ) : (
        <ColumnHeading title={t("shell.mailboxes")}>
          <div className="flex items-center gap-0.5">
            {/* Nothing to refresh, so nothing to press. Hidden rather than left unavailable: a
                control that cannot do anything is one a person tries anyway, and being unavailable
                for a good reason still costs them the press. */}
            {accounts.length > 0 && (
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label={syncing ? t("shell.refreshing") : t("shell.refresh")}
                title={t("shell.refresh")}
                // Not disabled while it works. Disabling the button somebody has just pressed blurs
                // it in every engine this ships on, so the focus fell to the top of the window and
                // the wait itself was announced by nothing. The second press is refused instead.
                aria-busy={syncing}
                disabled={accounts.length === 0}
                onClick={getMail}
              >
                <RefreshCw className={syncing ? "animate-spin" : undefined} />
              </Button>
            )}
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label={t("workspace.connectAction")}
              title={t("workspace.connectAction")}
              onClick={onAddAccount}
            >
              <Plus />
            </Button>
          </div>
        </ColumnHeading>
      )}

      {/* The wait and its end, for somebody who cannot see the turning circle. */}
      <span role="status" className="sr-only">
        {syncing ? t("shell.refreshing") : ""}
      </span>

      {syncFailure &&
        (folded ? (
          <div role="alert">
            <RailButton
              label={failureSentence(syncFailure, accounts, t)}
              // The sentences about a password name the account themselves.
              hints={
                asksForThePasswordAgain(syncFailure.error)
                  ? []
                  : [emailOf(syncFailure.accountId, accounts)]
              }
              tone="alert"
              onClick={() => onUnfold?.()}
            >
              <CircleAlert />
            </RailButton>
          </div>
        ) : (
          <SyncFailure
            failure={syncFailure}
            accounts={accounts}
            onReconnect={onReconnect}
            t={t}
          />
        ))}

      {/* The one part of the column that scrolls, and it scrolls because a handful of mailboxes
          with their folders is taller than the window. It stays a scroller -- the wheel, the
          trackpad and the focus moving from row to row all carry it -- and only the bar is not
          drawn: a bar beside the rows is a gutter the rows and the grey behind the chosen one
          stop short of. Never sideways: a scroller that is one way only in the stylesheet is
          both ways in the engine, and a badge a pixel too wide for a narrow column would slide
          the whole tree under the pointer.

          Folded, a column of icons the width of the rail, with room around each for the ring the
          focus draws: the scroller cuts off whatever reaches past its edges. */}
      <nav
        aria-label={t("shell.mailboxes")}
        className={`no-gutter flex flex-col gap-0.5 overflow-x-hidden overflow-y-auto text-sm ${
          folded ? "w-full items-center py-1" : ""
        }`}
      >
        {folded ? (
          <Rail
            accounts={counted}
            selected={selected}
            onSelect={onSelect}
            onUnfold={onUnfold}
            t={t}
          />
        ) : (
          <>
            {accounts.length > 1 && (
              <Row
                label={t("shell.allInboxes")}
                badge={unreadBadge(
                  counted.reduce((sum, account) => sum + account.unread, 0),
                )}
                selected={selected?.kind === "everything"}
                onSelect={() => onSelect({ kind: "everything" })}
                t={t}
              />
            )}

            {arrangement.map((entry) =>
              entry.kind === "group" ? (
                <GroupBranch
                  key={entry.id}
                  group={entry}
                  accounts={entry.accounts}
                  selected={selected}
                  onSelect={onSelect}
                  onFold={foldGroup}
                  onToggleAccount={toggle}
                  actions={actions}
                  t={t}
                />
              ) : (
                <AccountBranch
                  key={entry.id}
                  account={entry.account}
                  single={single}
                  actions={actions}
                  selected={selected}
                  onSelect={onSelect}
                  onToggle={toggle}
                  t={t}
                />
              ),
            )}
          </>
        )}
      </nav>
    </div>
  );
}

/// What went wrong, said about an account by name.
///
/// A password that is no longer in the store is not a failure to report and forget: it is a
/// thing to do, and nothing else will do it. Everything else is a sentence.
function SyncFailure({
  failure,
  accounts,
  onReconnect,
  t,
}: {
  failure: { accountId: string; error: CommandError };
  accounts: AccountNodeDto[];
  onReconnect: (accountId: string) => void;
  t: Translator;
}) {
  const email = emailOf(failure.accountId, accounts);

  if (asksForThePasswordAgain(failure.error)) {
    return (
      <div role="alert" className="flex flex-col items-start gap-1 px-2">
        <span className="text-xs text-muted-foreground">
          {failureSentence(failure, accounts, t)}
        </span>
        <Button
          type="button"
          variant="outline"
          size="xs"
          onClick={() => onReconnect(failure.accountId)}
        >
          {t("account.reconnect")}
        </Button>
      </div>
    );
  }

  return (
    <p role="alert" className="px-2 text-xs text-destructive">
      {email}: {failureSentence(failure, accounts, t)}
    </p>
  );
}

/** The address of the account a failure is about, or what it is known by where it is gone. */
function emailOf(accountId: string, accounts: AccountNodeDto[]): string {
  return (
    accounts.find((one) => one.accountId === accountId)?.email ?? accountId
  );
}

/**
 * The two failures a person can do something about, and they are one thing to do. A password
 * that is gone and a password the server refuses both end at the same form with the same account
 * filled in; telling them apart in the sentence and not in the button would be describing the
 * difference to somebody who has the same next move either way.
 */
function asksForThePasswordAgain(error: CommandError): boolean {
  return error.code === "credentialsMissing" || error.code === "unauthorized";
}

/** What a failure is said as, the same in the open sidebar and in the hint of the folded one. */
function failureSentence(
  failure: { accountId: string; error: CommandError },
  accounts: AccountNodeDto[],
  t: Translator,
): string {
  const email = emailOf(failure.accountId, accounts);

  if (failure.error.code === "credentialsMissing")
    return t("account.noPassword", { email });
  if (failure.error.code === "unauthorized")
    return t("account.passwordRefused", { email });

  return describeError(failure.error, t);
}

function AccountBranch({
  account,
  single,
  grouped = false,
  actions,
  selected,
  onSelect,
  onToggle,
  t,
}: {
  account: AccountNodeDto;
  single: boolean;
  /** In a group, so its own row sits one level in. Its folders do not: see `INDENT`. */
  grouped?: boolean;
  actions: AccountActions;
  selected: Mailbox | null;
  onSelect: (mailbox: Mailbox) => void;
  onToggle: (accountId: string, collapsed: boolean) => void;
  t: Translator;
}) {
  const open = single || !account.collapsed;

  return (
    <>
      {!single && (
        <ContextMenu>
          {/* The whole row is what is picked up, and what a row is dropped on. Not the button
              inside it: a drag started on a control is a drag a person did not mean, and every
              press would have to be told apart from a hold. */}
          <ContextMenuTrigger
            draggable
            // Says this is the window moving something of its own, not the browser carrying it
            // out. Without it the drag is cancelled at the document, one level above every
            // handler below, and the row cannot be picked up at all.
            data-drags=""
            aria-grabbed={actions.drag.carried === account.accountId}
            onDragStart={(event) => {
              // Something has to be carried or the drop never fires. Under a type only this
              // window knows, so a row dropped on another application carries nothing it can
              // read: as plain text it was an identifier out of somebody's mailbox, handed to
              // whatever was under the pointer.
              event.dataTransfer.effectAllowed = "move";
              event.dataTransfer.setData(
                CARRYING_AN_ACCOUNT,
                account.accountId,
              );
              actions.drag.start(account.accountId);
            }}
            onDragOver={(event) => {
              if (actions.drag.carried === null) return;
              // Refusing the default is what makes a drop possible at all.
              event.preventDefault();
              event.dataTransfer.dropEffect = "move";
              actions.drag.hover(account.accountId);
            }}
            onDrop={(event) => {
              event.preventDefault();
              actions.drag.drop(account.accountId);
            }}
            onDragEnd={actions.drag.end}
            // A line where the row would land, and nothing else moves until it does. Rows sliding
            // out of the way under the pointer is a list that answers before the person has.
            className={
              actions.drag.over === account.accountId &&
              actions.drag.carried !== null &&
              actions.drag.carried !== account.accountId
                ? "rounded-md ring-1 ring-primary/70"
                : undefined
            }
          >
            <Folding
              depth={grouped ? 1 : 0}
              label={t(
                account.collapsed ? "shell.expandNamed" : "shell.collapseNamed",
                { account: account.displayName.trim() || account.email },
              )}
              expanded={!account.collapsed}
              onFold={() => onToggle(account.accountId, !account.collapsed)}
            >
              <Row
                depth={grouped ? 1 : 0}
                label={account.displayName.trim() || account.email}
                title={account.email}
                badge={unreadBadge(account.unread)}
                selected={
                  selected !== null &&
                  isSameMailbox(selected, {
                    kind: "account",
                    accountId: account.accountId,
                  })
                }
                onSelect={() =>
                  onSelect({ kind: "account", accountId: account.accountId })
                }
                t={t}
              />
            </Folding>
          </ContextMenuTrigger>
          <AccountMenu account={account} actions={actions} t={t} />
        </ContextMenu>
      )}

      {/* An account whose folders have not arrived is an account being read, not an account with
          nothing in it. Left blank it reads as broken — which is what it looked like for the
          minute a first pass over a hosted server takes.

          And an account whose mail could not be read at all is neither: saying "reading folders"
          about it would be a message that never comes true. */}
      {open && account.folders.length === 0 && (
        <p
          role={account.answered ? undefined : "alert"}
          className={`py-1.5 pe-2 text-xs ${INDENT[single ? 0 : FOLDER]} ${
            account.answered ? "text-muted-foreground" : "text-destructive"
          }`}
        >
          {t(account.answered ? "shell.readingFolders" : "shell.mailNotRead")}
        </p>
      )}

      {open &&
        account.folders.map((folder) => (
          <Row
            key={folder.folderId}
            label={folderName(folder, t)}
            badge={folderBadge(folder)}
            depth={single ? 0 : FOLDER}
            selected={
              selected !== null &&
              isSameMailbox(selected, {
                kind: "folder",
                accountId: account.accountId,
                folderId: folder.folderId,
                role: folder.role,
              })
            }
            onSelect={() =>
              onSelect({
                kind: "folder",
                accountId: account.accountId,
                folderId: folder.folderId,
                role: folder.role,
              })
            }
            t={t}
          />
        ))}
    </>
  );
}

/** One thing at the top of the sidebar: a group with the accounts in it, or an account in none. */
type AtTheTop =
  | {
      kind: "group";
      id: string;
      name: string;
      collapsed: boolean;
      accounts: AccountNodeDto[];
    }
  | { kind: "account"; id: string; account: AccountNodeDto };

/** The rows a drag or a menu entry moves one among, and whether that is the top of the sidebar. */
type Run = { ids: string[]; atTheTop: boolean };

/**
 * Everything at the top of the sidebar, in the order it is drawn: the groups and the accounts in
 * no group, in one order, by the place each was given.
 *
 * Arithmetic over values rather than a hook: nothing here is state, and the sidebar is not the
 * only thing that will ever need to know which mailbox sits where.
 *
 * A group naming an account this device no longer holds simply draws without it. Removing an
 * account does not walk the arrangements it was in, so this happens whenever somebody removes a
 * grouped mailbox — and an entry that could not be drawn is better than a row for a mailbox that
 * is not there.
 */
function arrangedBy(
  groups: AccountGroupDto[],
  accounts: AccountNodeDto[],
): AtTheTop[] {
  const byId = new Map(accounts.map((account) => [account.accountId, account]));
  const grouped = new Set<string>();

  const drawn = groups.map((group) => {
    const held = group.accounts
      .map((accountId) => byId.get(accountId))
      .filter((account): account is AccountNodeDto => account !== undefined);

    for (const account of held) grouped.add(account.accountId);

    const entry: AtTheTop = {
      kind: "group",
      id: group.id,
      name: group.name,
      collapsed: group.collapsed,
      accounts: held,
    };
    return { place: group.place, entry };
  });

  const loose = accounts
    .filter((account) => !grouped.has(account.accountId))
    .map((account) => {
      const entry: AtTheTop = {
        kind: "account",
        id: account.accountId,
        account,
      };
      return { place: account.place, entry };
    });

  // Where two places agree, a group first and otherwise the order they came in, which `sort`
  // keeps: what every sidebar showed before the two were counted together.
  return [...drawn, ...loose]
    .sort((one, other) => one.place - other.place)
    .map(({ entry }) => entry);
}

/**
 * One group: a row that shows the mail of everything in it, and the mailboxes under it.
 *
 * The row is selectable as well as foldable, because the narrowing is the point — a person with
 * fifty mailboxes reads "Work" far more often than any one mailbox in it.
 */
function GroupBranch({
  group,
  accounts,
  selected,
  onSelect,
  onFold,
  onToggleAccount,
  actions,
  t,
}: {
  group: { id: string; name: string; collapsed: boolean };
  accounts: AccountNodeDto[];
  selected: Mailbox | null;
  onSelect: (mailbox: Mailbox) => void;
  onFold: (groupId: string, collapsed: boolean) => void;
  onToggleAccount: (accountId: string, collapsed: boolean) => void;
  actions: AccountActions;
  t: Translator;
}) {
  return (
    <>
      {/* Where a row from the top of the sidebar is dropped to take the group's place, as it would
          take an account's. Only that: a drop never puts an account into a group, which is the
          menu's decision. A row the group cannot take is not offered the drop at all. */}
      <div
        onDragOver={(event) => {
          if (!actions.drag.lands(group.id)) return;
          event.preventDefault();
          event.dataTransfer.dropEffect = "move";
          actions.drag.hover(group.id);
        }}
        onDrop={(event) => {
          event.preventDefault();
          actions.drag.drop(group.id);
        }}
        className={
          actions.drag.over === group.id && actions.drag.lands(group.id)
            ? "rounded-md ring-1 ring-primary/70"
            : undefined
        }
      >
        <Folding
          depth={0}
          // Not the account's label: what a group hides is mailboxes, and "Hide folders" over a
          // list of mailboxes names the wrong thing.
          label={
            group.collapsed ? t("shell.showMailboxes") : t("shell.hideMailboxes")
          }
          expanded={!group.collapsed}
          onFold={() => onFold(group.id, !group.collapsed)}
        >
          <Row
            label={group.name}
            badge={unreadBadge(
              accounts.reduce((sum, account) => sum + account.unread, 0),
            )}
            selected={
              selected !== null &&
              isSameMailbox(selected, { kind: "group", groupId: group.id })
            }
            onSelect={() => onSelect({ kind: "group", groupId: group.id })}
            t={t}
          />
        </Folding>
      </div>

      {/* A group somebody made and has not filled yet. Said rather than drawn as a blank, which
          reads as a group whose mailboxes failed to load. */}
      {!group.collapsed && accounts.length === 0 && (
        <p
          className={`py-1.5 pe-2 text-xs text-muted-foreground ${INDENT[FOLDER]}`}
        >
          {t("shell.groupIsEmpty")}
        </p>
      )}

      {/* No wrapper, and no padding on one: an account in a group is drawn one level in by the
          row itself. A container that indented them took the width off the surface a person
          points at as well, so the thing that lit up under the pointer stopped short of the edge
          and was a different width from every row outside a group. */}
      {!group.collapsed &&
        accounts.map((account) => (
          <AccountBranch
            key={account.accountId}
            account={account}
            single={false}
            grouped
            actions={actions}
            selected={selected}
            onSelect={onSelect}
            onToggle={onToggleAccount}
            t={t}
          />
        ))}
    </>
  );
}
