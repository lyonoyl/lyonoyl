import type { ReactNode } from "react";

import { useI18n } from "@/shared/i18n";
import { Button } from "@/shared/ui/primitives/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/shared/ui/primitives/tooltip";

const NO_HINTS: string[] = [];

/**
 * A button that is only an icon, with its name in a hint beside it.
 *
 * What the folded sidebar is made of: Compose, the button that gets mail, every mailbox, and the
 * marks that stand for what the open sidebar would say in words. The name is the button's own as
 * well as the hint's, so a screen reader hears what a pointer is shown, and the hint opens on the
 * keyboard's focus as it does under the pointer.
 *
 * The hint opens towards the mail rather than over the edge of the window: to the right of a
 * column on the left, and to the left of one a mirrored window puts on the right.
 */
export function RailButton({
  label,
  hints = NO_HINTS,
  onClick,
  current = false,
  dot = false,
  busy,
  variant = "ghost",
  size = "icon-lg",
  tone = "quiet",
  disabled,
  hidden,
  className = "",
  children,
}: {
  /** What the button is, said by the hint and by the button itself. */
  label: string;
  /** Lines under the name, such as whose mailbox it is and how many are unread. */
  hints?: string[];
  onClick: () => void;
  /** The mailbox being read, marked the way the row in the open sidebar is. */
  current?: boolean;
  /**
   * Something unread or waiting inside: a dot, where the open sidebar has the number, because a
   * number in a corner this size is one nobody can read. The number is in the hint.
   */
  dot?: boolean;
  busy?: boolean;
  variant?: "ghost" | "default";
  size?: "icon-lg" | "icon-sm";
  /**
   * The colour the open sidebar says the same thing in: `quiet` for a mailbox, `alert` for
   * something that went wrong, `news` for a newer version, and `plain` beside the gear.
   */
  tone?: "quiet" | "alert" | "news" | "plain";
  disabled?: boolean;
  hidden?: boolean;
  className?: string;
  /** The icon. */
  children: ReactNode;
}) {
  const { dir } = useI18n();

  const colour =
    variant === "default"
      ? ""
      : tone === "alert"
        ? "text-destructive"
        : tone === "news"
          ? "text-primary"
          : tone === "plain"
            ? ""
            : current
              ? "bg-muted text-foreground"
              : "text-muted-foreground";

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant={variant}
          size={size}
          aria-current={current ? "true" : undefined}
          aria-busy={busy}
          disabled={disabled}
          hidden={hidden}
          onClick={onClick}
          className={`relative ${colour} ${hidden ? "hidden" : ""} ${className}`}
        >
          {children}
          {/* A line each, so the name a screen reader hears has a pause between them rather than
              the address run into the count. */}
          <span className="sr-only">
            <span className="block">{label}</span>
            {hints.map((line) => (
              <span key={line} className="block">
                {line}
              </span>
            ))}
          </span>
          {dot && (
            <span
              aria-hidden="true"
              className="absolute end-1 top-1 size-2 rounded-full bg-primary"
            />
          )}
        </Button>
      </TooltipTrigger>
      <TooltipContent side={dir === "rtl" ? "left" : "right"}>
        <span dir="auto" className="block">
          {label}
        </span>
        {hints.map((line) => (
          <span key={line} dir="auto" className="block opacity-80">
            {line}
          </span>
        ))}
      </TooltipContent>
    </Tooltip>
  );
}
