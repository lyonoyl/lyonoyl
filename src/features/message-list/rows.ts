/**
 * How large a row is, where it sits, the name it is found by, and how it writes its date.
 *
 * In a file of its own because the row draws itself by these and the list places and announces
 * rows by the same ones: two copies would be a list whose scrollbar and whose rows disagree, or a
 * list naming a row by an identifier the row does not carry. The folder for sent mail draws rows
 * from this device above the list, and they write their date as the rows below them do.
 */
import type { DensityDto } from "@/shared/ipc/generated/DensityDto";

/**
 * Fixed per density, so a position maps to a pixel offset without measuring anything.
 *
 * Comfortable gives the preview three lines of its own; compact is one line per message, for
 * reading a long list at a glance.
 */
/// Tall enough for what a row holds: sender, subject, three lines of text, and the same breathing
/// room below them as above. Content that did not fit would be clipped against the bottom edge,
/// which reads as a layout mistake rather than as a message that goes on.
///
/// Two pixels of that are the gap above and below the selection, so the row that holds the content
/// is the same size it always was and nothing moved closer together to pay for the highlight.
export const ROW_HEIGHT: Record<DensityDto, number> = {
  comfortable: 101,
  compact: 40,
};

/// The gap above the first row and below the last, in pixels.
///
/// A selected first row sitting flush against the top edge reads as cut off rather than as chosen.
/// The virtualizer is told about it rather than the scroller being padded, so every offset it
/// computes already includes it and scrolling to a row still lands on the row.
export const LIST_GUTTER = 6;

export const LIST_ID = "message-list";

/** How a row writes its date, in the language the window is in. */
export function rowDates(language: string): Intl.DateTimeFormat {
  return new Intl.DateTimeFormat(language, { dateStyle: "medium" });
}

export function rowId(index: number): string {
  return `${LIST_ID}-row-${index}`;
}

/// A message of an open conversation, named by the row it hangs under and its place under it.
export function insideId(index: number, at: number): string {
  return `${LIST_ID}-row-${index}-inside-${at}`;
}
