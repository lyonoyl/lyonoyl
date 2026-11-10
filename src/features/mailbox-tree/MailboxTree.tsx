import { CircleAlert, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  unreadBadge,
  withReadChanges,
  type ReadChange,
} from "@/entities/folder";
import type { Mailbox } from "@/entities/mailbox";
import { describeError, useTranslation, type Translator } from "@/shared/i18n";
import {
  arrangeSidebar,
  getMailboxTree,
  isCommandError,
  listAccountGroups,
  placeAccountInGroup,
  reorderAccounts,
  setAccountCollapsed,
  setAccountGroupCollapsed,
  syncAccount,
} from "@/shared/ipc/client";
import { ColumnHeading } from "@/shared/ui/ColumnHeading";
import { AccountBranch } from "./AccountBranch";
import { movedWithin, type AccountActions } from "./AccountMenu";
import { AddMenu } from "./AddMenu";
import {
  afterDrop,
  END,
  landingMark,
  landingOn,
  type Carried,
  type Landing,
  type Move,
  type Placing,
} from "./dropping";
import { GroupBranch } from "./GroupBranch";
import { Rail, RailRule, Row } from "./Row";
import { Button } from "@/shared/ui/primitives/button";
import { RailButton } from "@/shared/ui/RailButton";
import type { AccountGroupDto } from "@/shared/ipc/generated/AccountGroupDto";
import type { AccountNodeDto } from "@/shared/ipc/generated/AccountNodeDto";
import type { FolderDto } from "@/shared/ipc/generated/FolderDto";
import type { CommandError } from "@/shared/ipc/generated/CommandError";
import type { PlacedDto } from "@/shared/ipc/generated/PlacedDto";
import type { SortingDto } from "@/shared/ipc/generated/SortingDto";

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

/** No changes, as one value, so a tree given none does not lay them over its counts every draw. */
const NO_CHANGES: ReadChange[] = [];

export function MailboxTree({
  selected,
  onSelect,
  onAddAccount,
  onArrangementChanged,
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
  /** Called when the groups change here, so the settings beside this read them again. */
  onArrangementChanged?: () => void;
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
  /**
   * What a drag is carrying, and where a drop would land. The handlers decide from the refs, which
   * hold the answer the moment it changes; the state only draws it.
   */
  const [carried, setCarried] = useState<Carried | null>(null);
  const [over, setOver] = useState<Landing | null>(null);
  const carrying = useRef<Carried | null>(null);
  const landing = useRef<Landing | null>(null);
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
   * An account already being read is not a failure, whoever asked for it: what was asked for is
   * under way, and the line at the foot of the window shows it.
   */
  const sync = useCallback(
    async (targets: AccountNodeDto[]) => {
      if (targets.length === 0) return;
      setSyncing(true);
      setSyncFailure(null);

      try {
        for (const account of targets) asked.current.add(account.accountId);

        const answers = await Promise.allSettled(
          targets.map((account) => syncAccount(account.accountId)),
        );

        for (const [at, answer] of answers.entries()) {
          const account = targets[at];
          if (account === undefined) continue;

          if (answer.status === "fulfilled") continue;

          const reason: unknown = answer.reason;

          // Being read already is not a refusal and not a failure: what was asked for is
          // happening. It stays asked, and is let go of again the next time anything changes --
          // taking it out here asks the running pass the same question a hundred times a second
          // for as long as the pass lasts, and every turn redraws the tree and flashes the line
          // above it.
          if (isCommandError(reason) && reason.code === "alreadyReading") {
            collided.current.add(account.accountId);
            continue;
          }

          failed(account.accountId, reason);
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
    if (fresh.length > 0) void sync(fresh);
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
   * What is drawn beside this row, in the order it is drawn: the accounts in its group, or, for an
   * account in no group and for a group, everything at the top of the sidebar.
   *
   * Move up and Move down move a row inside its own run and never out of it. A drag can carry a
   * mailbox into another run, and `afterDrop` says where.
   */
  const runHolding = (accountId: string): Run => {
    const group = (groups ?? []).find((one) =>
      one.accounts.includes(accountId),
    );
    if (group !== undefined) return { ids: group.accounts, groupId: group.id };

    return { ids: arrangement.map((entry) => entry.id), groupId: null };
  };

  /** The arrangement as a drop reads it: what is at the top, and the mailboxes in each group. */
  const placing: Placing[] = arrangement.map((entry) =>
    entry.kind === "group"
      ? {
          kind: "group",
          id: entry.id,
          accounts: entry.accounts.map((account) => account.accountId),
        }
      : { kind: "account", id: entry.id },
  );

  const regrouped = () => {
    setReloads((count) => count + 1);
    onArrangementChanged?.();
  };

  /**
   * Shows a move at once, so a row does not snap back on the way, and writes it down: where the
   * mailbox now belongs first, then the order it landed in, which names it there.
   */
  const land = ({ home, run }: Move) => {
    setGroups((current) =>
      current === null
        ? current
        : current.map((group) => {
            // An order made by hand is the group's order now, as the core keeps it.
            if (run !== undefined && run.groupId === group.id) {
              return { ...group, accounts: run.ids, sorting: "manual" };
            }
            if (home === undefined) return group;

            const left = group.accounts.filter((id) => id !== home.accountId);
            if (home.groupId === group.id) {
              return { ...group, accounts: [...left, home.accountId] };
            }
            return left.length === group.accounts.length
              ? group
              : { ...group, accounts: left };
          }),
    );

    if (run !== undefined && run.groupId === null) {
      const places = new Map(run.ids.map((id, place) => [id, place]));
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
    }

    const grouping = new Set((groups ?? []).map((group) => group.id));
    const written = async () => {
      if (home !== undefined) {
        await placeAccountInGroup(home.accountId, home.groupId);
      }
      if (run === undefined) return;
      await (run.groupId === null
        ? arrangeSidebar(
            run.ids.map(
              (id): PlacedDto =>
                grouping.has(id)
                  ? { kind: "group", id }
                  : { kind: "account", id },
            ),
          )
        : reorderAccounts(run.ids));
    };

    void written().then(
      () => {
        // A group sorted by name or by date puts a mailbox that joins it in its own place, which
        // the core says; and the settings beside this list who is in which group.
        if (home !== undefined) regrouped();
      },
      () => {
        // The order that was actually written is the one to show. Leaving the row where the drag
        // put it would be a sidebar that disagrees with itself at the next start.
        setReloads((count) => count + 1);
      },
    );
  };

  const reordered = (accountId: string, by: -1 | 1) => {
    const run = runHolding(accountId);
    return {
      run,
      ordered: movedWithin(run.ids, accountId, run.ids.indexOf(accountId) + by),
    };
  };

  const endDrag = () => {
    carrying.current = null;
    landing.current = null;
    setCarried(null);
    setOver(null);
  };

  /** Shows where a drop would land, redrawing only when that changed: a drag reports many times a second. */
  const showLanding = (next: Landing | null) => {
    const now = landing.current;
    if (now?.id === next?.id && now?.where === next?.where) return;
    landing.current = next;
    setOver(next);
  };

  const actions: AccountActions = {
    groups: groups ?? [],
    sync: (account) => void sync([account]),
    settings: onReconnect,
    regrouped,
    move: (accountId, by) => {
      const { run, ordered } = reordered(accountId, by);
      if (ordered !== null) land({ run: { ids: ordered, groupId: run.groupId } });
    },
    canMove: (accountId, by) => reordered(accountId, by).ordered !== null,
    drag: {
      carried,
      over,
      start: (what) => {
        carrying.current = what;
        setCarried(what);
      },
      // A row takes a drop only where it would change something, which `afterDrop` is the one
      // answer to: a line drawn where nothing would move is a promise the drop does not keep.
      hover: (target, fraction) => {
        const what = carrying.current;
        const at = what === null ? null : landingOn(target, what, fraction);
        const takes =
          what !== null && at !== null && afterDrop(placing, what, at) !== null;
        showLanding(takes ? at : null);
        return takes;
      },
      away: () => showLanding(null),
      drop: () => {
        const what = carrying.current;
        const at = landing.current;
        endDrag();
        if (what === null || at === null) return;

        const move = afterDrop(placing, what, at);
        if (move !== null) land(move);
      },
      end: endDrag,
    },
  };

  /**
   * Every account asked at once, for the button in the heading and the one in the folded sidebar:
   * one press, one piece of work, whichever of them it was.
   */
  const getMail = () => {
    if (syncing) return;
    void sync(accounts);
  };

  return (
    <div
      className={`flex min-h-0 flex-col gap-1 ${folded ? "w-full items-center" : ""}`}
    >
      {folded ? (
        // Under Compose, where the heading stood: the heading is words, and the plus beside it
        // adds a mailbox or a group, which is something done with the sidebar open.
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
            <AddMenu onAddAccount={onAddAccount} onGroupAdded={actions.regrouped} />
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
        // Rows that take a drop keep their drag to themselves, so one that reaches here is over a
        // folder or the space between rows, where nothing lands.
        onDragOver={actions.drag.away}
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

            {/* Below everything at the top, while something is carried: a drop here puts it
                last. No row above can offer that place when the last thing is an open group,
                whose row has its mailboxes below it. Drawn under the rows, so none of them
                moves when it appears. */}
            {carried !== null && (
              <div
                aria-hidden="true"
                data-end=""
                className={`min-h-6 flex-1 ${landingMark(over?.id === END ? "before" : null)}`}
                onDragOver={(event) => {
                  event.stopPropagation();
                  if (!actions.drag.hover({ kind: "end" }, null)) return;
                  event.preventDefault();
                  event.dataTransfer.dropEffect = "move";
                }}
                onDrop={(event) => {
                  event.preventDefault();
                  event.stopPropagation();
                  actions.drag.drop();
                }}
              />
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

/** One thing at the top of the sidebar: a group with the accounts in it, or an account in none. */
type AtTheTop =
  | {
      kind: "group";
      id: string;
      name: string;
      collapsed: boolean;
      sorting: SortingDto;
      accounts: AccountNodeDto[];
    }
  | { kind: "account"; id: string; account: AccountNodeDto };

/** The rows a menu entry moves one among: a group's, or the top of the sidebar's for `null`. */
type Run = { ids: string[]; groupId: string | null };

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
      sorting: group.sorting,
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
