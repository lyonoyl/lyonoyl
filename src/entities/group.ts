import type { Mailbox } from "@/entities/mailbox";

/**
 * Groups in the order of their names, as the person's language sorts them.
 *
 * Wherever a mailbox's group is chosen — the settings, a mailbox's menu, the panel that adds one —
 * the groups are a list to find a name in, and a name is found by its letters. The sidebar keeps
 * the order a person arranged instead, which answers a different question.
 *
 * Case and accents aside, and numbers read as numbers, so "Project 2" stands before "Project 10".
 */
export function inOrderOfNames<T extends { name: string }>(
  groups: readonly T[],
  language: string,
): T[] {
  const names = new Intl.Collator(language, {
    sensitivity: "base",
    numeric: true,
  });
  return [...groups].sort((one, other) => names.compare(one.name, other.name));
}

/**
 * The group a selection in the sidebar stands in: the group itself, or the group holding the
 * mailbox or folder chosen. `null` for everything at once, a mailbox in no group, or a group that
 * is gone.
 */
export function groupOf(
  selection: Mailbox | null,
  groups: readonly { id: string; accounts: readonly string[] }[],
): string | null {
  if (selection === null || selection.kind === "everything") return null;
  if (selection.kind === "group") {
    return groups.some((group) => group.id === selection.groupId)
      ? selection.groupId
      : null;
  }
  return (
    groups.find((group) => group.accounts.includes(selection.accountId))
      ?.id ?? null
  );
}
