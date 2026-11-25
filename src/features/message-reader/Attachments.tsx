import { useEffect, useSyncExternalStore } from "react";
import { ChevronDown, Loader2 } from "lucide-react";

import { describeError, useI18n } from "@/shared/i18n";
import {
  isCommandError,
  openAttachment,
  saveAttachment,
} from "@/shared/ipc/client";
import type { AttachedDto } from "@/shared/ipc/generated/AttachedDto";
import type { CommandError } from "@/shared/ipc/generated/CommandError";
import { inRoundNumbers } from "@/shared/ui/bytes";
import { cn } from "@/shared/ui/cn";
import { Button } from "@/shared/ui/primitives/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/primitives/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/shared/ui/primitives/tooltip";

import { FileName, kindOf } from "./FileName";
import {
  began,
  ended,
  failureAt,
  forgetAllBut,
  isOnItsWay,
  placeOf,
  said,
  subscribe,
  versionNow,
} from "./filesAsked";

/** What a refusal is, whatever shape it arrived in. */
function asCommandError(reason: unknown): CommandError {
  return isCommandError(reason)
    ? reason
    : { code: "internal", message: "", path: null };
}

/**
 * A file's name set apart from the sentence it is said in, so that a name written right to left
 * does not turn the sentence around in a window that runs left to right, nor the other way.
 */
function isolated(name: string): string {
  return `⁨${name}⁩`;
}

/**
 * The files a message came with: each a button that opens it where its own bytes say it is a
 * document of pages or a picture, and saves it otherwise, with a menu beside a file that opens that
 * saves it as well.
 *
 * Shown under a body that was drawn and under one there was nothing to draw alike: a message whose
 * whole content is an attachment, and a reply too deep to render whose pictures travelled inside it,
 * both have files to hand over and no body to hang them beneath. Nothing where the message carried
 * nothing.
 *
 * Whether a file opens is the core's word, read from its bytes as the message was drawn and read
 * again as it is opened, and never the name this row shows. A file that does not open after all, or
 * is not saved, is said in a line above the files, one for each such file, and saving it is still
 * there. What was asked, and what it came to, outlives the row (`filesAsked`).
 */
export function Attachments({
  messageId,
  files,
}: {
  messageId: string;
  files: AttachedDto[];
}) {
  const { t, language } = useI18n();
  // Drawn again whenever what was asked of any file changes.
  useSyncExternalStore(subscribe, versionNow);

  useEffect(() => {
    forgetAllBut(messageId);
  }, [messageId]);

  if (files.length === 0) return null;

  const isOpening = (file: AttachedDto) =>
    isOnItsWay(placeOf(messageId, file.at));

  // Asked by position. The name is shown and never sent: two files may share one, and where a
  // saved file goes is the system's dialog to decide. Closing the dialog is an answer and says
  // nothing; a save that fails says so, where a file that did not open says so — that line sends
  // the person here.
  const save = (file: AttachedDto) => {
    const place = placeOf(messageId, file.at);
    said(place, null);
    void saveAttachment(messageId, file.at).catch((reason: unknown) => {
      said(place, {
        messageId,
        name: file.name,
        error: asCommandError(reason),
      });
    });
  };

  const open = (file: AttachedDto) => {
    const place = placeOf(messageId, file.at);
    if (!began(place)) return;

    openAttachment(messageId, file.at).then(
      () => ended(place),
      (reason: unknown) =>
        ended(place, {
          messageId,
          name: file.name,
          error: asCommandError(reason),
        }),
    );
  };

  // One line for each file that failed, and one line however many files a failure that names
  // none of them befell: two lines saying the same thing tell the person nothing more.
  const lines = [
    ...new Set(
      files.flatMap((file) => {
        const failure = failureAt(placeOf(messageId, file.at));
        return failure
          ? [
              describeError(failure.error, t, false, {
                name: isolated(failure.name),
              }),
            ]
          : [];
      }),
    ),
  ];

  return (
    <div className="flex flex-col gap-1 border-t border-border px-6 py-3">
      <p className="text-xs font-medium text-muted-foreground">
        {t("reading.attachments")}
      </p>
      {/* Above the files rather than under them: the row stands at the foot of the reader, so a line
          under it would lift the file just pressed from under the pointer, and one above it moves
          only the heading. A name without a space in it wraps where it must rather than run out of
          the pane. */}
      {lines.map((line) => (
        <p
          key={line}
          role="alert"
          className="min-w-0 text-sm text-muted-foreground [overflow-wrap:anywhere]"
        >
          {line}
        </p>
      ))}
      <ul className="flex flex-wrap gap-2">
        {files.map((file) => (
          // Never wider than the row: in the narrowest window a long name gives way from its start,
          // as `FileName` cuts it, and the chevron beside it stays on screen.
          <li key={file.at} className="flex min-w-0 max-w-full">
            {/* Said to be busy rather than disabled while its file is on its way: a disabled
                button loses the focus, and a person at the keyboard would be left nowhere. A
                second press is refused by `began`. */}
            <Button
              type="button"
              variant="outline"
              size="sm"
              className={cn(
                "min-w-0 shrink aria-disabled:opacity-50",
                file.opens && "rounded-e-none",
              )}
              aria-label={t(
                file.opens ? "reading.openAttachment" : "reading.saveAttachment",
                { name: file.name },
              )}
              aria-busy={isOpening(file) || undefined}
              aria-disabled={isOpening(file) || undefined}
              onClick={() => (file.opens ? open(file) : save(file))}
            >
              <FileName name={file.name} />
              <span className="ms-2 shrink-0 text-xs tabular-nums text-muted-foreground">
                {[kindOf(file.name), inRoundNumbers(file.bytes, language, t)]
                  .filter(Boolean)
                  .join(" · ")}
              </span>
            </Button>

            {file.opens && (
              <DropdownMenu>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <DropdownMenuTrigger asChild>
                      <Button
                        type="button"
                        variant="outline"
                        size="icon-sm"
                        className="-ms-px rounded-s-none"
                        aria-label={t("reading.attachmentWays", {
                          name: file.name,
                        })}
                        aria-busy={isOpening(file) || undefined}
                      >
                        {/* The sign that the file is on its way, where it moves nothing beside
                            it. */}
                        {isOpening(file) ? (
                          <Loader2
                            aria-hidden="true"
                            className="size-3 animate-spin motion-reduce:animate-none"
                          />
                        ) : (
                          <ChevronDown aria-hidden="true" className="size-3" />
                        )}
                      </Button>
                    </DropdownMenuTrigger>
                  </TooltipTrigger>
                  <TooltipContent>
                    {t("reading.attachmentWays", { name: file.name })}
                  </TooltipContent>
                </Tooltip>
                <DropdownMenuContent
                  align="end"
                  // As wide as its longest line rather than as the chevron, at whose width no line
                  // of it has room.
                  className="w-auto max-w-(--radix-dropdown-menu-content-available-width)"
                >
                  <DropdownMenuItem
                    disabled={isOpening(file)}
                    onSelect={() => open(file)}
                  >
                    {t("reading.openAttachmentItem")}
                  </DropdownMenuItem>
                  <DropdownMenuItem onSelect={() => save(file)}>
                    {t("reading.saveAttachmentItem")}
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}
