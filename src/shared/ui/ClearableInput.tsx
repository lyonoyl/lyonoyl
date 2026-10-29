import type { ComponentProps } from "react";

import { Input } from "@/shared/ui/primitives/input";

/**
 * A one-line field with a cross at its far end once something is typed into it, the one the search
 * box has.
 *
 * The cross is the engine's own, drawn because the field is a search field to it: in the colours of
 * the window's theme, gone while the field is empty, and emptying the field the way a person would.
 * A cross drawn by hand is a second cross, matching neither the theme nor the search box.
 *
 * To somebody using a screen reader it stays a plain field unless it says otherwise. A name typed
 * into a panel that announces itself as a search is one more thing to work out.
 */
export function ClearableInput({
  role = "textbox",
  ...props
}: Omit<ComponentProps<typeof Input>, "type">) {
  return <Input type="search" role={role} {...props} />;
}
