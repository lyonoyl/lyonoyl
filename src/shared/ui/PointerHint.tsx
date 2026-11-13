import { useState, type ReactElement } from "react";

import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/shared/ui/primitives/tooltip";

/**
 * A control's name in a hint drawn in the theme's colours, which opens under the pointer and never
 * on the focus.
 *
 * Never on the focus, because these controls get it back by hand — a menu hands it back as it
 * closes, a screen as it is left — and a hint that opened on it stood over whatever came next, with
 * no pointer to take it away. The keyboard hears the control's own name, which is the words of the
 * hint. Under the pointer it is the hint every icon of the folded sidebar has, where the system's
 * own, drawn in its colours and letters, made the one control that looked like another
 * application's.
 */
export function PointerHint({
  label,
  side,
  children,
}: {
  /** What the control is: the words of its own name. */
  label: string;
  /** Where the hint opens; the primitive's own place where there is none. */
  side?: "top" | "right" | "bottom" | "left";
  children: ReactElement;
}) {
  const [pointedAt, setPointedAt] = useState(false);

  return (
    <Tooltip
      open={pointedAt}
      onOpenChange={(open) => {
        if (!open) setPointedAt(false);
      }}
    >
      <TooltipTrigger
        asChild
        onPointerEnter={() => setPointedAt(true)}
        onPointerLeave={() => setPointedAt(false)}
      >
        {children}
      </TooltipTrigger>
      <TooltipContent side={side}>{label}</TooltipContent>
    </Tooltip>
  );
}
