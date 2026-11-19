import type { ComponentProps } from "react";

import { typedDirection } from "@/shared/ui/direction";

/**
 * The longest signature the core keeps, in characters: `LONGEST_SIGNATURE` in the domain's
 * `signature` module, which `MailboxRow.test.tsx` reads to hold the two the same. The field stops the
 * typing there rather than taking a signature the core would then refuse.
 */
export const LONGEST_SIGNATURE = 2000;

/**
 * The field a mailbox's signature is typed into, in the settings and in a message being written:
 * lines of their own in the direction they are written, and no more of them than the core keeps.
 */
export function SignatureText({
  value,
  ...field
}: Omit<ComponentProps<"textarea">, "value" | "dir" | "maxLength"> & {
  value: string;
}) {
  return (
    <textarea
      {...field}
      dir={typedDirection(value)}
      value={value}
      maxLength={LONGEST_SIGNATURE}
      className="min-h-24 w-full resize-y rounded-lg border border-input bg-transparent px-2.5 py-2 text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 dark:bg-input/30"
    />
  );
}
