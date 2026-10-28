import { Fragment, useState } from "react";

import { listOf, useI18n } from "@/shared/i18n";
import { askAboutAWrittenLink } from "@/shared/ipc/client";
import type { SentHereDto } from "@/shared/ipc/generated/SentHereDto";
import { linksIn } from "@/shared/ui/links";
import { LinkConfirmation, type Asked } from "./LinkConfirmation";

/**
 * A message sent from this device whose account has not been read back with it yet, as the reader
 * shows it: who it went to, what it says and what it carries.
 *
 * Nothing here acts on it. Answering, filing and throwing away act on the server's copy, and that
 * copy takes this one's place in the list seconds after a server takes the message.
 *
 * The text is what the person wrote in the composer, which writes plain text: it is drawn as text,
 * the way the composer draws it, and nothing in it is read as markup. Its addresses are links, as
 * they are in its copy once that is read back, and through the same question: the core finds the
 * address in the message again and answers with what to ask, and opens nothing it did not find.
 */
export function SentHereReader({
  sent,
  accountId,
}: {
  sent: SentHereDto;
  /** The account it was sent from, which is where the core looks for it. */
  accountId: string | null;
}) {
  const { t, language } = useI18n();
  const [asked, setAsked] = useState<Asked | null>(null);

  const ask = (written: string) => {
    if (accountId === null) return;
    askAboutAWrittenLink(accountId, sent.id, written)
      .then((link) => setAsked({ link, failed: false }))
      .catch(() => {
        /* Read back since it was drawn: its copy takes this one's place, with the same links. */
      });
  };
  const addressed = [
    { field: "to", label: t("compose.to"), addresses: sent.to },
    { field: "cc", label: t("compose.cc"), addresses: sent.cc },
    { field: "bcc", label: t("compose.bcc"), addresses: sent.bcc },
  ].filter(({ addresses }) => addresses.length > 0);

  return (
    <article
      data-selectable
      className="flex min-h-0 flex-1 flex-col overflow-auto"
    >
      <header className="border-b border-border p-6">
        <h1 dir="auto" className="text-lg font-semibold tracking-tight">
          {sent.subject.trim() || t("list.noSubject")}
        </h1>

        <dl className="mt-4 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
          <dt className="text-muted-foreground">{t("reading.from")}</dt>
          <dd dir="auto">{sent.sender}</dd>

          {addressed.map(({ field, label, addresses }) => (
            <div key={field} className="contents">
              <dt className="text-muted-foreground">{label}</dt>
              {/* Joined the way the language joins a list, not with an English comma. */}
              <dd dir="auto">{listOf(language, addresses)}</dd>
            </div>
          ))}
        </dl>
      </header>

      <div
        dir="auto"
        className="p-6 text-sm leading-relaxed break-words whitespace-pre-wrap"
      >
        {piecesOf(sent.body).map((piece, at) =>
          piece.link ? (
            <button
              key={at}
              type="button"
              onClick={() => ask(piece.text)}
              className="inline text-start break-all text-primary underline underline-offset-2 hover:text-primary/80"
            >
              {piece.text}
            </button>
          ) : (
            <Fragment key={at}>{piece.text}</Fragment>
          ),
        )}
      </div>

      {asked && (
        <LinkConfirmation
          asked={asked}
          onCancel={() => setAsked(null)}
          onFailed={() => setAsked({ ...asked, failed: true })}
        />
      )}

      {sent.files.length > 0 && (
        <div className="flex flex-col gap-1 border-t border-border px-6 py-3">
          <p className="text-xs font-medium text-muted-foreground">
            {t("reading.attachments")}
          </p>
          <ul className="flex flex-wrap gap-2">
            {sent.files.map((name, at) => (
              <li
                key={at}
                dir="auto"
                className="rounded-md border border-border px-2 py-1 text-xs"
              >
                {name}
              </li>
            ))}
          </ul>
        </div>
      )}
    </article>
  );
}

/** The text cut at its addresses, in order: what is written between them, and each address. */
function piecesOf(text: string): { text: string; link: boolean }[] {
  const pieces: { text: string; link: boolean }[] = [];
  let at = 0;

  for (const { start, end } of linksIn(text)) {
    if (start > at) pieces.push({ text: text.slice(at, start), link: false });
    pieces.push({ text: text.slice(start, end), link: true });
    at = end;
  }
  if (at < text.length) pieces.push({ text: text.slice(at), link: false });

  return pieces;
}
