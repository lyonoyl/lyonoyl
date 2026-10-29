import { ClearableInput } from "@/shared/ui/ClearableInput";
import { typedDirection } from "@/shared/ui/direction";

/** The field a group's name is typed into, with the search box's cross once there is something in it. */
export function GroupNameField({
  label,
  value,
  onChange,
  placeholder,
}: {
  /** What a reader hears the field called, in the words of the panel it is in. */
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
}) {
  return (
    <ClearableInput
      dir={typedDirection(value)}
      value={value}
      onChange={(event) => onChange(event.target.value)}
      placeholder={placeholder}
      aria-label={label}
      maxLength={64}
    />
  );
}
