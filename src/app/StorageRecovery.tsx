import { useEffect, useState } from "react";

import type { Failure } from "@/features/account-setup/useAccountSetup";
import { describeError, type Translator } from "@/shared/i18n";
import { secretStoreName, systemHere } from "@/shared/platform";
import {
  getSecretStoreState,
  restartAfterSettingAside,
  setAsideLocalStorage,
} from "@/shared/ipc/client";
import { Button } from "@/shared/ui/primitives/button";

/**
 * Reads the whole window again from the start.
 *
 * For the one moment the core's answer to everything changes at once: a device whose secrets are in
 * a file of its own opens its databases only when a person opens that file, and every list, setting
 * and count the window read before then was read from nothing. Kept beside the recovery screen
 * rather than in the window that calls it so that a test can put something in its place: a page
 * that reloads under the test runner ends the run.
 */
export function startTheWindowAgain(): void {
  window.location.reload();
}

/**
 * Shown when local storage could not be opened or migrated.
 *
 * The window must appear even then: crashing with no window is the worst possible first
 * user incident, and the user needs the path to be able to act.
 */
export function StorageRecovery({
  failure,
  t,
  onUnlock,
}: {
  failure: Failure;
  t: Translator;
  /** Brings the master password back, on a device whose secrets are in a file of its own. */
  onUnlock?: () => void;
}) {
  // Once the files are aside, this screen says one thing. Everything above it was advice about a
  // file that is no longer in the way, and leaving it there had a person reading two instructions
  // that contradicted each other.
  const [moved, setMoved] = useState<string[] | null>(null);
  const file = useFileOfSecrets();
  // Three failures arrive here. A key that could not be had leaves everything whole: setting the
  // files aside repairs nothing and costs the account settings and everything unsent, so the way
  // out that does that is not offered — except where the key may be gone for good, which `TheKey`
  // says. A keychain or Credential Manager that answers it holds no key is not this failure: the
  // core reports files that will not open, and that screen offers the way out.
  const theKey =
    failure.kind === "command" &&
    failure.error.code === "storageKeyUnavailable";
  // A file a newer build wrote is whole as well, and it is that build's to read. Setting it aside
  // would leave that build a fresh, empty profile to find in its place, so nothing here offers to
  // move it.
  const newerBuild =
    failure.kind === "command" &&
    failure.error.code === "storageFromANewerBuild";
  // What is wrong with the key is said once, by `TheKey`, and only once the answer about the store
  // is in. The path is shown wherever a file is named; beside a file of secrets still to be chosen
  // or opened it would read as that file.
  const path =
    failure.kind === "command" &&
    (!theKey || file === null || file === "open")
      ? failure.error.path
      : null;
  const reason = theKey
    ? null
    : failure.kind === "command"
      ? describeError(failure.error, t)
      : t("error.bridgeUnavailable");

  if (moved !== null) {
    return <SettingAsideDone t={t} where={moved} />;
  }

  return (
    <div
      className="m-auto flex max-w-lg flex-col items-start gap-3"
      role="alert"
    >
      <h1 className="text-xl font-semibold tracking-tight">
        {t("recovery.title")}
      </h1>
      {reason && <p className="text-sm text-muted-foreground">{reason}</p>}
      {path && (
        // A path reads left to right in every language.
        <p
          dir="ltr"
          className="w-full overflow-x-auto rounded-md border border-border bg-muted px-3 py-2 font-mono text-xs"
        >
          {path}
        </p>
      )}
      {theKey ? (
        <TheKey t={t} file={file} onUnlock={onUnlock} onMoved={setMoved} />
      ) : newerBuild ? (
        <p className="text-sm text-muted-foreground">
          {t("recovery.newerBuildBody")}
        </p>
      ) : (
        <>
          <p className="text-sm text-muted-foreground">{t("recovery.body")}</p>
          <StartFresh
            t={t}
            means={t("recovery.startFreshMeans")}
            onMoved={setMoved}
          />
        </>
      )}
    </div>
  );
}

/** What a device whose secrets are in a file of its own needs of that file. */
type FileOfSecrets = "choose" | "unlock" | "open";

/**
 * Whether this device keeps its secrets in a file of its own, and what the file needs: choosing,
 * opening, or nothing because it is open. `null` for a device with a store of its own, and
 * `undefined` while the answer is not in — so nothing is said about a store before it is known.
 */
function useFileOfSecrets(): FileOfSecrets | null | undefined {
  const [file, setFile] = useState<FileOfSecrets | null | undefined>(
    undefined,
  );

  useEffect(() => {
    let active = true;
    getSecretStoreState()
      .then((state) => {
        if (!active) return;
        setFile(
          state === "needsChoosing"
            ? "choose"
            : state === "needsUnlocking"
              ? "unlock"
              : state === "unlocked"
                ? "open"
                : null,
        );
      })
      .catch(() => {
        // A store that cannot be asked about is the system's, and the general sentences stand.
        if (active) setFile(null);
      });
    return () => {
      active = false;
    };
  }, []);

  return file;
}

/**
 * What is said when the key the databases are written under could not be had.
 *
 * Two kinds of device reach this. One with a store of its own was refused by it, and the way back is
 * that store's: on macOS and Windows a store that is always there, and on Linux a keyring that may
 * not be running, or be locked, or be gone for good — so starting it is said first, and Start fresh
 * beside it for the last. One whose secrets are in a file of its own keeps the key in that file,
 * closed until the master password is given, and there the way back is the password: offered here
 * as well as at the start, because somebody who put it off would otherwise be left reading about a
 * store their device does not have. And a file that is open with no key for these databases in it
 * is the only place this device would look for one, so Start fresh is offered with it — a screen
 * that only describes its own trouble is a screen nobody leaves.
 */
function TheKey({
  t,
  file,
  onUnlock,
  onMoved,
}: {
  t: Translator;
  file: FileOfSecrets | null | undefined;
  onUnlock?: () => void;
  onMoved: (where: string[]) => void;
}) {
  if (file === undefined) return null;

  if (file === null) {
    const store = t(secretStoreName());

    return systemHere() === "linux" ? (
      <>
        <p className="text-sm text-muted-foreground">
          {t("recovery.theKeyringBody", { store })}
        </p>
        <StartFresh
          t={t}
          means={t("recovery.startFreshMeansKeyring", { store })}
          onMoved={onMoved}
        />
      </>
    ) : (
      <p className="text-sm text-muted-foreground">
        {t("recovery.theKeyBody", { store })}
      </p>
    );
  }

  if (file === "open") {
    return (
      <>
        <p className="text-sm text-muted-foreground">
          {t("recovery.keyNotInTheFile")}
        </p>
        <StartFresh
          t={t}
          means={t("recovery.startFreshMeans")}
          onMoved={onMoved}
        />
      </>
    );
  }

  return (
    <>
      <p className="text-sm text-muted-foreground">
        {t(file === "choose" ? "secrets.chooseBody" : "secrets.unlockBody")}
      </p>
      {onUnlock && (
        <Button variant="outline" size="sm" onClick={onUnlock}>
          {t(file === "choose" ? "secrets.choose" : "secrets.unlock")}
        </Button>
      )}
    </>
  );
}

/** Everything the recovery screen says once the files have been set aside — which is one line. */
function SettingAsideDone({ t, where }: { t: Translator; where: string[] }) {
  return (
    <div
      className="m-auto flex max-w-lg flex-col items-start gap-3"
      role="status"
    >
      <h1 className="text-xl font-semibold tracking-tight">
        {t("recovery.movedAside")}
      </h1>
      <p className="text-sm text-muted-foreground">
        {t("recovery.restarting")}
      </p>
      {where.map((path) => (
        <p
          key={path}
          dir="ltr"
          className="w-full overflow-x-auto rounded-md border border-border bg-muted px-3 py-2 font-mono text-xs"
        >
          {path}
        </p>
      ))}
    </div>
  );
}

/**
 * The way out, from inside the window.
 *
 * Refusing to destroy what somebody wrote is right. Leaving them with a notice about it and no
 * button is a different thing and a worse one: the application becomes a description of its own
 * trouble. So the files are set aside — moved, never removed, every byte kept under a name with
 * the moment beside it — and where they went is said, because somebody told their mail was put
 * somewhere and not told where has been told nothing. Files a newer build wrote are not trouble,
 * and this is not offered for them: the way out there is that build.
 */
function StartFresh({
  t,
  means,
  onMoved,
}: {
  t: Translator;
  /** What pressing it costs and what follows, said before it is pressed. */
  means: string;
  onMoved: (where: string[]) => void;
}) {
  const [state, setState] = useState<
    | { status: "asking" }
    | { status: "moving" }
    | { status: "moved"; where: string[] }
    | { status: "failed" }
  >({ status: "asking" });

  return (
    <div className="flex flex-col items-start gap-2">
      <Button
        variant="outline"
        size="sm"
        disabled={state.status === "moving"}
        onClick={() => {
          setState({ status: "moving" });
          setAsideLocalStorage()
            .then((done) => {
              onMoved(done.moved);
              // Asked for after the window has been told, so a restart that cannot happen leaves
              // somebody reading where their files went rather than a screen that never changed.
              void restartAfterSettingAside();
            })
            .catch(() => setState({ status: "failed" }));
        }}
      >
        {t("recovery.startFresh")}
      </Button>
      <p className="text-sm text-muted-foreground">{means}</p>
      {state.status === "failed" && (
        <p className="text-sm text-muted-foreground" role="alert">
          {t("recovery.couldNotMove")}
        </p>
      )}
    </div>
  );
}
