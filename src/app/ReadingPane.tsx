import { useEffect, useRef, useState } from "react";

import type { Answer } from "@/entities/message";
import type { Filed } from "@/features/message-list/FilingNotice";
import { LEAVING_MS } from "@/features/message-list/MessageRow";
import { MessageReader } from "@/features/message-reader/MessageReader";
import { SentHereReader } from "@/features/message-reader/SentHereReader";
import type { FolderIdDto } from "@/shared/ipc/generated/FolderIdDto";
import type { SentHereDto } from "@/shared/ipc/generated/SentHereDto";
import type { ShowPicturesDto } from "@/shared/ipc/generated/ShowPicturesDto";

/**
 * The place a message is read in, and the message leaving it.
 *
 * The reader is drawn under a key, and keeps it from one message to the next as somebody reads.
 * When the message in it is archived, thrown away or moved, that copy of the reader keeps its key
 * and the message it shows, and slides out over a new one under the next key, which reads the
 * next message — or says there is none — underneath. Kept whole, the message leaves as it was
 * drawn, with nothing read again for the sake of a movement, and it leaves with its row in the
 * list, in the same time.
 */
export function ReadingPane({
  sentHere,
  sentFrom,
  opened,
  filed,
  answerAsked,
  answerAllAsked,
  onReply,
  forwardAsked,
  throwAwayAsked,
  onForward,
  onFiled,
  nothingToChoose,
  showPictures,
}: {
  /** A message sent from this device, shown in the reader's place before it is read back. */
  sentHere: SentHereDto | null;
  /** The account the folder for sent mail being shown belongs to. */
  sentFrom: string | null;
  opened: { messageId: string; folderId: FolderIdDto | null } | null;
  /** The last message filed away anywhere in the window: by the reader, or by the list. */
  filed: Filed | null;
  answerAsked: number;
  answerAllAsked: number;
  onReply: (answer: Answer) => void;
  forwardAsked: number;
  throwAwayAsked: number;
  onForward: (forward: Answer) => void;
  onFiled: (what: Filed) => void;
  nothingToChoose: boolean;
  showPictures: ShowPicturesDto;
}) {
  const key = useRef(0);
  const [leaving, setLeaving] = useState<{
    key: number;
    messageId: string;
    folderId: FolderIdDto | null;
    /** What it was asked to answer when it left, so a key pressed now is not its. */
    asked: number;
    /** What it was asked to pass on when it left, for the same reason. */
    askedOn: number;
    /** What it was asked to answer everybody on when it left, for the same reason. */
    askedAll: number;
    /** What it was asked to throw away when it left, for the same reason. */
    askedAway: number;
  } | null>(null);
  /**
   * The filing last weighed, so a window drawn again over the same one does not weigh it twice.
   * One that happened before this was drawn — the pane goes while a screen stands in front of it —
   * is not weighed at all: its message left while nobody was looking.
   */
  const weighed = useRef<Filed | null>(filed);

  // A filing of the message being read takes it out of here. Marking it read does not: it is still
  // where it was. Weighed after the window has drawn the filing, which is when the list chooses
  // the next message, so the copy of the reader that leaves is the one that was on the screen.
  useEffect(() => {
    if (filed === null || filed === weighed.current) return;
    weighed.current = filed;
    if (filed.what === "read" || opened?.messageId !== filed.messageId) return;

    setLeaving({
      key: key.current,
      messageId: opened.messageId,
      folderId: opened.folderId,
      asked: answerAsked,
      askedOn: forwardAsked,
      askedAll: answerAllAsked,
      askedAway: throwAwayAsked,
    });
    key.current += 1;
    // Only a filing is weighed; what is open and what was asked are read as they stand then.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filed]);

  // Gone once it has slid out, at the moment its row in the list has.
  useEffect(() => {
    if (leaving === null) return;

    const gone = window.setTimeout(() => setLeaving(null), LEAVING_MS);
    return () => window.clearTimeout(gone);
  }, [leaving]);

  if (sentHere !== null) {
    return (
      <section className="flex min-h-0 flex-1 flex-col">
        <SentHereReader sent={sentHere} accountId={sentFrom} />
      </section>
    );
  }

  return (
    <section className="relative flex min-h-0 flex-1 flex-col overflow-hidden">
      {[
        // The one leaving, over the one underneath and out of reach: nothing in it is read aloud,
        // pressed or answered while it goes. Not drawn at all for somebody who asked for less
        // motion, who is shown what comes next at once.
        ...(leaving === null
          ? []
          : [
              <div
                key={leaving.key}
                aria-hidden="true"
                inert
                data-leaving=""
                style={{ animationDuration: `${LEAVING_MS}ms` }}
                className="absolute inset-0 z-10 flex flex-col bg-background animate-out fade-out-0 slide-out-to-right fill-mode-forwards motion-reduce:hidden rtl:slide-out-to-left"
              >
                <MessageReader
                  messageId={leaving.messageId}
                  folderId={leaving.folderId}
                  onReply={() => {}}
                  askedToAnswer={leaving.asked}
                  askedToAnswerAll={leaving.askedAll}
                  askedToForward={leaving.askedOn}
                  askedToThrowAway={leaving.askedAway}
                  onFiled={() => {}}
                  nothingToChoose={false}
                  showPictures={showPictures}
                />
              </div>,
            ]),
        <div key={key.current} className="flex min-h-0 flex-1 flex-col">
          <MessageReader
            // Nothing while the message leaving is still the one chosen: the list chooses the next
            // a moment later, and reading the one on its way out again, underneath itself, would
            // be work for a picture nobody sees.
            messageId={
              leaving !== null && leaving.messageId === opened?.messageId
                ? null
                : (opened?.messageId ?? null)
            }
            folderId={opened?.folderId ?? null}
            onReply={onReply}
            askedToAnswer={answerAsked}
            askedToAnswerAll={answerAllAsked}
            onForward={onForward}
            askedToForward={forwardAsked}
            askedToThrowAway={throwAwayAsked}
            onFiled={onFiled}
            nothingToChoose={nothingToChoose}
            showPictures={showPictures}
          />
        </div>,
      ]}
    </section>
  );
}
