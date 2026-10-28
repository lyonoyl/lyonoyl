import { useI18n } from "@/shared/i18n";
import type { WhatChangedDto } from "@/shared/ipc/generated/WhatChangedDto";

/**
 * What changed in the version now running, shown once after it was installed.
 *
 * **In the language the person chose.** The release writes a note for every catalogue the window
 * has, and the one shown is the one for the language in use; English, the language every catalogue
 * is kept level with, where a release wrote nothing in it.
 *
 * Stands over the list and the message the way the settings do, and is left the same way: by the
 * cross the pane puts in its corner. That cross is what marks it read — invariant 20 says leaving a
 * screen is the screen's own business, and marking it read on the way *in* would lose it for
 * somebody who was interrupted before reading a word.
 *
 * **The text is what the release wrote, and it is shown as text.** Not as markup, not as anything a
 * browser interprets: what a message may not do to this window, a release note may not do either,
 * and the day this project's own release process is the thing that was tampered with is exactly the
 * day that matters.
 */
export function WhatChanged({ changed }: { changed: WhatChangedDto }) {
  const { t, language } = useI18n();
  // The release names a note after its catalogue file, `pt-BR`; the window names a language in
  // lower case, `pt-br`. A tag is the same tag in either case.
  const own = Object.entries(changed.notes).find(
    ([tag]) => tag.toLowerCase() === language,
  )?.[1];
  // The first that says something: a note left blank in this language is no note in it.
  const notes =
    [own, changed.notes.en].find(
      (note) => note !== undefined && note.trim() !== "",
    ) ?? "";

  return (
    <div className="flex flex-col gap-4 p-6">
      <h2 className="text-lg font-semibold">
        {t("update.changesTitle", { version: changed.version })}
      </h2>

      {notes.trim() === "" ? (
        <p className="text-sm text-muted-foreground">
          {t("update.changesEmpty")}
        </p>
      ) : (
        // `whitespace-pre-wrap` because the release wrote lines and they are lines. Anything that
        // turned them into markup would be this window interpreting text that arrived over a
        // network.
        <p className="max-w-prose whitespace-pre-wrap text-sm leading-relaxed">
          {notes}
        </p>
      )}
    </div>
  );
}
