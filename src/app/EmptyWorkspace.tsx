import { type Translator } from "@/shared/i18n";
import { Button } from "@/shared/ui/primitives/button";

/**
 * The first thing anybody sees, and the only thing there is to do.
 *
 * Its own file because it has two readers now: the shell renders it in place of everything when
 * nobody has connected a mailbox, and it is what the pane holds instead of a list and a reading
 * area that would both be empty.
 */
export function EmptyWorkspace({
  t,
  onConnect,
}: {
  t: Translator;
  onConnect: () => void;
}) {
  return (
    <div className="m-auto flex max-w-lg flex-col items-start gap-3">
      <h1 className="text-xl font-semibold tracking-tight">
        {t("workspace.connectTitle")}
      </h1>
      <p className="text-sm text-muted-foreground">{t("workspace.body")}</p>
      <Button onClick={onConnect}>{t("workspace.connectAction")}</Button>
    </div>
  );
}
