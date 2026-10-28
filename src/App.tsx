import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ComponentProps,
} from "react";

import {
  isPlainKey,
  mailChord,
  opensSettings,
  somebodyIsTyping,
  type MailChord,
} from "@/shared/ui/keys";

import { leaveScreen } from "@/app/leave-screen";
import { applyNativeFeel, markWindowControls } from "@/app/native-feel";
import { applyTheme } from "@/app/theme";
import { NOTHING_TO_ANSWER, type Answer } from "@/entities/message";
import {
  isSameMailbox,
  leftAtKey,
  mailboxKey,
  returningTo,
  scopeKey,
  scopeOf,
  type LeftAt,
  type Mailbox,
} from "@/entities/mailbox";
import type { FolderIdDto } from "@/shared/ipc/generated/FolderIdDto";
import type { SentHereDto } from "@/shared/ipc/generated/SentHereDto";
import type { ViewScopeDto } from "@/shared/ipc/generated/ViewScopeDto";
import { AccountSetup } from "@/features/account-setup/AccountSetup";
import { ReadingPane } from "@/app/ReadingPane";
import { ScreenPane } from "@/app/ScreenPane";
import { Sidebar } from "@/app/Sidebar";
import { StorageRecovery, startTheWindowAgain } from "@/app/StorageRecovery";
import { Keys } from "@/app/Keys";
import { EmptyWorkspace } from "@/app/EmptyWorkspace";
import { useReadChanges } from "@/features/mailbox-tree/useReadChanges";
import { WhatChanged } from "@/features/updates/WhatChanged";
import type { WhatChangedDto } from "@/shared/ipc/generated/WhatChangedDto";
import { FilingNotice, type Filed } from "@/features/message-list/FilingNotice";
import { Composer } from "@/features/composer/Composer";
import { DraftsInFolder } from "@/features/message-list/DraftsInFolder";
import { SentInFolder } from "@/features/message-list/SentInFolder";
import { EmptyTrash } from "@/features/message-list/EmptyTrash";
import { MessageList } from "@/features/message-list/MessageList";
import { SearchBox } from "@/features/message-list/SearchBox";
import { SettingsScreen } from "@/features/settings/SettingsScreen";
import { MasterPasswordGate } from "@/features/secret-store/MasterPasswordGate";
import { Onboarding } from "@/features/onboarding/Onboarding";
import { ResizableColumn } from "@/shared/ui/ResizableColumn";
import { useI18n } from "@/shared/i18n";
import {
  getPaneWidths,
  changesSeen,
  getSecretStoreState,
  whatChanged,
  onMailAsked,
  onMailChanged,
  onOpenSettings,
  onTheKeyArrived,
  getPreferences,
  getWorkspaceOverview,
  isCommandError,
  openingMessage,
  setPaneWidth,
} from "@/shared/ipc/client";
import type { Failure } from "@/features/account-setup/useAccountSetup";
import type { PreferencesDto } from "@/shared/ipc/generated/PreferencesDto";
import type { SecretStoreStateDto } from "@/shared/ipc/generated/SecretStoreStateDto";
import type { WorkspaceOverviewDto } from "@/shared/ipc/generated/WorkspaceOverviewDto";

/** A failure the core reported, or the bridge itself not answering — two different sentences. */

type State =
  | { status: "loading" }
  | { status: "ready"; workspace: WorkspaceOverviewDto }
  | { status: "failed"; failure: Failure };

/** A screen that stands over the list and the message being read. */
type Screen = "writing" | "settings" | "account";

function App() {
  const [state, setState] = useState<State>({ status: "loading" });
  /** Bumped after seeding, to make the folders and the list read the index again. */
  const [listGeneration, setListGeneration] = useState(0);
  /**
   * Bumped when a person's change is written down, to make the counts beside the folders read
   * again. The list already shows the change.
   */
  const [countsGeneration, setCountsGeneration] = useState(0);
  /** Where the person left each mailbox this session, by the mailbox. */
  const places = useRef(new Map<string, LeftAt>());
  /** Read and unread changes the counts beside the folders may not have been read since. */
  const [readChanges, marking] = useReadChanges();
  /// What somebody typed into the search box, as they typed it. Empty means they are not
  /// searching, and the list shows the mailbox they picked.
  const [lookingFor, setLookingFor] = useState("");
  const [mailbox, setMailbox] = useState<Mailbox | null>(null);
  /// What the list is showing: what is being looked for while somebody is looking, and the
  /// mailbox they picked the rest of the time.
  const shown: ViewScopeDto =
    lookingFor.trim() !== ""
      ? { kind: "looking", typed: lookingFor }
      : mailbox
        ? scopeOf(mailbox)
        : { kind: "everything" };
  const [openedMessage, setOpenedMessage] = useState<{
    messageId: string;
    folderId: FolderIdDto | null;
  } | null>(null);
  /**
   * A message sent from this device that the reader shows instead, before its account has been
   * read back with it. Nothing while the reader shows mail, which is most of the time.
   */
  const [openedHere, setOpenedHere] = useState<SentHereDto | null>(null);
  /** A message chosen in a list of mail is the one the reader shows, whatever it showed before. */
  const openFromList = useCallback(
    (opened: { messageId: string; folderId: FolderIdDto | null } | null) => {
      setOpenedMessage(opened);
      if (opened !== null) setOpenedHere(null);
    },
    [],
  );
  /**
   * What is being written, when something is.
   *
   * `null` is nobody writing. Anything else is the composer open, and what it holds says whether
   * this is a new message or an answer to the one that was being read.
   */
  const [writing, setWriting] = useState<Answer | null>(null);
  /**
   * The message that has just been filed away, and what became of it.
   *
   * Two panes, one action: the buttons are on the message being read, the row is in the list, and
   * the line saying what happened belongs under the list rather than on a message nobody is
   * looking at any more. This is what passes between all three.
   */
  const [filed, setFiled] = useState<Filed | null>(null);
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
  }, []);

  const putBack = useCallback((messageId: string) => {
    setBack(messageId);
    // Read again as well as shown again: once the server has carried the filing out and a pass has
    // read it back, the list no longer holds the row it would show, and only its own answer --
    // the way back laid over the index -- brings the message back into it.
    setListGeneration((generation) => generation + 1);
  }, []);
  /**
   * Forgotten once the notice about it has gone from the screen.
   *
   * What this holds is "there is a notice showing". Never cleared, it stayed true for the rest of
   * the session after the first message anybody filed -- and anything else that wanted the same
   * corner, like the offer to empty the trash, was never seen again.
   */
  const forgetWhatWasFiled = useCallback(() => setFiled(null), []);
  const [addingAccount, setAddingAccount] = useState(false);
  /** An account already here whose password has to be typed again. */
  const [reconnecting, setReconnecting] = useState<string | undefined>(
    undefined,
  );
  /**
   * Which screen of adding an account is showing.
   *
   * Here rather than in the form, because the way back is the arrow in the pane's corner: the
   * same corner as the cross, on every screen, so a person looks in one place for both.
   */
  const [accountStep, setAccountStep] = useState<"start" | "settings">("start");
  /** An account just connected whose inbox is to be opened once the sidebar knows its folders. */
  const [openInboxOf, setOpenInboxOf] = useState<string | null>(null);
  /** How many the open list holds, or `null` while it does not know yet. */
  const [holds, setHolds] = useState<number | null>(null);

  /**
   * The account whose drafts are being looked at, when the drafts folder is the one open.
   *
   * Nothing the rest of the time. Not while something is being searched for either: a search is
   * over mail, and a draft is not mail yet.
   */
  const showingDrafts =
    lookingFor.trim() === "" &&
    mailbox?.kind === "folder" &&
    mailbox.role === "drafts"
      ? mailbox.accountId
      : null;
  /** The account whose sent mail is being looked at, the same way, for the folder for sent mail. */
  const showingSent =
    lookingFor.trim() === "" &&
    mailbox?.kind === "folder" &&
    mailbox.role === "sent"
      ? mailbox.accountId
      : null;
  /**
   * Everything a person can decide about how this looks and reads.
   *
   * Held whole rather than field by field: every write answers with the whole of the settings as
   * they were stored, so the window wears what is on disk and cannot drift from it. `null` until
   * the first read answers, which is when the defaults already in the stylesheet are correct.
   */
  const [preferences, setPreferences] = useState<PreferencesDto | null>(null);
  /** Whether the settings screen is the thing being shown. */
  const [inSettings, setInSettings] = useState(false);
  /**
   * What changed in the version now running, where this copy updated itself and nobody has read it.
   *
   * Asked once, on the start after an update and never otherwise. It is not a screen somebody
   * navigates to: it arrives because the application is not the one they were using yesterday.
   */
  const [changed, setChanged] = useState<WhatChangedDto | null>(null);
  /** The last screen asked for, which is the one in front for as long as it stays open. */
  const [inFront, setInFront] = useState<Screen | null>(null);
  /** Column widths, as the reader left them. Defaults until the stored ones arrive. */
  const [columns, setColumns] = useState({ sidebar: 240, list: 416 });
  /**
   * Where this device keeps its secrets. `system` where they are in the system's own store; the
   * other answers mean there is a file to choose a master password for, or to open.
   */
  const [secrets, setSecrets] = useState<SecretStoreStateDto | null>(null);
  /** Whether the master password is being asked for right now. */
  const [askingForMaster, setAskingForMaster] = useState(false);
  /** Whether the onboarding may still be shown in this run: until it is finished or gone on from. */
  const [welcoming, setWelcoming] = useState(true);

  // Asked once, on the start after this copy updated itself. On every other start it answers
  // nothing, which is what it does for the whole life of a build nobody updated.
  useEffect(() => {
    let active = true;

    whatChanged()
      .then((found) => {
        if (active) setChanged(found);
      })
      .catch(() => {
        /* A version that cannot say what changed still runs. */
      });

    return () => {
      active = false;
    };
  }, []);

  // Asked once, at the start: a device with no credential store has to be told about it before it
  // is asked for a mailbox password that would have nowhere to go.
  useEffect(() => {
    let active = true;
    getSecretStoreState()
      .then((state) => {
        if (!active) return;
        setSecrets(state);
        setAskingForMaster(
          state === "needsChoosing" || state === "needsUnlocking",
        );
      })
      .catch(() => {
        /* A device whose store cannot even be asked about is one with a system store: the
           commands answer `system` everywhere else, and a failure here must not put a password
           prompt in front of someone who has a keychain. */
      });
    return () => {
      active = false;
    };
  }, []);

  /** Whether a secret could be stored at all right now. */
  const secretsClosed =
    secrets === "needsChoosing" || secrets === "needsUnlocking";

  /** Opening the account form, unless there is nowhere to put the password it would ask for. */
  function connectAccount(accountId?: string) {
    if (secretsClosed) {
      setAskingForMaster(true);
      return;
    }
    setReconnecting(accountId);
    // An account already here has answered the first screen's three questions long ago; what is
    // missing is its password, and that is on the long one.
    setAccountStep(accountId ? "settings" : "start");
    setAddingAccount(true);
    setInFront("account");
  }

  useEffect(() => {
    let active = true;
    getPaneWidths()
      .then((stored) => {
        if (!active) return;
        setColumns((current) => ({
          sidebar: stored.sidebar ?? current.sidebar,
          list: stored.list ?? current.list,
        }));
      })
      .catch(() => {
        /* The defaults are already on screen; a layout is not worth an error message. */
      });
    return () => {
      active = false;
    };
  }, []);

  // A webview brings the browser's habits with it; an application does not want them.
  useEffect(() => {
    markWindowControls();

    return applyNativeFeel();
  }, []);

  // The core speaks when a pass changed something, and when a person's change has been written
  // down; hearing either is reason enough to read again. The first is read into the tree and the
  // list, the second into the counts alone: the list already shows what the person did.
  useEffect(() => {
    const stops: (() => void)[] = [];
    let active = true;
    const keep = (unlisten: () => void) => {
      if (active) stops.push(unlisten);
      else unlisten();
    };

    onMailChanged(() => setListGeneration((generation) => generation + 1))
      .then(keep)
      .catch(() => {
        /* Without the bridge nothing arrives on its own; the refresh button still works. */
      });
    onMailAsked(() => setCountsGeneration((generation) => generation + 1))
      .then(keep)
      .catch(() => {});
    // The key the recovery screen says could not be had, handed over by somebody answering the
    // system late: the databases are open now, and nothing on screen was read from them.
    onTheKeyArrived(() => startTheWindowAgain())
      .then(keep)
      .catch(() => {});

    return () => {
      active = false;
      for (const stop of stops) stop();
    };
  }, []);

  const resize = (pane: "sidebar" | "list") => (width: number) => {
    setColumns((current) => ({ ...current, [pane]: width }));
    setPaneWidth(pane, width).catch(() => {
      /* It stays this way until the app is closed; saying so would interrupt a drag. */
    });
  };
  const { t, setPreference } = useI18n();

  // Preferences are applied independently of the workspace: a failure to read the mail index
  // must not leave the user staring at the wrong theme or the wrong language on the recovery screen.
  useEffect(() => {
    getPreferences()
      .then(setPreferences)
      .catch(() => {
        /* Defaults already applied: the stylesheet for the theme, the OS locale for the language. */
      });
  }, []);

  // One place where settings become the window, so a change made on the settings screen and one
  // read at startup take exactly the same path. Before the window is painted: the colours change in
  // the same frame as the settings screen offering them, rather than one frame after it.
  useLayoutEffect(() => {
    if (preferences === null) return;

    setPreference(preferences.language);
    return applyTheme(
      preferences.theme,
      preferences.darkScheme,
      preferences.lightScheme,
    );
  }, [preferences, setPreference]);

  useEffect(() => {
    let active = true;
    // With the message the window opens on, if there is one, so the list holds it from its first
    // frame: the list reads what is open once, as it appears. There is one only in the build made
    // for screenshots, and a window that cannot ask opens on nothing, as every other build does.
    Promise.all([getWorkspaceOverview(), openingMessage().catch(() => null)])
      .then(([workspace, opening]) => {
        if (!active) return;
        if (opening !== null) openFromList(opening);
        setState({ status: "ready", workspace });
      })
      .catch((reason: unknown) => {
        if (!active) return;
        setState({
          status: "failed",
          failure: isCommandError(reason)
            ? { kind: "command", error: reason }
            : { kind: "bridge" },
        });
      });
    return () => {
      active = false;
    };
  }, [openFromList]);

  /** Whether the panel asking for the master password is covering the window. */
  const asking = askingForMaster && secretsClosed;
  /**
   * Whether the onboarding is covering the window, which is not seen until it is finished.
   *
   * For somebody who has never finished it, which the settings say. From the first frame, while
   * they are still being read, so the window is not glimpsed before it — and for somebody who has
   * finished it, that is the moment it takes to read them. Never over the trouble a failed start
   * explains, and never beside the master password, which comes first. Settings that could not be
   * read at all leave nothing to greet with, and the window is shown.
   */
  const greeting =
    welcoming &&
    !asking &&
    state.status !== "failed" &&
    (preferences === null
      ? state.status === "loading"
      : !preferences.welcomed);
  /** Whether anything covers the window, so the window behind it neither listens nor is reached. */
  const covered = asking || greeting;

  /** Bumped when somebody asks for an answer by key; the reader is what knows how to make one. */
  const [answerAsked, setAnswerAsked] = useState(0);
  /** The same, for an answer to everybody on it. */
  const [answerAllAsked, setAnswerAllAsked] = useState(0);
  /** The same, for passing the message being read on. */
  const [forwardAsked, setForwardAsked] = useState(0);
  /** The same, for throwing the message being read away. */
  const [throwAwayAsked, setThrowAwayAsked] = useState(0);
  const [showingKeys, setShowingKeys] = useState(false);

  /**
   * Nobody has connected a mailbox yet.
   *
   * Not a state to decorate but a state to take things away for. A search box over nothing to
   * search, a reading pane beside nothing to read and a compose button on an account that does not
   * exist are three controls that answer nothing — and a first screen made mostly of those reads
   * as an application that is already broken.
   */
  const empty =
    state.status === "ready" && state.workspace.accounts.length === 0;

  /**
   * The screens that are open, and which of them is in front.
   *
   * They stack rather than replace one another. The sidebar stays beside every screen, so the
   * gear and the plus can be pressed while a message is half written — and before this the branch
   * that happened to come first simply won the slot: the gear did nothing, and then the settings
   * appeared by themselves once the message was put away. Opening one over another leaves the one
   * underneath as it was, and closing the front one goes back to it.
   */
  const open: Screen[] = [];
  if (writing !== null && state.status === "ready") open.push("writing");
  if (inSettings && preferences !== null) open.push("settings");
  if (addingAccount && state.status !== "failed") open.push("account");
  const showing: Screen | null =
    inFront !== null && open.includes(inFront)
      ? inFront
      : (open[open.length - 1] ?? null);

  const screen = useRef<HTMLElement>(null);
  /**
   * Filled by the composer with its own way out, which writes the draft down before it goes, and
   * which goes on to what it is handed once the window has gone.
   */
  const leaveWriting = useRef<((then?: () => void) => void) | null>(null);
  /** The same, for the account form, whose way out is refused while it is working. */
  const leaveAccount = useRef<(() => void) | null>(null);

  /**
   * What leaves the screen in front, or nothing where there is none to leave.
   *
   * The very callbacks the cross is given, so the key and the cross cost the same: the composer
   * writes down what was typed and refuses while a message is going out, and the account form
   * refuses while it is working. Neither refusal is repeated here — a screen that will not be
   * left says so from inside its own way out.
   */
  const wayOutOf = (screen: Screen | null): (() => void) | null => {
    switch (screen) {
      case "writing":
        return leaveWriting.current;
      case "account":
        return leaveAccount.current;
      case "settings":
        return () => setInSettings(false);
      case null:
        return null;
    }
  };

  /**
   * What is being written, as the window last drew it, for the compose button and its key to weigh
   * what they ask for against.
   *
   * Published before the paint, as the composer publishes its way out, so the two always describe
   * the same composer. The window's keys are installed again only by an ordinary effect, after the
   * paint, and a key that lands in between reaches the listener installed before. Read from that
   * listener's render, `c` pressed as a new message replaces a draft would weigh it against the
   * draft, take the new message's own way out to open the message already open, and leave that
   * window on its way out for good, with every field and every way out refused.
   */
  const writingNow = useRef(writing);
  useLayoutEffect(() => {
    writingNow.current = writing;
  }, [writing]);

  /**
   * Opens the composer on `answer`, for the compose button and its key.
   *
   * Over a message being written that opened under another key — another message answered, or
   * another To, Cc or subject — the one open leaves first, by its own way out, the very one the
   * cross takes, and `answer` opens in its place once it has gone. Put in its place directly, the
   * composer would go with none of what leaving does: nothing asked, the last second of typing
   * never written down, and anything the drafts would not take gone without a word. Where leaving
   * is refused — a message going out, the question put away, a keep that could not be made —
   * nothing opens, and what is being written stays as it was.
   *
   * Over a message that opened under the same key, nothing leaves and nothing opens, whatever else
   * it holds: a new message asked for over a draft with no To, Cc or subject that answers nothing
   * leaves that draft where it is.
   */
  function startWriting(answer: Answer) {
    setInFront("writing");

    const now = writingNow.current;
    const wayOut = leaveWriting.current;
    if (
      now === null ||
      wayOut === null ||
      openingKey(now) === openingKey(answer)
    ) {
      setWriting(answer);
      return;
    }

    leaveScreen(() => wayOut(() => setWriting(answer)));
  }

  // The window's own keys. One preset, plain letters, and inert wherever somebody is typing —
  // otherwise `c` in a subject line would throw away the message being written and open another.
  //
  // Not while a panel covers the window: it says it is the only thing there, and a shortcut
  // acting behind it would be the window doing something nobody can see.
  useEffect(() => {
    if (covered) return;

    /** What a key asks for, by letter or by chord: the same either way. */
    function ask(what: MailChord) {
      switch (what) {
        case "compose":
          startWriting(NOTHING_TO_ANSWER);
          return;
        case "answer":
          setAnswerAsked((asked) => asked + 1);
          return;
        case "answerAll":
          setAnswerAllAsked((asked) => asked + 1);
          return;
        case "forward":
          setForwardAsked((asked) => asked + 1);
          return;
        case "throwAway":
          setThrowAwayAsked((asked) => asked + 1);
          return;
      }
    }

    function onKey(event: KeyboardEvent) {
      // The one shortcut that is not a plain letter, and it is the one every application on this
      // desk has. It works while somebody is typing, because on every system it is the settings
      // and never a comma: a person who reaches for it in a subject line means the settings.
      if (opensSettings(event)) {
        event.preventDefault();
        setInSettings(true);
        setInFront("settings");
        return;
      }

      // Leaving what is open, and the only shortcut here that answers while a field has the
      // focus: a screen is left from inside it, and a person filling in the eleventh field should
      // not have to find something else to click first. Anything a field wants Escape for -- a
      // name being reverted, a question being dismissed -- is that field's to take first, which
      // it does by stopping the key before it reaches this.
      //
      // Not during an input method's own composition: there Escape cancels the half-typed string
      // and belongs to the person's keyboard, not to this window. `keyCode` 229 is what the
      // engines that predate the flag report.
      if (
        event.key === "Escape" &&
        isPlainKey(event) &&
        !event.isComposing &&
        event.keyCode !== 229
      ) {
        const wayOut = wayOutOf(showing);
        if (wayOut === null) return;

        event.preventDefault();
        leaveScreen(wayOut);
        return;
      }

      // The chords every mail application answers to, beside the letters. Not where somebody types,
      // where they edit the text instead, and not when the list already answered the same key by
      // throwing its selection away.
      const chord = mailChord(event);
      if (chord !== null) {
        if (somebodyIsTyping(event.target) || event.defaultPrevented) return;
        event.preventDefault();
        ask(chord);
        return;
      }

      if (!isPlainKey(event) || somebodyIsTyping(event.target)) return;

      switch (event.key) {
        case "c":
          event.preventDefault();
          ask("compose");
          return;
        case "r":
          event.preventDefault();
          ask("answer");
          return;
        case "a":
          event.preventDefault();
          ask("answerAll");
          return;
        case "f":
          event.preventDefault();
          ask("forward");
          return;
        case "/":
          event.preventDefault();
          document.querySelector<HTMLInputElement>("[data-search]")?.focus();
          return;
        case "?":
          event.preventDefault();
          setShowingKeys(true);
          return;
        default:
      }
    }

    window.addEventListener("keydown", onKey);
    // The application menu's settings item, which on macOS takes the key before this window
    // hears it: the same door, held shut by the same panel.
    let stopListening: (() => void) | null = null;
    let listening = true;
    void onOpenSettings(() => {
      setInSettings(true);
      setInFront("settings");
    })
      .then((stop) => {
        if (listening) stopListening = stop;
        else stop();
      })
      .catch(() => {
        /* The menu cannot reach this window; the keys still open the settings. */
      });
    return () => {
      window.removeEventListener("keydown", onKey);
      listening = false;
      stopListening?.();
    };
    // `showing` is in here because Escape has to leave whichever screen is in front now, and not
    // the one that was when this was last installed. What `c` weighs is not read from here:
    // `writingNow` says why.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [covered, showing]);
  /** What had the focus when the screen opened, so it can have it back. */
  const cameFrom = useRef<HTMLElement | null>(null);

  /** Which screen the focus was last put into, so a second one opening is noticed. */
  const focused = useRef<Screen | null>(null);

  // Focus follows the pane. Swapping what fills it left focus on a button that no longer exists,
  // so it fell to the body: the new screen was never announced, and Tab started again from the
  // top of the window rather than continuing inside what had just opened. Coming back puts it
  // where it was, which is usually the row somebody was standing on.
  useEffect(() => {
    const before = focused.current;
    focused.current = showing;

    if (showing !== null) {
      // Only on the way in from the window itself. A screen opened over another comes back to
      // that one, and the row behind them both is still where the last of them will return to.
      if (before === null) {
        const had = document.activeElement;
        // The body is not somewhere to come back to. Recorded, it would make closing the screen a
        // focus that goes nowhere and reads as the window starting over.
        cameFrom.current =
          had instanceof HTMLElement && had !== document.body ? had : null;
      }
      screen.current?.focus();
      return;
    }

    const back = cameFrom.current;
    cameFrom.current = null;
    // Only if it is still on the page. A row that was filed while the screen was open is gone,
    // and focusing it would throw the focus away a second time.
    if (back?.isConnected) {
      back.focus();
      return;
    }

    // The gear is not drawn beside the settings it opens, so the control that opened this screen
    // was off the page for as long as the screen was on it. It is back now and it is the same
    // control, even though it is not the same element -- and without this the focus would fall to
    // the body, which reads as the window starting over.
    if (before === "settings") {
      document.querySelector<HTMLElement>("[data-opens-settings]")?.focus();
    }
  }, [showing]);

  const placeKey = leftAtKey(shown);
  const left = placeKey === null ? undefined : places.current.get(placeKey);
  /** What the list of mail is given, whether it stands alone or under the drafts kept here. */
  const folderList = {
    generation: listGeneration,
    scope: shown,
    density: preferences?.density ?? "comfortable",
    onOpen: openFromList,
    // The message the reader shows, which the list finds and marks as chosen without opening it
    // again; and the row that was at the top of this mailbox when it was left.
    returnTo: returningTo(left, openedMessage),
    onPlace: (place: LeftAt) => {
      if (placeKey !== null) places.current.set(placeKey, place);
    },
    onMarking: marking,
    onFiled: filedAway,
    onHolds: setHolds,
    filed:
      unreadAgain !== null
        ? { ...unreadAgain, state: "unread" as const }
        : back !== null
          ? { messageId: back, state: "back" as const }
          : // A message marked read has not gone anywhere. Telling the list it had would slide out
            // the row somebody is reading.
            filed === null || filed.what === "read"
            ? null
            : { messageId: filed.messageId, state: "gone" as const },
    // Only when there is nothing to read anywhere. An empty folder is an ordinary thing that says
    // so itself; putting the welcome there instead offered to connect an account to somebody who
    // has one, in a folder they had just emptied.
    emptyState:
      shown.kind === "looking" ? (
        <div className="space-y-1 text-center">
          <p>{t("search.nothing")}</p>
          {/* Said here rather than nowhere: mail whose text is not on this device cannot be
              searched by its text, and a person looking at an empty list deserves to know which
              of the two happened. */}
          <p className="text-xs text-muted-foreground">
            {t("search.onlyWhatIsHere")}
          </p>
        </div>
      ) : undefined,
  } satisfies ComponentProps<typeof MessageList>;

  return (
    <>
      {showingKeys && <Keys onClose={() => setShowingKeys(false)} t={t} />}

      {asking && (
        <MasterPasswordGate
          mode={secrets === "needsChoosing" ? "choose" : "unlock"}
          onOpened={() => {
            if (state.status !== "ready") return startTheWindowAgain();
            setSecrets("unlocked");
            setAskingForMaster(false);
          }}
          onLater={() => setAskingForMaster(false)}
        />
      )}

      {greeting && (
        <Onboarding
          preferences={preferences}
          onChanged={setPreferences}
          onDone={() => setWelcoming(false)}
        />
      )}

      {/* Out of reach while the panel is up, which is what the panel already claims of itself.
          Without this the shell behind it kept every one of its tab stops: a person pressing Tab
          in a panel that covers the window walked out of it into a sidebar they cannot see, with
          no way of knowing they had left. The panel is rendered outside this rather than inside
          it, because a panel inside what it silences would silence itself. */}
      <main
        inert={covered}
        className="window-panes flex h-full overflow-hidden bg-background text-foreground"
      >
        <Sidebar
          width={columns.sidebar}
          onResize={resize("sidebar")}
          settingsInFront={showing === "settings"}
          empty={empty}
          ready={state.status === "ready"}
          onCompose={() => startWriting(NOTHING_TO_ANSWER)}
          tree={{
            generation: listGeneration + countsGeneration,
            selected: mailbox,
            readChanges,
            onSelect: (next) => {
              setMailbox(next);
              // The mailbox already chosen, chosen again, leaves the reader as it is: what it
              // shows may be a search's answer the list is still showing.
              if (mailbox !== null && isSameMailbox(mailbox, next)) return;
              // The first, which the sidebar chooses itself as the window starts. Nobody left
              // anything behind, and the message the window opened on, if it opened on one, stays.
              if (mailbox === null) return;

              // Back to what was open there, or to nothing: a mailbox is somewhere to read, and
              // arriving in it chooses nothing.
              const there = places.current.get(mailboxKey(next));
              setOpenedMessage(
                there?.messageId
                  ? { messageId: there.messageId, folderId: there.folderId }
                  : null,
              );
              setOpenedHere(null);
            },
            onAddAccount: () => connectAccount(),
            onReconnect: (accountId) => connectAccount(accountId),
            openInboxOf,
            onInboxOpened: () => setOpenInboxOf(null),
            recovering: state.status === "failed" ? state.failure : null,
          }}
          preferences={preferences}
          onOpenSettings={() => {
            setInSettings(true);
            setInFront("settings");
          }}
        />

        {writing !== null && state.status === "ready" && (
          <ScreenPane
            key="writing"
            inFront={showing === "writing"}
            paneRef={showing === "writing" ? screen : { current: null }}
            closeLabel={t("shell.closeScreen")}
            // Not `setWriting(null)`: leaving here writes down what was typed in the last second
            // first, and refuses while the message is going out. The composer holds that, and this
            // is where it puts it.
            onClose={() => leaveWriting.current?.()}
          >
            <Composer
              leaveRef={leaveWriting}
              // What the composer starts with is read once, when it appears. The compose button in
              // the sidebar stays clickable while it is open, so without this, asking for a new
              // message in the middle of an answer would leave the answer's address and subject in
              // the fields. Two openings that would fill them identically share a key, and then
              // there is nothing to remount for. Cc is in it because answering one person and
              // answering the room can address the same person and differ only in who else hears.
              key={openingKey(writing)}
              accounts={state.workspace.accounts}
              answer={writing}
              onCancel={() => {
                setWriting(null);
                // Leaving may have left a draft behind, or taken one away. Either way the count
                // beside Drafts is now describing what was there a moment ago.
                setListGeneration((generation) => generation + 1);
              }}
              // Counted beside Drafts from the moment it is kept, rather than from whenever the
              // sidebar next happened to be read.
              onKept={() => setListGeneration((generation) => generation + 1)}
              onWritten={() => {
                setWriting(null);
                // The message is in the outbox, which the folder for sent mail shows from now on,
                // and a push is on its way to hand it over.
                setListGeneration((generation) => generation + 1);
              }}
            />
          </ScreenPane>
        )}

        {changed !== null && (
          <ScreenPane
            key="what-changed"
            inFront={true}
            paneRef={{ current: null }}
            closeLabel={t("update.changesClose")}
            onClose={() => {
              // Marked read on the way out rather than on the way in: somebody interrupted before
              // reading a word gets it again, and somebody who closed it meant to close it.
              changesSeen().catch(() => {
                /* Shown once more next time, which is better than never. */
              });
              setChanged(null);
            }}
          >
            <WhatChanged changed={changed} />
          </ScreenPane>
        )}

        {inSettings && preferences !== null && (
          <ScreenPane
            key="settings"
            inFront={showing === "settings"}
            paneRef={showing === "settings" ? screen : { current: null }}
            closeLabel={t("shell.closeScreen")}
            onClose={() => setInSettings(false)}
          >
            <SettingsScreen
              preferences={preferences}
              onChanged={setPreferences}
              accounts={
                state.status === "ready" ? state.workspace.accounts : []
              }
              onOpenAccount={(accountId) => {
                setInSettings(false);
                connectAccount(accountId);
              }}
              // The sidebar is drawn beside this screen rather than under it, so an arrangement
              // changed here has to reach it — otherwise a group made in the settings appears only
              // after a restart.
              onArrangementChanged={() =>
                setListGeneration((generation) => generation + 1)
              }
            />
          </ScreenPane>
        )}

        {addingAccount && state.status !== "failed" && (
          <ScreenPane
            key="account"
            inFront={showing === "account"}
            paneRef={showing === "account" ? screen : { current: null }}
            closeLabel={t("shell.closeScreen")}
            onClose={() => leaveAccount.current?.()}
            // Only where there is a screen behind: an account being reconnected starts on the
            // long form, and an arrow back from it would go nowhere.
            onBack={
              accountStep === "settings" && reconnecting === undefined
                ? () => setAccountStep("start")
                : null
            }
            backLabel={t("account.back")}
          >
            <AccountSetup
              // A form of its own for each account. Turned to another account while it stands open,
              // one form would carry the first account's typed password into the second's fields.
              key={reconnecting ?? ""}
              leaveRef={leaveAccount}
              step={accountStep}
              onStep={setAccountStep}
              reconnecting={reconnecting}
              secretsInAFile={secrets !== null && secrets !== "system"}
              address={
                state.status === "ready"
                  ? state.workspace.accounts.find(
                      (account) => account.id === reconnecting,
                    )?.email
                  : undefined
              }
              onCancel={() => setAddingAccount(false)}
              onAdded={(account) => {
                setAddingAccount(false);
                // The new account brings folders with it, so both the sidebar and the list are read
                // again rather than left showing the state from before it existed.
                // Replaced where it is already here, appended where it is not. The same screen
                // does both now, and appending an account that was only corrected would put it in
                // the sidebar twice.
                setState((current) =>
                  current.status === "ready"
                    ? {
                        status: "ready",
                        workspace: {
                          ...current.workspace,
                          accounts: current.workspace.accounts.some(
                            (one) => one.id === account.id,
                          )
                            ? current.workspace.accounts.map((one) =>
                                one.id === account.id ? account : one,
                              )
                            : [...current.workspace.accounts, account],
                        },
                      }
                    : current,
                );
                setListGeneration((generation) => generation + 1);
                // And open its inbox. A mailbox that was just connected and shows nothing is a
                // person wondering whether it worked; the folder they came here for is the one
                // to land in. Asked for by account rather than by folder, because which folder
                // is its inbox is answered by the sidebar when it reads itself again.
                setOpenInboxOf(account.id);
              }}
              onRemoved={(accountId) => {
                setAddingAccount(false);
                // Taken out of the sidebar here rather than left until the next start: an account
                // that is gone from the disk and still in the list is a row that answers nothing
                // when it is clicked.
                setState((current) =>
                  current.status === "ready"
                    ? {
                        status: "ready",
                        workspace: {
                          ...current.workspace,
                          accounts: current.workspace.accounts.filter(
                            (account) => account.id !== accountId,
                          ),
                        },
                      }
                    : current,
                );
                setListGeneration((generation) => generation + 1);
              }}
            />
          </ScreenPane>
        )}

        {showing !== null ? null : state.status === "failed" ? (
          <section className="flex min-h-0 flex-1 flex-col">
            <StorageRecovery failure={state.failure} t={t} onUnlock={() => setAskingForMaster(true)} />
          </section>
        ) : empty ? (
          // One pane, and one thing in it. The list and the reading pane both exist to show mail;
          // with none of either, two columns of nothing is a window explaining that it is empty
          // three times over.
          <section className="flex min-h-0 flex-1 flex-col">
            <EmptyWorkspace t={t} onConnect={() => connectAccount()} />
          </section>
        ) : (
          <>
            <ResizableColumn
              width={columns.list}
              onResize={resize("list")}
              label={t("list.label")}
              // The notice about a message just filed floats over this rather than sitting under
              // it, so filing one does not shove the rows up and let them drop again.
              className="relative min-h-0 border-e border-border"
            >
              <SearchBox
                onLookFor={setLookingFor}
                autoFocusKey={mailbox ? mailboxKey(mailbox) : "none"}
              />
              {/* The drafts folder holds drafts from two places: what is being written here, which
                  lives on this device where no listing of mail finds it, and what the server's
                  folder holds, which is mail like any other. It shows both, and its list of mail
                  is the very list any other folder gets. */}
              {showingDrafts !== null ? (
                <DraftsInFolder
                  key={scopeKey(shown)}
                  accountId={showingDrafts}
                  list={folderList}
                  onOpenDraft={(answer) => {
                    setWriting(answer);
                    setInFront("writing");
                  }}
                  onChanged={() =>
                    setListGeneration((generation) => generation + 1)
                  }
                />
              ) : showingSent !== null ? (
                // The same for the folder for sent mail: what was sent from here and not read back
                // yet, above the very list any other folder gets.
                <SentInFolder
                  key={scopeKey(shown)}
                  accountId={showingSent}
                  list={folderList}
                  opened={openedHere?.id ?? null}
                  onOpen={(sent) => {
                    setOpenedHere(sent);
                    setOpenedMessage(null);
                  }}
                />
              ) : (
                <MessageList
                  // Keyed by what is being shown rather than by the mailbox, so a search is a list of
                  // its own that starts at the top -- and stays one list while somebody types, rather
                  // than throwing away its rows and its place on every letter.
                  key={scopeKey(shown)}
                  {...folderList}
                />
              )}
              <FilingNotice
                filed={back === null ? filed : null}
                onTakenBack={putBack}
                onGone={forgetWhatWasFiled}
                onUnreadAgain={(messageId, folderId) =>
                  setUnreadAgain({ messageId, folderId })
                }
                t={t}
              />
              {/* Only where it means something, and never over the notice about a message just
                  filed: two floating things in one corner is one of them hidden. The notice says
                  when it has gone, so this comes back -- held for ever, it kept this corner after
                  the first message anybody filed and never gave it up. */}
              {filed === null &&
                mailbox?.kind === "folder" &&
                mailbox.role === "trash" &&
                holds !== null && (
                  <EmptyTrash
                    folderId={mailbox.folderId}
                    holds={holds}
                    onEmptied={() =>
                      setListGeneration((generation) => generation + 1)
                    }
                    t={t}
                  />
                )}
            </ResizableColumn>

            <ReadingPane
              sentHere={openedHere}
              sentFrom={showingSent}
              opened={openedMessage}
              filed={filed}
              answerAsked={answerAsked}
              answerAllAsked={answerAllAsked}
              onReply={(answer) => {
                setWriting(answer);
                setInFront("writing");
              }}
              forwardAsked={forwardAsked}
              throwAwayAsked={throwAwayAsked}
              onForward={(forward) => {
                setWriting(forward);
                setInFront("writing");
              }}
              onFiled={filedAway}
              nothingToChoose={holds === 0}
              showPictures={preferences?.showPictures ?? "whenAsked"}
            />
          </>
        )}
      </main>
    </>
  );
}

/**
 * What the composer is keyed by: the message an opening answers, and the To, Cc and subject it
 * fills in.
 *
 * Two openings keyed alike share one composer, and the second replaces nothing, whatever else it
 * would fill differently: the text, the blind copies, the files. A draft with no To, Cc or subject
 * that answers nothing is keyed as a new message is.
 */
function openingKey(answer: Answer): string {
  return `${answer.answering ?? ""}|${answer.to}|${answer.cc}|${answer.subject}`;
}

export default App;
