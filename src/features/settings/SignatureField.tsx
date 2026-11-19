import { useState } from "react";

import { Label } from "@/shared/ui/primitives/label";
import { SignatureText } from "@/shared/ui/SignatureText";

/**
 * One mailbox's signature, kept when the field is left, as a group's name is.
 *
 * What the core kept wins when it changes underneath, so once kept the field shows the signature as
 * it will stand in a message: on lines of its own, without blank lines before it or spaces after
 * it.
 */
export function SignatureField({
  id,
  label,
  hint,
  stored,
  unsaved,
  onKeep,
  onGiveUp,
}: {
  id: string;
  /** What the field is called, with the mailbox it belongs to. */
  label: string;
  /** Where the signature goes, said under the field. */
  hint: string;
  /** The signature as the core keeps it, empty for none. */
  stored: string;
  /** What was typed and could not be kept, which the field opens with so it is not lost. */
  unsaved: string | null;
  onKeep: (typed: string) => void;
  /** Told an edit was given up, which gives up a failure to keep it too. */
  onGiveUp: () => void;
}) {
  const hintId = `${id}-hint`;
  const [typed, setTyped] = useState(unsaved ?? stored);
  // Compared while rendering rather than in an effect, as a group's name is in the settings: an
  // effect lands after the paint, over whatever was typed in between.
  const [seen, setSeen] = useState(stored);
  if (seen !== stored) {
    setSeen(stored);
    setTyped(stored);
  }

  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      <SignatureText
        id={id}
        // Opened to be written in: the button that opens it is where the pointer or the keyboard
        // already is, and the field is the next thing anybody does.
        autoFocus
        value={typed}
        rows={4}
        aria-describedby={hintId}
        onChange={(event) => setTyped(event.target.value)}
        onBlur={() => {
          if (typed !== stored) onKeep(typed);
        }}
        onKeyDown={(event) => {
          // Only when there is an edit to give up, as in a group's name: otherwise Escape is the
          // window's, and leaves the settings.
          if (event.key === "Escape" && typed !== stored) {
            event.stopPropagation();
            setTyped(stored);
            onGiveUp();
          }
        }}
      />
      <p id={hintId} className="text-xs text-muted-foreground">
        {hint}
      </p>
    </div>
  );
}
