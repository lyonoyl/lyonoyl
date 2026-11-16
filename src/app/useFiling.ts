import { useCallback, useEffect, useRef, useState } from "react";

import type { Filed, PutAway } from "@/features/message-list/FilingNotice";
import type { FolderIdDto } from "@/shared/ipc/generated/FolderIdDto";

/** What the list is told about a message filed somewhere else in the window, as `filed`. */
type ToldTheList = {
  messageId: string;
  state: "gone" | "back" | "unread";
  folderId?: FolderIdDto;
} | null;

/**
 * The message just filed away and what became of it, as the list, the reader and the notice under
 * the list hear it, with the way back that the notice's button offers and the key offers too.
 *
 * Two panes, one action: the buttons are on the message being read, the row is in the list, and
 * the line saying what happened belongs under the list rather than on a message nobody is looking
 * at any more. Held in one place because the three must tell one story: a message comes back once,
 * whichever way back asked for it, and the corner under the list is free exactly when the notice
 * has nothing to say there.
 *
 * `setListGeneration` is the list's own, bumped when a message is back so the list reads itself
 * again.
 */
export function useFiling(
  setListGeneration: (bump: (generation: number) => number) => void,
) {
  /** The message that has just been filed away, which the list and the reader are told. */
  const [filed, setFiled] = useState<Filed | null>(null);
  /**
   * What the notice under the list is about: the message just filed, or what the key has asked for
   * back since.
   *
   * Kept apart from `filed`. The key raising the notice again about a message filed a minute ago
   * must not tell the list to play that row out a second time.
   */
  const [notice, setNotice] = useState<PutAway | null>(null);
  /**
   * What the key takes back, for as long as the window is open: the last message somebody filed
   * away or threw out themselves, or the trash emptied after it. Not only while the notice shows,
   * since the core can put a message back long after the notice has gone.
   */
  const putAway = useRef<PutAway | null>(null);
  /** Whether that is a message the key would take back now, which greys the menu's Undo or not. */
  const [canTakeBack, setCanTakeBack] = useState(false);
  /** Bumped when somebody asks for it back by key; the notice is what knows how to answer. */
  const [takeBackAsked, setTakeBackAsked] = useState(0);
  /**
   * What the notice is asking the core to take back, while the answer is on its way. Held here
   * rather than in the notice, which a screen opened in the meantime takes out of the tree: the
   * key pressed again once it has closed would ask a second time, and the late first answer would
   * leave the second line saying the message could not be put back over a list that has it.
   */
  const inFlight = useRef<PutAway | null>(null);
  /** The one just put back, which is a different thing to tell the list than a filing. */
  const [back, setBack] = useState<string | null>(null);
  /** A message marked read by being opened and put back to unread from the notice. */
  const [unreadAgain, setUnreadAgain] = useState<{
    messageId: string;
    folderId: FolderIdDto;
  } | null>(null);
  // Handed to the list once. The list marks it in the effects of the render that carries it, which
  // run before this one; left standing, a list remounted by a change of mailbox would mark it again.
  useEffect(() => {
    if (unreadAgain !== null) setUnreadAgain(null);
  }, [unreadAgain]);

  const filedAway = useCallback((what: Filed) => {
    setBack(null);
    setFiled(what);
    setNotice(what);
    // Not a message marked read by being opened, which nobody asked for: `PutAway` says why.
    if (what.what !== "read") {
      putAway.current = what;
      setCanTakeBack(true);
    }
  }, []);

  const putBack = useCallback(
    (messageId: string, sayingWhere = false) => {
      // Back, by the notice's button or by the key: nothing is left for the key to take back, and
      // nothing for the notice to say about it -- unless it is the line the key raised, which says
      // where the message went back to and then goes by itself, as any line does.
      const about = (one: PutAway | null) =>
        one !== null && one !== "forGood" && one.messageId === messageId;
      if (about(putAway.current)) {
        putAway.current = null;
        setCanTakeBack(false);
      }
      if (!sayingWhere) setNotice((was) => (about(was) ? null : was));
      setBack(messageId);
      // Read again as well as shown again: once the server has carried the filing out and a pass
      // has read it back, the list no longer holds the row it would show, and only its own answer
      // -- the way back laid over the index -- brings the message back into it.
      setListGeneration((generation) => generation + 1);
    },
    [setListGeneration],
  );

  /**
   * Forgotten once the notice about it has gone from the screen.
   *
   * What this holds is "there is a notice showing". Never cleared, it stayed true for the rest of
   * the session after the first message anybody filed -- and anything else that wanted the same
   * corner, like the offer to empty the trash, was never seen again.
   */
  const forgetWhatWasFiled = useCallback(() => {
    setFiled(null);
    setNotice(null);
  }, []);

  /** Put back to unread from the notice: the list marks it, and the notice is off the screen. */
  const markUnreadAgain = useCallback(
    (messageId: string, folderId: FolderIdDto) => {
      setUnreadAgain({ messageId, folderId });
      setNotice(null);
    },
    [],
  );

  /**
   * The trash emptied: the last thing done to mail now, and the one thing nothing brings back, so
   * the key says so rather than reaching past it for what was put away before.
   */
  const trashEmptied = useCallback(() => {
    putAway.current = "forGood";
    setCanTakeBack(false);
  }, []);

  /**
   * The key: the notice is raised about what was last put away and asked for it back. Nothing
   * while an answer is on its way, which is what the press would ask for again.
   */
  const takeBackByKey = useCallback(() => {
    const last = putAway.current;
    if (last === null || inFlight.current !== null) return;
    setNotice(last);
    setTakeBackAsked((asked) => asked + 1);
  }, []);

  const told: ToldTheList =
    unreadAgain !== null
      ? { ...unreadAgain, state: "unread" }
      : back !== null
        ? { messageId: back, state: "back" }
        : // A message marked read has not gone anywhere. Telling the list it had would slide out
          // the row somebody is reading.
          filed === null || filed.what === "read"
          ? null
          : { messageId: filed.messageId, state: "gone" };

  return {
    filed,
    notice,
    takeBackAsked,
    inFlight,
    canTakeBack,
    told,
    filedAway,
    putBack,
    forgetWhatWasFiled,
    markUnreadAgain,
    trashEmptied,
    takeBackByKey,
  };
}
