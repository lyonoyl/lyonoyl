import { useEffect, useRef, useState } from "react";

import { useI18n } from "@/shared/i18n";
import {
  closeMessageBody,
  onMessageLink,
  openOriginalBody,
  whatTheWindowWears,
} from "@/shared/ipc/client";
import type { MessageBodyDto } from "@/shared/ipc/generated/MessageBodyDto";
import { LinkConfirmation, type Asked } from "@/shared/ui/LinkConfirmation";
import { MessageFrame } from "@/shared/ui/MessageFrame";

/**
 * The message being passed on in its own form, drawn beneath what is being written.
 *
 * Drawn as the reader draws a message: in a frame that runs nothing and reaches nothing, from a page
 * the core serves under a name of its own, with nothing from the web. Nothing in it is changed here,
 * and what is drawn is what goes: its markup never reaches the window, which knows it by a token.
 * When it cannot be drawn, nothing is drawn in its place: the message still goes whole beneath what
 * is written.
 */
export function ForwardedOriginal({ token }: { token: string }) {
  const { t } = useI18n();
  const [shown, setShown] = useState<"asking" | "refused" | MessageBodyDto>(
    "asking",
  );
  /** A link followed in this frame, waiting for the person's answer. */
  const [asked, setAsked] = useState<Asked | null>(null);
  /** The name the frame fetches from, and what is let go of. */
  const showing = useRef<string | null>(null);

  useEffect(() => {
    let active = true;

    openOriginalBody(token, whatTheWindowWears())
      .then((body) => {
        // Answered after the composer had gone, or moved on to another: nothing will show it.
        if (!active) {
          void closeMessageBody(body.name).catch(() => {});
          return;
        }
        showing.current = body.name;
        setShown(
          body.kind === "html" || body.kind === "plainText" ? body : "refused",
        );
      })
      .catch(() => {
        if (active) setShown("refused");
      });

    return () => {
      active = false;
      const name = showing.current;
      showing.current = null;
      if (name !== null) {
        void closeMessageBody(name).catch(() => {
          /* The store holds a few and drops the quietest. */
        });
      }
    };
  }, [token]);

  // A link in the message was followed. The frame cannot go anywhere, so it asked. Nothing else
  // that listens is drawn while a composer is (`showing` in `App.tsx`), so the question is this
  // frame's.
  useEffect(() => {
    let active = true;
    let stop: (() => void) | null = null;

    onMessageLink((link) => active && setAsked({ link, failed: false }))
      .then((unlisten) => {
        if (active) stop = unlisten;
        else unlisten();
      })
      .catch(() => {
        /* Without the listener a link does nothing, which is the safe direction. */
      });

    return () => {
      active = false;
      stop?.();
    };
  }, []);

  return (
    <>
      {asked && (
        <LinkConfirmation
          asked={asked}
          onCancel={() => setAsked(null)}
          onFailed={() => setAsked({ ...asked, failed: true })}
        />
      )}
      {shown === "refused" ? null : (
        <MessageFrame
          title={t("compose.original")}
          url={shown === "asking" ? undefined : shown.url}
          className="min-h-64 w-full flex-[2] rounded-md border border-input bg-white dark:bg-neutral-900"
        />
      )}
    </>
  );
}
