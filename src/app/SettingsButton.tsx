import { Settings } from "lucide-react";
import { useState } from "react";

import { useI18n, type Translator } from "@/shared/i18n";
import { Button } from "@/shared/ui/primitives/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/shared/ui/primitives/tooltip";

/**
 * The way into the settings, at the foot of the side column.
 *
 * Its hint opens under the pointer and never on the focus. The focus is given back to the gear by
 * hand when the settings close, and a hint that opened on it appeared over a screen the person had
 * just left and stayed there with no pointer to take it away. Under the pointer it is the hint
 * every icon of the folded sidebar has.
 *
 * When the button cannot be pressed, the hint says why instead: a disabled control takes no
 * pointer events, so the span beside it takes them, and the hint is the only way a person with a
 * mouse finds out why nothing happens.
 */
export function SettingsButton({
  storageIsShut,
  onOpen,
  t,
}: {
  /** Appearance and language live in the file that will not open, so there is nothing to show. */
  storageIsShut: boolean;
  onOpen: () => void;
  t: Translator;
}) {
  const { dir } = useI18n();
  const [pointedAt, setPointedAt] = useState(false);

  const gear = (
    <Button
      variant="ghost"
      size="icon-sm"
      // The reason is in the name rather than only in the hint: a hint on a disabled control is
      // reachable by a mouse and by nothing else, and this is the sentence that stops the button
      // looking broken.
      aria-label={
        storageIsShut ? t("settings.needsStorage") : t("settings.open")
      }
      // Said by the button being unavailable rather than by it doing nothing when pressed: a
      // control that answers a press with silence is a control that looks broken.
      disabled={storageIsShut}
      // Named so the focus can be given back to it. It leaves the page while the settings are
      // open, so the element that was pressed is not the element that comes back.
      data-opens-settings=""
      // Only ever opens. It is not on screen while the settings are, so it has no pressed state
      // to wear and nothing to toggle back to.
      onClick={onOpen}
    >
      <Settings />
    </Button>
  );

  if (!storageIsShut) {
    return (
      <Tooltip
        open={pointedAt}
        onOpenChange={(open) => {
          if (!open) setPointedAt(false);
        }}
      >
        <TooltipTrigger
          asChild
          onPointerEnter={() => setPointedAt(true)}
          onPointerLeave={() => setPointedAt(false)}
        >
          {gear}
        </TooltipTrigger>
        <TooltipContent side={dir === "rtl" ? "left" : "right"}>
          {t("settings.open")}
        </TooltipContent>
      </Tooltip>
    );
  }

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="inline-flex">{gear}</span>
      </TooltipTrigger>
      <TooltipContent>{t("settings.needsStorage")}</TooltipContent>
    </Tooltip>
  );
}
