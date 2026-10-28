import { Check } from "lucide-react";
import { useId } from "react";

/**
 * A box to tick, with its words beside it.
 *
 * The system's own box is drawn in the system's light colours whatever the window wears, so this
 * one is drawn in the window's: a native checkbox with its look taken off and the palette's put on.
 * The words are its label, so pressing them ticks it, and the hint under them is read as its
 * description.
 */
export function Checkbox({
  label,
  hint,
  checked,
  disabled = false,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}) {
  const id = useId();
  const described = useId();

  return (
    <div className="flex items-start gap-3">
      {/* As tall as one line of the words beside it, with the box in its middle: the box stands
          level with the first line however the words are set around it. */}
      <span className="relative flex h-5 shrink-0 items-center">
        <input
          id={id}
          type="checkbox"
          checked={checked}
          disabled={disabled}
          aria-describedby={hint === undefined ? undefined : described}
          onChange={(event) => onChange(event.target.checked)}
          className="peer size-4 appearance-none rounded-[4px] border border-input bg-background transition-colors outline-none checked:border-primary checked:bg-primary focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:opacity-50"
        />
        <Check
          aria-hidden="true"
          strokeWidth={3}
          className="pointer-events-none absolute top-1/2 left-1/2 size-3 -translate-x-1/2 -translate-y-1/2 text-primary-foreground opacity-0 peer-checked:opacity-100"
        />
      </span>
      <div className="min-w-0">
        <label htmlFor={id} className="block text-sm leading-5 font-medium">
          {label}
        </label>
        {hint !== undefined && (
          <p id={described} className="text-xs text-muted-foreground">
            {hint}
          </p>
        )}
      </div>
    </div>
  );
}
