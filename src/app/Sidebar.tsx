import { PanelLeftClose, PanelLeftOpen, SquarePen } from "lucide-react";
import { useEffect, useRef, useState, type ComponentProps } from "react";

import { SettingsButton } from "@/app/SettingsButton";
import { ActivityLine } from "@/features/activity/ActivityLine";
import { MailboxTree } from "@/features/mailbox-tree/MailboxTree";
import { SettingsSections } from "@/features/settings/SettingsSections";
import { UpdateStrip } from "@/features/updates/UpdateStrip";
import { useI18n } from "@/shared/i18n";
import {
  getSidebarFolded,
  setSidebarFolded,
  sidebarFoldedAtStart,
} from "@/shared/ipc/client";
import type { PreferencesDto } from "@/shared/ipc/generated/PreferencesDto";
import { Button } from "@/shared/ui/primitives/button";
import { RailButton } from "@/shared/ui/RailButton";
import { EDGE_MOVES_FOR, ResizableColumn } from "@/shared/ui/ResizableColumn";

/**
 * How wide the folded sidebar is: an icon with room round it, and on macOS the room the window's
 * three buttons take, since they float over this column's top and nothing else of the window is
 * under them.
 */
export const RAIL = 72;

/**
 * The column on the window's leading edge: Compose, the mailboxes, and the foot with the gear.
 *
 * It folds into a rail of icons and opens again by the button after the gear. The rail is the same
 * column drawn as icons whose names are in their hints — Compose, the button that gets mail, and
 * the mailboxes an icon can stand for: the folders of the only account, or with several accounts
 * All inboxes alone — and what the open sidebar says in words is a mark there whose press opens
 * it. What is being read stays what it was, folded or open.
 *
 * Folded or open is kept on this device beside the column's width, and a first start opens it.
 * What is kept is drawn from the first frame as it was left, without moving: the core reads it
 * before it makes the window and says it in the address the window opens at. It is read again once
 * the window is up, for a window started again since, whose address is as old as the window. The
 * edge moves for a person folding it, and for nothing else.
 */
export function Sidebar({
  width,
  onResize,
  settingsInFront,
  empty,
  ready,
  onCompose,
  tree,
  preferences,
  onOpenSettings,
}: {
  /** How wide it was dragged, which it goes back to when it opens. */
  width: number;
  onResize: (width: number) => void;
  /** Whether the settings are the screen in front, which this column lists the parts of. */
  settingsInFront: boolean;
  /** Nobody has connected a mailbox yet. */
  empty: boolean;
  /** Whether the mail has been read, which is when there is something to write from. */
  ready: boolean;
  onCompose: () => void;
  /** The mailboxes, as the window hands them over: this column decides only how they are drawn. */
  tree: Omit<ComponentProps<typeof MailboxTree>, "folded" | "onUnfold">;
  preferences: PreferencesDto | null;
  onOpenSettings: () => void;
}) {
  const { t } = useI18n();
  const [folded, setFolded] = useState(sidebarFoldedAtStart);
  /** Whether the edge is on its way, for as long as it takes to get there. */
  const [moving, setMoving] = useState(false);
  const arrives = useRef<number | null>(null);
  /**
   * Whether somebody has folded or opened it since the window started. What was kept is read again
   * after the first draw, and arriving after a press it would undo the press.
   */
  const pressed = useRef(false);

  useEffect(() => {
    let active = true;

    getSidebarFolded()
      .then((kept) => {
        if (active && !pressed.current) setFolded(kept);
      })
      .catch(() => {
        /* Open, which is how a first start opens it. */
      });

    return () => {
      active = false;
      if (arrives.current !== null) window.clearTimeout(arrives.current);
    };
  }, []);

  const fold = (to: boolean) => {
    pressed.current = true;
    setFolded(to);
    setMoving(true);
    if (arrives.current !== null) window.clearTimeout(arrives.current);
    arrives.current = window.setTimeout(() => {
      arrives.current = null;
      setMoving(false);
    }, EDGE_MOVES_FOR);

    setSidebarFolded(to).catch(() => {
      /* It stays this way until the app is closed, as a width that could not be kept does. */
    });
  };
  const unfold = () => fold(false);

  /**
   * Whether the column is drawn folded: as it was left, except beside the settings, whose parts it
   * lists whole. The fold itself is kept as it was, and the rail comes back when they close. What
   * is hidden beside the settings keeps the form the fold gives it, so the focus comes back to the
   * very control it left.
   */
  const drawnFolded = folded && !settingsInFront;

  return (
    <ResizableColumn
      width={width}
      onResize={onResize}
      label={settingsInFront ? t("settings.title") : t("shell.mailboxes")}
      foldsTo={RAIL}
      folded={drawnFolded}
      moving={moving}
      className="border-e border-border"
    >
      {/* Laid out, while the edge moves, at the width it is going to, so nothing inside reflows on
          the way: opening, the edge uncovers the sidebar as it will stand, and folding, the rail is
          where it will stay while the edge closes in on it.

          `under-window-controls`: where the window has no bar of its own, its buttons float over
          this corner, and the stylesheet moves what is here down out from under them. Less below
          than above: the foot is a status line, and height given to it is height taken from the
          mailboxes. */}
      <div
        style={moving ? { width: `${drawnFolded ? RAIL : width}px` } : undefined}
        className={`under-window-controls flex min-h-0 flex-1 flex-col pt-4 pb-2 ${
          drawnFolded ? "items-center gap-2" : "gap-4 px-4"
        }`}
      >
        {/* Nothing to write from, so nothing to press. Hidden rather than disabled: a disabled
            button is a promise about later, and what a person needs here is the one button that
            does something.

            Out of the column while the settings are in front as well, so the list of their parts
            starts at the top where this stood. Hidden rather than taken out: somebody who tabbed
            to it and opened the settings by key gets the focus back on it when they close. */}
        {!empty &&
          (folded ? (
            <RailButton
              variant="default"
              label={t("shell.compose")}
              hidden={settingsInFront}
              disabled={!ready}
              onClick={onCompose}
            >
              <SquarePen />
            </RailButton>
          ) : (
            <Button
              hidden={settingsInFront}
              className={settingsInFront ? "hidden" : "w-full"}
              disabled={!ready}
              onClick={onCompose}
            >
              {t("shell.compose")}
            </Button>
          ))}
        {/* While the settings are in front, the column beside them lists what they are made of
            rather than the folders of an account. Those rows answer a question nobody asks with
            this screen open, and the list of parts is how a long screen is got around: it is
            listed whole, with the column open whatever was kept. */}
        {settingsInFront && <SettingsSections />}

        {/* Hidden rather than taken away. What it holds is not only a list: it remembers which
            accounts it has already asked their server about, so an account whose folders never
            arrive is asked once and then left alone. Taken out of the tree and put back, it
            forgets that and asks again -- and with the settings swapping it in and out, "once"
            became "every time somebody opens the settings", with the button turning for each. The
            same is true of folding it, which draws the same tree as a rail. */}
        <div
          hidden={settingsInFront}
          className={
            settingsInFront
              ? "hidden"
              : `flex min-h-0 flex-col ${folded ? "w-full items-center" : ""}`
          }
        >
          <MailboxTree {...tree} folded={folded} onUnfold={unfold} />
        </div>

        {/* One foot. The line answers the two questions a background process creates -- is this
            current, and did what I asked for land -- and takes the width the buttons leave it,
            wrapping when it has more to say than one line holds; an update, when there is one,
            takes a line of its own above them. Every piece draws nothing when it has nothing to
            say.

            Along the bottom, so the gear and the button beside it stay at the column's foot
            however many lines stand beside them, one button tall: the foot's height is taken from
            the mailboxes. The line's padding below lifts its last line to the middle of the
            buttons. Folded, the pieces stand one above another, each a mark or nothing. */}
        <div
          className={`mt-auto flex gap-2 ${
            drawnFolded ? "w-full flex-col items-center" : "flex-wrap items-end"
          }`}
        >
          <UpdateStrip
            className={drawnFolded ? "" : "basis-full"}
            folded={drawnFolded}
            onUnfold={unfold}
          />
          <ActivityLine
            className={drawnFolded ? "w-full px-3" : "min-w-0 flex-1 pb-1.5"}
            folded={drawnFolded}
            onUnfold={unfold}
          />
          <div
            className={`flex items-center gap-1 ${drawnFolded ? "flex-col" : "ms-auto"}`}
          >
            {/* Gone while the settings are in front. The way out of a screen is the cross in its
                corner, and a button that opened what is already open is a second answer to a
                question nobody is asking. */}
            {!settingsInFront && (
              <SettingsButton
                storageIsShut={preferences === null}
                t={t}
                onOpen={onOpenSettings}
              />
            )}
            {/* After the gear: beside it while the sidebar is open, under it while folded. The same
                button in both, so a keyboard that pressed it is still on it once the column has
                moved.

                Hidden while the settings are in front, whose parts are listed whole: there is
                nothing there to fold. Hidden rather than taken out, as Compose is, so somebody who
                opened the settings by key from it gets the focus back on it. */}
            <RailButton
              size="icon-sm"
              tone="plain"
              hidden={settingsInFront}
              label={folded ? t("shell.unfoldSidebar") : t("shell.foldSidebar")}
              onClick={() => fold(!folded)}
            >
              {folded ? (
                <PanelLeftOpen className="rtl:-scale-x-100" />
              ) : (
                <PanelLeftClose className="rtl:-scale-x-100" />
              )}
            </RailButton>
          </div>
        </div>
      </div>
    </ResizableColumn>
  );
}
