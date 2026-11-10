import type { DragEvent } from "react";

/**
 * What a dragged row carries, under types no other application knows.
 *
 * The drop decides from what the window is already holding, so nothing reads them back — they are
 * here because a drag with an empty payload never produces a drop at all.
 */
export const CARRYING_AN_ACCOUNT = "application/x-lyonoyl-account";
export const CARRYING_A_GROUP = "application/x-lyonoyl-group";

/** What a drag carries: a mailbox, or a group with everything in it. */
export type Carried = { kind: "account" | "group"; id: string };

/** Where a drop lands: above a row, below it, or into a group. */
export type Landing = { id: string; where: "before" | "after" | "into" };

/** A row a drop can land on, as a drag reads it, or the space below everything at the top. */
export type DropTarget =
  | {
      kind: "group";
      id: string;
      /** Open with its mailboxes drawn under it, so that below its row is already inside it. */
      open: boolean;
    }
  | { kind: "account"; id: string; groupId: string | null }
  | { kind: "end" };

/**
 * Where the space below everything at the top is named in a landing. Not an identifier anything
 * else can carry, so nothing is mistaken for it.
 */
export const END = "\u0000end";

/** One thing at the top of the sidebar, as a drop reads it. */
export type Placing =
  | { kind: "group"; id: string; accounts: string[] }
  | { kind: "account"; id: string };

/**
 * What a drop does.
 *
 * `home` is where the mailbox carried now belongs — a group, or the top of the sidebar for `null` —
 * and is absent where it stays where it was. `run` is the order of the rows it lands among, in a
 * group or at the top for `null`, and is absent for a drop into a group, where the group's own
 * order says where it stands.
 */
export type Move = {
  home?: { accountId: string; groupId: string | null };
  run?: { ids: string[]; groupId: string | null };
};

/**
 * How much of a group's row, above and below, puts a mailbox beside the group rather than in it.
 *
 * The middle of the row is the group's, because that is where a person aims at it.
 */
const EDGE = 0.3;

/**
 * Where on a row the pointer is, from 0 at its top to 1 at its bottom.
 *
 * `null` where the row has no height to measure or the event no position, which is a drag the
 * window did not see the pointer of: it then lands where the row stands.
 */
export function fractionOf(event: DragEvent<HTMLElement>): number | null {
  const box = event.currentTarget.getBoundingClientRect();
  if (box.height <= 0 || !Number.isFinite(event.clientY)) return null;
  return Math.min(1, Math.max(0, (event.clientY - box.top) / box.height));
}

/**
 * Where a drop on this row would land, by where the pointer is on it, or `null` where the row
 * takes nothing.
 *
 * A mailbox over a group's row goes into the group, and only the edges put it above the group or,
 * where the group is folded, below it; below an open group's row is the group's first mailbox.
 * Anywhere else it goes above or below the row the pointer is on, in that row's group or at the
 * top of the sidebar. A group goes only among the things at the top: a group in a group is not
 * something the sidebar draws.
 */
export function landingOn(
  target: DropTarget,
  carried: Carried,
  fraction: number | null,
): Landing | null {
  // Below everything at the top: last there, which no row can offer when the last thing is an open
  // group, whose row has its mailboxes below it.
  if (target.kind === "end") return { id: END, where: "after" };
  if (target.id === carried.id) return null;

  if (carried.kind === "group") {
    if (target.kind === "account" && target.groupId !== null) return null;
    // Below an open group's row is its first mailbox, so only above it is a place for a group.
    if (target.kind === "group" && target.open) {
      return { id: target.id, where: "before" };
    }
    return { id: target.id, where: lowerHalf(fraction) ? "after" : "before" };
  }

  if (target.kind === "group") {
    if (fraction !== null && fraction < EDGE) {
      return { id: target.id, where: "before" };
    }
    if (fraction !== null && !target.open && fraction > 1 - EDGE) {
      return { id: target.id, where: "after" };
    }
    return { id: target.id, where: "into" };
  }

  return { id: target.id, where: lowerHalf(fraction) ? "after" : "before" };
}

function lowerHalf(fraction: number | null): boolean {
  return fraction !== null && fraction >= 0.5;
}

/**
 * What dropping `carried` at `landing` changes, or `null` where it changes nothing.
 *
 * Plain arithmetic over the arrangement, so that what a drag shows it will do and what the drop
 * then does are one answer.
 */
export function afterDrop(
  top: Placing[],
  carried: Carried,
  landing: Landing,
): Move | null {
  if (carried.id === landing.id) return null;

  const groups = top.filter(
    (one): one is Extract<Placing, { kind: "group" }> => one.kind === "group",
  );
  const atTheTop = top.map((one) => one.id);
  const groupHolding = (accountId: string) =>
    groups.find((group) => group.accounts.includes(accountId))?.id ?? null;

  const from = carried.kind === "account" ? groupHolding(carried.id) : null;

  if (landing.id === END) {
    const last = [...atTheTop.filter((id) => id !== carried.id), carried.id];
    const same = last.every((id, place) => id === atTheTop[place]);
    if (same && last.length === atTheTop.length) return null;

    const run = { ids: last, groupId: null };
    return from === null
      ? { run }
      : { run, home: { accountId: carried.id, groupId: null } };
  }

  if (carried.kind === "group") {
    if (landing.where === "into") return null;
    return reordered(atTheTop, carried.id, landing, null);
  }

  // Where in the group it stands is the group's own order: last, or by its name or its date.
  if (landing.where === "into") {
    return from === landing.id
      ? null
      : { home: { accountId: carried.id, groupId: landing.id } };
  }

  const isGroupRow = groups.some((group) => group.id === landing.id);
  const into = isGroupRow ? null : groupHolding(landing.id);
  const ids =
    into === null
      ? atTheTop
      : (groups.find((group) => group.id === into)?.accounts ?? []);

  const moved = reordered(ids, carried.id, landing, into);
  if (moved === null || from === into) return moved;
  return { ...moved, home: { accountId: carried.id, groupId: into } };
}

/** The run with `carried` taken out and put back beside `landing`, or `null` where nothing moved. */
function reordered(
  ids: string[],
  carried: string,
  landing: Landing,
  groupId: string | null,
): Move | null {
  const without = ids.filter((id) => id !== carried);
  const at = without.indexOf(landing.id);
  if (at < 0) return null;

  const ordered = [...without];
  ordered.splice(landing.where === "after" ? at + 1 : at, 0, carried);

  const same =
    ordered.length === ids.length &&
    ordered.every((id, place) => id === ids[place]);
  return same ? null : { run: { ids: ordered, groupId } };
}

/**
 * What a row wears while a drop over it would land there: a line above it or below it, or, for a
 * drop into a group, the row lit whole.
 *
 * Drawn inside the row, over what it holds. A ring drawn outside it was cut off at the sides by the
 * sidebar, which clips whatever reaches past its edges.
 */
export function landingMark(where: Landing["where"] | null): string {
  switch (where) {
    case "before":
      return "relative before:pointer-events-none before:absolute before:inset-x-1 before:top-0 before:h-0.5 before:rounded-full before:bg-primary";
    case "after":
      return "relative after:pointer-events-none after:absolute after:inset-x-1 after:bottom-0 after:h-0.5 after:rounded-full after:bg-primary";
    case "into":
      return "relative after:pointer-events-none after:absolute after:inset-0 after:rounded-md after:bg-primary/10 after:ring-1 after:ring-primary/70 after:ring-inset";
    default:
      return "";
  }
}
