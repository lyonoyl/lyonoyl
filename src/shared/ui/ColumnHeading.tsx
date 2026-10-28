import type { ReactNode } from "react";

/**
 * The line at the top of the side column, above whatever the column is listing.
 *
 * One piece rather than two, because the column lists two different things — the mailboxes, and
 * the parts of the settings while those are in front — and the heading over each has to be the
 * same heading: the same type, on a line as tall as a button whether or not buttons stand in it.
 * Two copies drift apart, and the one without buttons in its row ends up shorter than the other.
 *
 * Where the line stands is the column's business, not this one's. The settings take Compose out of
 * the column, so their list and its heading start higher up, in the room Compose leaves.
 */
export function ColumnHeading({
  title,
  children,
}: {
  title: string;
  /** Controls that belong to what is listed below, sitting at the far end of the same line. */
  children?: ReactNode;
}) {
  return (
    // The height is a button's, so the line is the same whether anything is standing in it.
    <div className="flex h-7 items-center justify-between ps-2">
      <span className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
        {title}
      </span>
      {children}
    </div>
  );
}
