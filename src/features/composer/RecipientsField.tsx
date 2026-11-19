import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";

import { suggestRecipients } from "@/shared/ipc/client";
import type { SuggestedRecipientDto } from "@/shared/ipc/generated/SuggestedRecipientDto";
import { cn } from "@/shared/ui/cn";
import { Input } from "@/shared/ui/primitives/input";

import { beingTyped, looksLikeAnAddress, withChosen } from "./addresses";

/** Nobody offered: one list, so setting it again draws nothing again. */
const NOBODY: SuggestedRecipientDto[] = [];

/**
 * A line of addresses that offers, as one is typed, the people the account sending has written to.
 *
 * Who is offered, how many and in what order is the core's answer, from the mail on this device:
 * the field asks with the address being typed and draws what comes back. When nobody is offered,
 * or the question could not be answered, it shows nothing and the line is an ordinary one.
 *
 * The first person offered is the one Enter and Tab take, so a few letters and Enter write an
 * address. Not once the line holds a whole address: that one is somebody's own typing, and
 * another address that holds it is taken only when somebody chooses it.
 *
 * The core reads every message an account holds to know who it has written to, and keeps what it
 * read until a newer message arrives. The field asks it to as it takes the focus, so the reading
 * is done while a person reaches for the first key rather than after it.
 */
export function RecipientsField({
  id,
  labelId,
  accountId,
  value,
  onChange,
  disabled = false,
  autoFocus = false,
}: {
  /** The field's own, which its label points at. */
  id: string;
  /** The label's, which names the list of people offered as well as the field. */
  labelId: string;
  /** The account the message goes from: whose mail the people are found in. */
  accountId: string;
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
  autoFocus?: boolean;
}) {
  const input = useRef<HTMLInputElement>(null);
  const [people, setPeople] = useState<SuggestedRecipientDto[]>(NOBODY);
  /** Which of them Enter and Tab take, by place in the list, or none. */
  const [chosen, setChosen] = useState<number | null>(null);
  /** The newest question asked. An answer to an older one comes too late to be drawn. */
  const asked = useRef(0);
  /** Where the caret goes once the line holds what was chosen. */
  const caretAfter = useRef<number | null>(null);
  const listId = `${id}-offered`;

  function close() {
    asked.current += 1;
    setPeople(NOBODY);
    setChosen(null);
  }

  function ask(written: string, caret: number) {
    const question = (asked.current += 1);
    const { typed } = beingTyped(written, caret);

    if (typed.trim() === "" || accountId === "") {
      setPeople(NOBODY);
      setChosen(null);
      return;
    }

    suggestRecipients(accountId, typed).then(
      (offered) => {
        if (question !== asked.current) return;
        setPeople(offered.length > 0 ? offered : NOBODY);
        setChosen(
          offered.length > 0 && !looksLikeAnAddress(typed.trim()) ? 0 : null,
        );
      },
      () => {
        // Nothing to offer is what a person sees either way: the line still takes any address.
        if (question !== asked.current) return;
        setPeople(NOBODY);
        setChosen(null);
      },
    );
  }

  function gatherAhead() {
    if (accountId === "") return;
    suggestRecipients(accountId, "").catch(() => {
      /* Nothing is offered either way, and the first key asks again. */
    });
  }

  function choose(place: number) {
    const person = people[place];
    if (person === undefined) return;

    const caret = input.current?.selectionStart ?? value.length;
    const next = withChosen(value, caret, person.address);
    caretAfter.current = next.caret;
    close();
    onChange(next.written);
  }

  // The caret after the address chosen, once the line holds it: a controlled field put at the end
  // of what it was given would leave it there, past the addresses after this one.
  useLayoutEffect(() => {
    const caret = caretAfter.current;
    if (caret === null) return;
    caretAfter.current = null;
    input.current?.setSelectionRange(caret, caret);
  }, [value]);

  // Offered from one account's mail and for a line that could be typed into: another account, or
  // a line that stopped taking typing, is a question nobody asked.
  useEffect(() => {
    asked.current += 1;
    setPeople(NOBODY);
    setChosen(null);
  }, [accountId, disabled]);

  function onKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (people.length === 0) return;
    // An input method's own keys: Enter there settles what is being composed.
    if (event.nativeEvent.isComposing || event.keyCode === 229) return;
    if (event.metaKey || event.ctrlKey || event.altKey) return;

    switch (event.key) {
      case "ArrowDown":
        if (event.shiftKey) return;
        event.preventDefault();
        setChosen((now) =>
          now === null ? 0 : Math.min(now + 1, people.length - 1),
        );
        return;
      case "ArrowUp":
        if (event.shiftKey) return;
        event.preventDefault();
        setChosen((now) => (now === null || now === 0 ? null : now - 1));
        return;
      case "Enter":
      case "Tab":
        if (chosen === null || event.shiftKey) return;
        event.preventDefault();
        choose(chosen);
        return;
      case "Escape":
        // The list's to take before the window, which leaves the message on Escape: closing what
        // is offered is not leaving what is written.
        event.preventDefault();
        event.stopPropagation();
        close();
        return;
      case "ArrowLeft":
      case "ArrowRight":
      case "Home":
      case "End":
        // The caret is going to another address, and what is offered was for this one.
        close();
        return;
    }
  }

  const open = people.length > 0;

  return (
    <div className="relative">
      <Input
        ref={input}
        id={id}
        role="combobox"
        aria-autocomplete="list"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        aria-activedescendant={
          open && chosen !== null ? `${listId}-${chosen}` : undefined
        }
        autoComplete="off"
        dir="ltr"
        autoCapitalize="none"
        autoCorrect="off"
        spellCheck={false}
        autoFocus={autoFocus}
        disabled={disabled}
        value={value}
        onChange={(event) => {
          const written = event.target.value;
          onChange(written);
          ask(written, event.target.selectionStart ?? written.length);
        }}
        onKeyDown={onKeyDown}
        // A press inside the line moves the caret, perhaps into another address.
        onMouseDown={close}
        onFocus={gatherAhead}
        onBlur={close}
      />
      {open && (
        <ul
          id={listId}
          role="listbox"
          aria-labelledby={labelId}
          className="absolute inset-x-0 top-full z-20 mt-1 flex flex-col overflow-hidden rounded-lg bg-popover p-1 text-popover-foreground shadow-md ring-1 ring-foreground/10"
        >
          {people.map((person, place) => (
            <li
              key={person.address}
              id={`${listId}-${place}`}
              role="option"
              aria-selected={place === chosen}
              // The line keeps the focus: given to the list, it would close the list before the
              // click lands on anybody.
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => choose(place)}
              className={cn(
                "flex min-w-0 items-baseline gap-2 rounded-md px-2 py-1.5 text-sm",
                place === chosen
                  ? "bg-accent text-accent-foreground"
                  : "hover:bg-accent/50",
              )}
            >
              {person.name !== null && (
                <bdi className="min-w-0 shrink truncate">{person.name}</bdi>
              )}
              <span
                dir="ltr"
                className="min-w-0 shrink-[2] truncate text-muted-foreground"
              >
                {person.address}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
