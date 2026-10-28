import type { ReactNode } from "react";

import { useTranslation } from "@/shared/i18n";
import { Label } from "@/shared/ui/primitives/label";

/**
 * One labelled thing to fill in, with the word for what may be left blank.
 *
 * Its own file because both screens of adding an account draw fields, and a form whose labels sat
 * a little differently on the second screen would read as a different form.
 */
export function Field({
  id,
  label,
  hint,
  required = false,
  children,
}: {
  id: string;
  label: string;
  hint?: ReactNode;
  /** Whether the form refuses to be sent without it. Said in the label, not only by the button. */
  required?: boolean;
  children: ReactNode;
}) {
  const t = useTranslation();

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-baseline gap-2">
        <Label htmlFor={id}>{label}</Label>
        {/* The mark is for the eye; the reader is told by the input's own `required`, which is
            what carries into the accessible name without changing the label's text. */}
        {required && (
          <span aria-hidden className="text-xs text-muted-foreground">
            {t("account.required")}
          </span>
        )}
      </div>
      {children}
      {/* Named, so the input can point at it. A hint the reader never hears is a hint that says
          "optional" to nobody — and that word is the only place this form said what may be left
          blank. */}
      {hint && (
        <p id={`${id}-hint`} className="text-xs text-muted-foreground">
          {hint}
        </p>
      )}
    </div>
  );
}
