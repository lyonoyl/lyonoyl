import { useRef, type KeyboardEvent } from "react";
import { Select as SelectPrimitive } from "radix-ui";

import { isPlainKey } from "@/shared/ui/keys";
import {
  Select,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/shared/ui/primitives/select";

/** How long typing may pause before the next letter starts a new search rather than adding to it. */
const SEARCH_PAUSE_MS = 1000;

/** The open list's frame, as the primitive draws it, with the height this control allows. */
const LIST =
  "relative z-50 max-h-[min(24rem,var(--radix-select-content-available-height))] max-w-(--radix-select-content-available-width) min-w-36 origin-(--radix-select-content-transform-origin) overflow-x-hidden overflow-y-auto rounded-lg bg-popover text-popover-foreground shadow-md ring-1 ring-foreground/10 duration-100 data-[side=bottom]:translate-y-1 data-[side=left]:-translate-x-1 data-[side=right]:translate-x-1 data-[side=top]:-translate-y-1 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95";

/**
 * One answer out of a list, behind a control that shows the answer already given.
 *
 * Written once so every list built on it looks and answers alike: the same way of opening, and the
 * same keys — arrows, Home and End, Enter, and typing the first letters of an entry to jump to it.
 *
 * **Typing finds an entry by its name or by the code drawn before it,** and a pause of a second
 * starts the search again: the letters typed after it are a new search, not more of the last one.
 * The search is this control's own: the primitive's knows one text an entry begins with, so a list
 * showing a code beside each name could be searched by one of them and not the other.
 *
 * The list opens beside the control and is never taller than the room the window has on that side,
 * nor taller than a comfortable handful of rows; past that it scrolls inside itself. A list of
 * languages grows with every catalogue added to the build, and one that ran off the edge of a short
 * window would hide the entries at its end from exactly the people looking for them.
 *
 * **It scrolls as any list does, with no arrows at its ends.** The primitive's list puts an arrow at
 * an end there is more to scroll to, and takes it away at the end itself: the rows jump by the
 * arrow's height each time one comes or goes, and an arrow that comes scrolls the list back to the
 * entry in focus. So the list here is the primitive's frame and rows without the arrows, and it does
 * not bounce at its ends either.
 */
export function Chooser<T extends string | null>({
  label,
  value,
  options,
  onChoose,
  size = "default",
  className,
}: {
  /** What the control is called, for a reader that cannot see what stands beside it. */
  label: string;
  value: T;
  /** In the order they are offered. `lang` is the language an entry is written in, when it is
      not the window's own, so a screen reader says it in that language's voice; `dir` is `auto`
      for a name somebody chose, which keeps its own direction in a window that runs the other way;
      `code` is a short code drawn before the label, for the eye only. */
  options: ReadonlyArray<{
    value: T;
    label: string;
    lang?: string;
    dir?: "auto";
    code?: string;
  }>;
  /** Called only when the answer changes; choosing the one already given is not a change. */
  onChoose: (value: T) => void;
  size?: "sm" | "default";
  className?: string;
}) {
  // Each answer travels through the list underneath as its place in the list rather than as
  // itself. That list refuses an empty string as an answer, because there it means "nothing chosen",
  // and "none of them" is a real answer some of these lists offer; a made-up word standing in for it
  // would be a word a real value could one day be. A place in the list cannot collide with anything.
  const chosen = options.findIndex((option) => option.value === value);
  // Where any entry has a code, every entry keeps the code's column, so the labels stand in one line.
  const coded = options.some((option) => option.code);
  const list = useRef<HTMLDivElement>(null);
  const search = useRef({ typed: "", at: 0 });

  /**
   * A letter typed at the control or at the open list.
   *
   * It is kept from the window, which answers plain letters with its own shortcuts: handed on, the
   * `c` that jumps to a group called "Clients" would also open a message over the settings. Kept
   * here rather than refused by the window, because the list of messages is a list of options too,
   * and its letters are the window's.
   *
   * It moves to the entry the letters typed so far begin. The control, closed, takes that entry as
   * the answer, as it always has; the open list moves there and waits for Enter.
   */
  const typeAhead = (event: KeyboardEvent<HTMLElement>, open: boolean) => {
    if (event.key.length !== 1 || !isPlainKey(event) || event.nativeEvent.isComposing) return;
    event.stopPropagation();
    const now = Date.now();
    const fresh = now - search.current.at > SEARCH_PAUSE_MS;
    // A space opens the list or picks the entry in focus, as it always has, unless it comes inside
    // a name being typed.
    if (event.key === " " && (fresh || search.current.typed === "")) return;
    event.preventDefault();
    const typed = (fresh ? "" : search.current.typed) + event.key;
    search.current = { typed, at: now };

    const focused = (document.activeElement as HTMLElement | null)?.closest("[data-place]");
    const from = open && focused ? Number(focused.getAttribute("data-place")) : chosen;
    const place = beginning(options, typed, from);
    if (place === -1) return;
    if (open) {
      const entry = list.current?.querySelector<HTMLElement>(`[data-place="${place}"]`);
      entry?.focus();
      entry?.scrollIntoView({ block: "nearest" });
    } else if (place !== chosen) {
      onChoose(options[place].value);
    }
  };

  return (
    <Select
      value={chosen === -1 ? "" : String(chosen)}
      onValueChange={(place) => {
        const option = options[Number(place)];
        if (option) onChoose(option.value);
      }}
    >
      <SelectTrigger
        aria-label={label}
        size={size}
        className={className}
        onKeyDown={(event) => typeAhead(event, false)}
      >
        <SelectValue />
      </SelectTrigger>
      <SelectPrimitive.Portal>
        <SelectPrimitive.Content
          ref={list}
          data-slot="select-content"
          // Beside the control rather than laid over it, so the answer it shows stays in view while
          // the list is open, and so the room on each side is what bounds the list's height.
          position="popper"
          align="start"
          className={LIST}
          // Escape closes the list and goes no further. The window leaves the screen on Escape, and a
          // key that shut the list and the screen behind it together would throw away everything
          // somebody was looking at in order to take back one press.
          onEscapeKeyDown={(event) => event.stopPropagation()}
          onKeyDown={(event) => typeAhead(event, true)}
        >
          <SelectPrimitive.Viewport className="h-(--radix-select-trigger-height) w-full min-w-(--radix-select-trigger-width) overscroll-none">
            {options.map((option, place) => (
              <SelectItem
                key={place}
                value={String(place)}
                lang={option.lang}
                data-place={place}
                // The entry as plain text, where the primitive wants one: its name, without the code.
                textValue={option.label}
                // The entry in focus hears a key before the list does, and it takes a space as its
                // choice: a space inside a name being typed has to reach the search first.
                onKeyDown={(event) => typeAhead(event, true)}
              >
                {coded && (
                  <span
                    aria-hidden="true"
                    className="w-7 shrink-0 font-mono text-xs text-muted-foreground uppercase"
                  >
                    {option.code}
                  </span>
                )}
                {/* Its own box, so a long name is cut short with an ellipsis in the closed control
                    rather than pushing the control wider than the room it was given. */}
                <span dir={option.dir} className="truncate">
                  {option.label}
                </span>
              </SelectItem>
            ))}
          </SelectPrimitive.Viewport>
        </SelectPrimitive.Content>
      </SelectPrimitive.Portal>
    </Select>
  );
}

/**
 * Which entry the letters typed so far find: the first, going on from the one in focus and round to
 * the top, whose name or code begins with them. The same letter typed again moves on to the next
 * entry it begins rather than staying where it is.
 */
function beginning(
  options: ReadonlyArray<{ label: string; code?: string }>,
  typed: string,
  from: number,
): number {
  const repeated = [...typed].every((letter) => letter === typed[0]);
  // Lower case the same way on every system: the system's own rules would turn a typed `I` into a
  // dotless `ı` on a Turkish one, which begins no code at all.
  const sought = (repeated ? typed[0] : typed).toLowerCase();
  const begins = (option: { label: string; code?: string }) =>
    option.label.toLowerCase().startsWith(sought) ||
    (option.code?.toLowerCase().startsWith(sought) ?? false);
  // The same letter skips the entry it starts from, where there is one to start from.
  const skip = repeated && from >= 0 ? 1 : 0;
  const start = Math.max(from, 0);
  for (let step = skip; step < options.length + skip; step += 1) {
    const place = (start + step) % options.length;
    if (begins(options[place])) return place;
  }
  return -1;
}
