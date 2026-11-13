import { ArrowLeft, X } from "lucide-react";
import type { ReactNode, RefObject } from "react";

import { leaveScreen } from "@/app/leave-screen";
import { systemHere, type System } from "@/shared/platform";
import { PointerHint } from "@/shared/ui/PointerHint";
import { Button } from "@/shared/ui/primitives/button";

/**
 * The pane a screen takes over: the settings, writing a message, adding an account.
 *
 * It stands where the list and the message being read normally are, and covers both. That is what
 * makes the way out its job rather than each screen's: the control that opened it is behind it,
 * and a screen a person can only leave by finding the button they cannot see is a screen they are
 * stuck on. The cross sits over the content rather than in it, so it is in the same corner on
 * every screen and stays there while the screen is scrolled.
 *
 * Each screen mounts its own pane -- `key` in the caller, one per screen. Reusing one element
 * across them carried the scroll offset from the screen being left into the screen being opened,
 * so the settings arrived halfway down a page nobody had scrolled.
 */
export function ScreenPane({
  paneRef,
  closeLabel,
  onClose,
  onBack = null,
  backLabel = "",
  inFront = true,
  system = systemHere(),
  children,
}: {
  /**
   * Which system draws the window, which decides the cross's corner: on macOS the start of the
   * line, where the window's own buttons are, with the arrow back beside it; elsewhere the end, as
   * Windows and Linux put a window's close.
   */
  system?: System;
  /** The pane takes the focus when it opens, so tabbing continues inside what just appeared. */
  paneRef: RefObject<HTMLElement | null>;
  /**
   * Its own name, never the screen's own Cancel repeated.
   *
   * Two buttons called Cancel on one screen are two buttons a person listening cannot tell apart,
   * and they do different things: one leaves, the other gives up what was being filled in.
   */
  closeLabel: string;
  /**
   * Back to the step before this one, when the screen has one. Absent otherwise.
   *
   * Opposite the cross and in the same line, because they answer the same shape of question --
   * where does this take me -- and a person looks in the corners for both.
   */
  onBack?: (() => void) | null;
  backLabel?: string;
  /**
   * Absent where leaving is not free.
   *
   * A screen being filled in has to finish something before it may be left -- the composer writes
   * down what was typed in the last second, and refuses while a message is going out. Its own
   * control does that work; a cross in the corner that only closed the screen would look like the
   * same button and lose the text.
   */
  onClose: (() => void) | null;
  /**
   * Whether this is the screen in front.
   *
   * A screen opened over another does not close it, and the one underneath stays in the tree with
   * everything in it -- a half-written message is still half-written when the settings are put
   * away. Taken out of the tree instead, it would come back empty, which is the same as losing it.
   *
   * Hidden two ways on purpose: the class is what the window obeys, since a `flex` would otherwise
   * beat the attribute, and the attribute is what a screen reader and the tests obey.
   */
  inFront?: boolean;
  children: ReactNode;
}) {
  const back = (
    <div className="p-3">
      {onBack && (
        <PointerHint label={backLabel} side="bottom">
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={backLabel}
            className="bg-background/80 backdrop-blur"
            onClick={onBack}
          >
            <ArrowLeft className="rtl:-scale-x-100" />
          </Button>
        </PointerHint>
      )}
    </div>
  );
  const close = (
    <div className="p-3">
      {onClose && (
        <PointerHint label={closeLabel} side="bottom">
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={closeLabel}
            className="bg-background/80 backdrop-blur"
            onClick={() => leaveScreen(onClose)}
          >
            <X />
          </Button>
        </PointerHint>
      )}
    </div>
  );

  return (
    <section
      ref={paneRef}
      tabIndex={-1}
      hidden={!inFront}
      className={
        inFront
          ? "flex min-h-0 flex-1 flex-col overflow-y-auto outline-none"
          : "hidden"
      }
    >
      {/* Over the content rather than in a bar of its own, which would take a strip of height on
          every screen to hold one button. No height at all, so nothing below it moves; stuck to
          the top, so it is still there once a long screen has been scrolled; backed and blurred,
          so it is still a button when the screen scrolls under it. */}
      {(onClose || onBack) && (
        // On macOS the cross takes the corner the window's own buttons are in, with the arrow back
        // beside it; elsewhere the two keep opposite corners.
        <div
          className={`sticky top-0 z-10 flex h-0 ${
            system === "mac" ? "justify-start" : "justify-between"
          }`}
        >
          {system === "mac" ? (
            <>
              {close}
              {back}
            </>
          ) : (
            <>
              {back}
              {close}
            </>
          )}
        </div>
      )}

      {children}
    </section>
  );
}
