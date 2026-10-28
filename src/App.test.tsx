import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { Profiler, useLayoutEffect, useState, type ReactNode } from "react";
import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";

import { I18nProvider } from "@/shared/i18n";
import type { AccountNodeDto } from "@/shared/ipc/generated/AccountNodeDto";
import type { MailboxTreeDto } from "@/shared/ipc/generated/MailboxTreeDto";
import type { MessageDetailsDto } from "@/shared/ipc/generated/MessageDetailsDto";
import App from "./App";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    getPreferences: vi.fn(),
    setTheme: vi.fn(),
    setLookForUpdates: vi.fn(),
    setShareAnonymousData: vi.fn(),
    setWelcomed: vi.fn(),
    getWorkspaceOverview: vi.fn(),
    getMailboxTree: vi.fn(),
    setAccountCollapsed: vi.fn(),
    onMailChanged: vi.fn(),
    onOpenSettings: vi.fn(),
    onMailAsked: vi.fn(),
    onTheKeyArrived: vi.fn(),
    markMessageRead: vi.fn(),
    archiveMessage: vi.fn(),
    answerAudience: vi.fn(),
    forwardMessage: vi.fn(),
    forgetAttachment: vi.fn(),
    deleteMessage: vi.fn(),
    undoAction: vi.fn(),
    getAccountSettings: vi.fn(),
    getPaneWidths: vi.fn(),
    setPaneWidth: vi.fn(),
    getSidebarFolded: vi.fn(),
    setSidebarFolded: vi.fn(),
    getViewWindow: vi.fn(),
    getMessage: vi.fn(),
    openingMessage: vi.fn(),
    setAsideLocalStorage: vi.fn(),
    syncAccount: vi.fn(),
    restartAfterSettingAside: vi.fn(),
    howThingsAre: vi.fn(),
    whatIsBeingRead: vi.fn(),
    keptDrafts: vi.fn(),
    sentHere: vi.fn(),
    discardDraft: vi.fn(),
    keepDraft: vi.fn(),
    reopenDraft: vi.fn(),
    noteDraft: vi.fn(),
    forgetNotedDraft: vi.fn(),
    getSecretStoreState: vi.fn(),
    unlockSecrets: vi.fn(),
  };
});


vi.mock("@/app/StorageRecovery", async () => {
  const actual = await vi.importActual<
    typeof import("@/app/StorageRecovery")
  >("@/app/StorageRecovery");
  return { ...actual, startTheWindowAgain: vi.fn() };
});

const client = await import("@/shared/ipc/client");
const preferences = vi.mocked(client.getPreferences);
const overview = vi.mocked(client.getWorkspaceOverview);
const tree = vi.mocked(client.getMailboxTree);

/** The sidebar as the core answers it, read before anything was asked for. */
function sidebar(accounts: AccountNodeDto[]): MailboxTreeDto {
  return { accounts, askedThrough: 0 };
}
const window_ = vi.mocked(client.getViewWindow);
const mailChanged = vi.mocked(client.onMailChanged);
const menuOpens = vi.mocked(client.onOpenSettings);
const mailAsked = vi.mocked(client.onMailAsked);
const keyArrived = vi.mocked(client.onTheKeyArrived);
const marking = vi.mocked(client.markMessageRead);
const deleting = vi.mocked(client.deleteMessage);
const archiving = vi.mocked(client.archiveMessage);
const answering = vi.mocked(client.answerAudience);
const forwarding = vi.mocked(client.forwardMessage);
const undoing = vi.mocked(client.undoAction);
const reading = vi.mocked(client.getMessage);
const opening = vi.mocked(client.openingMessage);
const paneWidths = vi.mocked(client.getPaneWidths);
const keptFolded = vi.mocked(client.getSidebarFolded);
const keepingFolded = vi.mocked(client.setSidebarFolded);
const setAside = vi.mocked(client.setAsideLocalStorage);
const restart = vi.mocked(client.restartAfterSettingAside);
const syncing = vi.mocked(client.syncAccount);
const howThings = vi.mocked(client.howThingsAre);
const beingRead = vi.mocked(client.whatIsBeingRead);
const keptHere = vi.mocked(client.keptDrafts);
const sentFromHere = vi.mocked(client.sentHere);
const throwingAway = vi.mocked(client.discardDraft);
const keeping = vi.mocked(client.keepDraft);
const reopening = vi.mocked(client.reopenDraft);
const noting = vi.mocked(client.noteDraft);
const forgetting = vi.mocked(client.forgetNotedDraft);
const secretStore = vi.mocked(client.getSecretStoreState);
const unlocking = vi.mocked(client.unlockSecrets);
const startingAgain = vi.mocked(
  (await import("@/app/StorageRecovery")).startTheWindowAgain,
);

/** Everything as it should be, which is when the activity line says nothing at all. */
const QUIET = {
  lastSuccessAt: Math.round(Date.now() / 1000) - 120,
  stoppedBy: null,
  accountsFailing: 0,
  waiting: 0,
  stopped: 0,
  cameBack: 0,
  sending: 0,
};

/** One mailbox connected, so there is something to write from and Compose is drawn. */
const ONE_ACCOUNT = {
  accounts: [
    {
      id: "acc-1",
      email: "user@example.com",
      displayName: "User",
      provider: "imap",
      canSend: true,
    },
  ],
  isLocalFirst: true,
};

const INBOX = {
  folderId: "1",
  name: "INBOX",
  role: "inbox" as const,
  total: 3,
  unread: 1,
  draftsHere: 0,
  canReceive: true,
};

const ACCOUNT = {
  accountId: "acc-1",
  email: "user@example.com",
  displayName: "User",
  collapsed: false,
  place: 0,
  answered: true,
  unread: 1,
  folders: [INBOX],
};

function renderApp() {
  render(
    <I18nProvider initial="en">
      <App />
    </I18nProvider>,
  );
}

/** What the engine on each system says it is, as far as the window can tell. */
const MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)";
const WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64)";
const LINUX = "Mozilla/5.0 (X11; Linux x86_64)";

/** The window says it is running where the test says, for a test about what that system says. */
function runningOn(agent: string) {
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue(agent);
}

afterEach(() => {
  vi.restoreAllMocks();
});

beforeEach(() => {
  preferences.mockResolvedValue({
    theme: "system",
    darkScheme: "graphite",
    lightScheme: "paper",
    appIcon: "ember",
    density: "comfortable",
    language: "en",
    newMailNotice: "inbox",
    noticeShows: { sender: true, subject: true, preview: true },
    showPictures: "whenAsked",
    lookForUpdates: "notAsked",
    shareAnonymousData: false,
    welcomed: true,
  });
  overview.mockResolvedValue({
    accounts: [],
    isLocalFirst: true,
  });
  tree.mockResolvedValue(sidebar([ACCOUNT]));
  window_.mockResolvedValue({
    rows: [],
    total: 0,
    viewEpoch: "1",
    notAnswering: [],
  });
  mailChanged.mockResolvedValue(() => {});
  menuOpens.mockReset();
  menuOpens.mockResolvedValue(() => {});
  mailAsked.mockReset();
  mailAsked.mockResolvedValue(() => {});
  keyArrived.mockReset();
  keyArrived.mockResolvedValue(() => {});
  marking.mockReset();
  marking.mockResolvedValue(1);
  reading.mockReset();
  reading.mockResolvedValue(null as never);
  opening.mockReset();
  opening.mockResolvedValue(null);
  paneWidths.mockResolvedValue({ sidebar: null, list: null });
  keptFolded.mockReset();
  keptFolded.mockResolvedValue(false);
  keepingFolded.mockReset();
  keepingFolded.mockResolvedValue(undefined);
  howThings.mockReset();
  howThings.mockResolvedValue(QUIET);
  beingRead.mockReset();
  beingRead.mockResolvedValue({ done: 0, asked: 0, through: 0, of: 0 });
  keptHere.mockReset();
  keptHere.mockResolvedValue([]);
  sentFromHere.mockReset();
  sentFromHere.mockResolvedValue([]);
  throwingAway.mockReset();
  throwingAway.mockResolvedValue(undefined);
  keeping.mockReset();
  keeping.mockResolvedValue(9);
  reopening.mockReset();
  reopening.mockResolvedValue([]);
  noting.mockReset();
  noting.mockResolvedValue(undefined);
  forgetting.mockReset();
  forgetting.mockResolvedValue(undefined);
  // Reset with the rest, so a count of calls is about the test that makes them.
  answering.mockReset();
  forwarding.mockReset();
  deleting.mockReset();
  vi.mocked(client.forgetAttachment).mockReset();
  vi.mocked(client.forgetAttachment).mockResolvedValue(undefined);
  syncing.mockReset();
  syncing.mockResolvedValue({
    folders: 0,
    arrived: 0,
    updated: 0,
    removed: 0,
    unseenInInbox: 0,
    foldersReset: 0,
    operationsToReresolve: 0,
    operationsFound: 0,
  } as never);
  setAside.mockReset();
  secretStore.mockReset();
  secretStore.mockResolvedValue("system");
  unlocking.mockReset();
  unlocking.mockResolvedValue(undefined);
  startingAgain.mockReset();
});

describe("application shell", () => {
  it("offers to connect a mailbox from the sidebar", async () => {
    // The entry point cannot live only in the empty state of the list: a workspace with mail in it
    // then has no way to add a second account.
    renderApp();

    const connect = await screen.findByRole("button", {
      name: "Connect an email account",
    });
    expect(connect).toBeTruthy();
  });

  it("keeps offering it once messages are in the list", async () => {
    window_.mockResolvedValue({
      rows: [
        {
          inConversation: 1,
          conversationId: null,
          messageId: "m-1",
          folderId: "1",
          stopped: null,
          subject: "A message",
          sender: "sender@example.com",
          preview: null,
          receivedAt: 1_700_000_000,
          unread: true,
          leaving: false,
          arriving: false,
        },
      ],
      total: 1,
      viewEpoch: "1",
      notAnswering: [],
    });
    renderApp();

    expect(
      await screen.findByRole("button", { name: "Connect an email account" }),
    ).toBeTruthy();
  });

  it("reads again when the core says something changed", async () => {
    // Mail that arrives on its own is the point of the background pass; a window that only reads
    // when asked would show yesterday's list until someone pressed refresh.
    let announce = (_change: unknown) => {};
    mailChanged.mockImplementation(async (handle) => {
      announce = handle as (change: unknown) => void;
      return () => {};
    });
    renderApp();

    await waitFor(() => expect(mailChanged).toHaveBeenCalled());
    const before = tree.mock.calls.length;

    announce({ accountId: "acc-1", arrived: 1, updated: 0, removed: 0 });

    await waitFor(() => expect(tree.mock.calls.length).toBeGreaterThan(before));
  });

  it("stops listening when the window goes", async () => {
    const stop = vi.fn();
    mailChanged.mockResolvedValue(stop);
    const { unmount } = render(
      <I18nProvider initial="en">
        <App />
      </I18nProvider>,
    );

    await waitFor(() => expect(mailChanged).toHaveBeenCalled());
    unmount();

    await waitFor(() => expect(stop).toHaveBeenCalled());
  });

  it("opens the account widths it was left at", async () => {
    paneWidths.mockResolvedValue({ sidebar: 300, list: 500 });
    renderApp();

    await waitFor(() =>
      expect(
        screen.getByRole("separator", { name: "Resize Mailboxes" })
          .parentElement?.style.width,
      ).toBe("300px"),
    );
  });

  it("opens the account form when it is pressed", async () => {
    renderApp();

    fireEvent.click(
      await screen.findByRole("button", { name: "Connect an email account" }),
    );

    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Connect a mailbox" }),
      ).toBeTruthy(),
    );
  });

  it("moves the corner out from under the window's own buttons", async () => {
    // On a window with no bar of its own, the close, minimise and zoom buttons float over the
    // top-left corner. What sits there has to start below them; the rest of the window gains the
    // height the bar used to take.
    renderApp();
    await screen.findByRole("button", { name: "Connect an email account" });

    const corner = document.querySelector(".under-window-controls");

    expect(
      corner,
      "nothing is marked as sitting under the buttons",
    ).toBeTruthy();
  });

  it("asks about looking for updates on the onboarding and in the settings, and nowhere else", async () => {
    // Settings nobody answered and a window past its welcome: the window behind it asks nothing.
    // The onboarding comes back at the next start for whoever went on without saving, and the
    // settings hold the same box.
    renderApp();

    expect(await screen.findByRole("button", { name: "Settings" })).toBeTruthy();
    expect(screen.queryByText(/updates/i)).toBeNull();
    expect(screen.queryByRole("button", { name: "Yes" })).toBeNull();
  });

  it("offers settings from the part of the window that is always there", async () => {
    // Not from an empty state and not from a menu that only exists once there is mail: a way in
    // that appears later is a way in nobody finds on the first day.
    renderApp();

    const gear = await screen.findByRole("button", { name: "Settings" });

    expect(gear.textContent).toBe("");
    expect(gear.querySelector("svg")).toBeTruthy();
  });

  it("shows the settings screen when it is pressed, and takes the button away", async () => {
    // The button opens and does nothing else. Left on the page beside the screen it opens, it is
    // a second answer to a question nobody is asking -- and the way out is the cross in the
    // corner, which is where it is on every screen.
    renderApp();

    fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
    expect(
      await screen.findByRole("heading", { name: "Settings" }),
    ).toBeTruthy();

    expect(screen.queryByRole("button", { name: "Settings" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    await waitFor(() =>
      expect(screen.queryByRole("heading", { name: "Settings" })).toBeNull(),
    );
    expect(screen.getByRole("button", { name: "Settings" })).toBeTruthy();
  });

  it("leaves the settings by the cross in the corner", async () => {
    // The screen covers the list and the message, and the gear that opened it is behind it. A way
    // out that is only the control a person cannot see is a screen they are stuck on.
    renderApp();

    fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
    await screen.findByRole("heading", { name: "Settings" });

    const cross = screen.getByRole("button", { name: "Close this screen" });
    // Stuck to the top and taking no height, so it is still there once a long screen has been
    // scrolled and nothing below it has moved to make room. Asserted as the rule rather than as
    // the effect: this runs without layout, so nothing here can scroll.
    const pinned = cross.closest("div.sticky");
    expect(pinned, "the cross is pinned to the top of the pane").toBeTruthy();
    expect(pinned!.className).toContain("h-0");

    fireEvent.click(cross);

    await waitFor(() =>
      expect(screen.queryByRole("heading", { name: "Settings" })).toBeNull(),
    );
    // Gone, not crashed: the window it goes back to is still there.
    expect(screen.getByRole("button", { name: "Settings" })).toBeTruthy();
  });

  it("opens a screen at the top, whatever the last one had been scrolled to", async () => {
    // The screens stand in one place, one after another in the same slot, and React reuses the
    // element between them unless each is told it is a different thing. Going from one straight
    // to the next -- the settings, then the account they open -- carried where the first had been
    // scrolled to into the second, so the second arrived halfway down a page nobody had scrolled.
    overview.mockResolvedValue({
      accounts: [
        {
          id: "acc-1",
          email: "user@example.com",
          displayName: "User",
          provider: "imap",
          canSend: true,
        },
      ],
      isLocalFirst: true,
    });
    vi.mocked(client.getAccountSettings).mockResolvedValue({
      email: "user@example.com",
      displayName: "User",
      imapHost: "imap.example.com",
      imapPort: 993,
      imapEncryption: "tls",
      imapUsername: "user@example.com",
      submission: null,
    });
    renderApp();

    fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
    const settings = (
      await screen.findByRole("heading", { name: "Settings" })
    ).closest("section");
    expect(settings, "the settings stand in a pane of their own").toBeTruthy();
    settings!.scrollTop = 240;

    fireEvent.click(screen.getByRole("button", { name: "Edit User" }));

    const account = (
      await screen.findByRole("button", { name: "Close this screen" })
    ).closest("section");
    // A pane of its own, not the one just vacated wearing new content: where a screen starts is
    // settled by which element it is, and the offset only follows from that.
    expect(account).not.toBe(settings);
    expect(account!.scrollTop).toBe(0);
  });

  it("does not ask the servers again just because the settings were opened", async () => {
    // The sidebar remembers which accounts it has already asked, and that memory lives in the
    // sidebar. Taken out of the tree and put back, it forgets -- so an account whose folders never
    // arrive was asked again every time somebody opened the settings, with the button turning for
    // each of them and never stopping.
    tree.mockResolvedValue(
      sidebar([
        {
          accountId: "acc-1",
          email: "user@example.com",
          displayName: "User",
          collapsed: false,
          place: 0,
          unread: 0,
          answered: true,
          // None, so the sidebar asks its server about them -- once.
          folders: [],
        },
      ]),
    );

    renderApp();
    await waitFor(() => expect(syncing).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    await screen.findByRole("heading", { name: "Settings" });
    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    await screen.findByRole("navigation", { name: "Mailboxes" });
    // A second asking would come after the sidebar has read itself again, which is a round trip
    // away. Asserted too soon, this passes on a sidebar that is about to ask.
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(syncing).toHaveBeenCalledTimes(1);
  });

  it("puts the parts of the settings where the mailboxes were, and back again", async () => {
    // The folders of an account answer a question nobody is asking with this screen open, and a
    // screen this long needs a way around it. The column beside it is where both of those live.
    renderApp();

    fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
    await screen.findByRole("heading", { name: "Settings" });

    expect(screen.getByRole("navigation", { name: "Settings" })).toBeTruthy();
    // Out of sight, though still in the tree: what it remembers about which accounts it has
    // already asked has to survive the settings being opened over it.
    expect(screen.queryByRole("navigation", { name: "Mailboxes" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));

    await waitFor(() =>
      expect(screen.queryByRole("navigation", { name: "Settings" })).toBeNull(),
    );
    // The mailboxes are back, rather than the column being left empty.
    expect(screen.getByRole("navigation", { name: "Mailboxes" })).toBeTruthy();
  });

  it("puts the settings over a half-written message and gives it back", async () => {
    // The sidebar stands beside every screen, so the gear can be pressed while a message is being
    // written. The press brings the settings out in front at once, rather than being remembered
    // and showing them by themselves once the message is put away.
    overview.mockResolvedValue({
      accounts: [
        {
          id: "acc-1",
          email: "user@example.com",
          displayName: "User",
          provider: "imap",
          canSend: true,
        },
      ],
      isLocalFirst: true,
    });
    renderApp();

    // Drawn, and unavailable, while the workspace is still being read: a press then does nothing.
    const compose = await screen.findByRole("button", { name: /^compose$/i });
    await waitFor(() =>
      expect((compose as HTMLButtonElement).disabled).toBe(false),
    );
    fireEvent.click(compose);
    const subject = await screen.findByLabelText(/^subject$/i);
    fireEvent.change(subject, { target: { value: "half a thought" } });

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(
      await screen.findByRole("heading", { name: "Settings" }),
      "the settings come out in front rather than being remembered silently",
    ).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));

    // Back to the message, with what was typed still in it.
    expect(
      ((await screen.findByLabelText(/^subject$/i)) as HTMLInputElement).value,
    ).toBe("half a thought");
  });

  it("wears what was stored, colours and all", async () => {
    preferences.mockResolvedValue({
      theme: "dark",
      darkScheme: "nord",
      lightScheme: "gruvboxLight",
      appIcon: "ember",
      density: "compact",
      language: "en",
      newMailNotice: "inbox",
  noticeShows: { sender: true, subject: true, preview: true },
    showPictures: "whenAsked",
    lookForUpdates: "notAsked",
    shareAnonymousData: false,
    welcomed: true,
    });
    renderApp();

    await waitFor(() => {
      expect(document.documentElement.classList.contains("dark")).toBe(true);
      expect(document.documentElement.getAttribute("data-scheme")).toBe("nord");
      expect(
        document.documentElement.getAttribute("data-light-scheme"),
      ).toBe("gruvboxLight");
    });
  });

  it("names the scheme even while the window is light, so dark arrives already right", async () => {
    // Otherwise the colours would be wrong for the moment between the system turning dark and the
    // application noticing.
    preferences.mockResolvedValue({
      theme: "light",
      darkScheme: "dracula",
      lightScheme: "rosePineDawn",
      appIcon: "ember",
      density: "comfortable",
      language: "en",
      newMailNotice: "inbox",
  noticeShows: { sender: true, subject: true, preview: true },
    showPictures: "whenAsked",
    lookForUpdates: "notAsked",
    shareAnonymousData: false,
    welcomed: true,
    });
    renderApp();

    await waitFor(() =>
      expect(document.documentElement.getAttribute("data-scheme")).toBe(
        "dracula",
      ),
    );
    expect(document.documentElement.getAttribute("data-light-scheme")).toBe(
      "rosePineDawn",
    );
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("puts on the colours chosen in the change that draws the palettes offered for them", async () => {
    // Put on after the window is painted, the side just chosen shows its palettes in the other
    // side's colours for a frame, and the window flickers.
    vi.mocked(client.setTheme).mockResolvedValue({
      theme: "dark",
      darkScheme: "graphite",
      lightScheme: "paper",
      appIcon: "ember",
      density: "comfortable",
      language: "en",
      newMailNotice: "inbox",
      noticeShows: { sender: true, subject: true, preview: true },
      showPictures: "whenAsked",
      lookForUpdates: "notAsked",
      shareAnonymousData: false,
      welcomed: true,
    });
    // A profiler hears of each change after the effects that run before painting, and before the
    // ones that run after it.
    const changes: Array<{ palettes: boolean; colours: boolean }> = [];
    render(
      <I18nProvider initial="en">
        <Profiler
          id="window"
          onRender={() =>
            changes.push({
              palettes: document.body.textContent?.includes("Dark colors") ?? false,
              colours: document.documentElement.classList.contains("dark"),
            })
          }
        >
          <App />
        </Profiler>
      </I18nProvider>,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
    fireEvent.click(await screen.findByRole("button", { name: "Dark" }));

    await screen.findByRole("group", { name: "Dark colors" });
    const withThePalettes = changes.filter((change) => change.palettes);
    expect(withThePalettes.length).toBeGreaterThan(0);
    expect(withThePalettes.every((change) => change.colours)).toBe(true);
  });

  it("offers a way out when the local files will not open, instead of a notice about it", async () => {
    // Refusing to destroy what somebody wrote is right. Leaving them with a window that says so
    // and offers nothing is a different thing: the application becomes a description of its own
    // trouble, and a person who cannot get past it cannot even reach their settings.
    const broken = {
      code: "storageUnavailable",
      message: "The local database could not be opened",
      path: "/somewhere/profile.sqlite3",
    };
    overview.mockRejectedValue(broken);
    preferences.mockRejectedValue(broken);
    setAside.mockResolvedValue({
      moved: ["/somewhere/profile.sqlite3.set-aside-1"],
    });

    renderApp();

    await screen.findByText(/local storage is unavailable/i);

    // The explanation under the button is read before the button is pressed, so it is the last
    // thing that can still lie. It told people to quit and start the app again — advice for a
    // build that did not restart itself, kept next to one that does, which is two instructions
    // for one press and the reason somebody follows neither.
    expect(screen.queryByText(/start the app again afterwards/i)).toBeNull();

    fireEvent.click(await screen.findByRole("button", { name: /start fresh/i }));

    await waitFor(() => expect(setAside).toHaveBeenCalled());
    expect(
      await screen.findByText(/somewhere\/profile\.sqlite3\.set-aside-1/),
    ).toBeTruthy();
    // A path reads left to right in every language.
    expect(
      screen
        .getByText(/somewhere\/profile\.sqlite3\.set-aside-1/)
        .getAttribute("dir"),
    ).toBe("ltr");

    // What the person is left reading, which is the thing that was wrong: the advice about a file
    // that is no longer in the way stayed on screen under the answer, so two instructions
    // contradicted each other and neither said what to do next.
    expect(screen.queryByText(/quit the app and check/i)).toBeNull();
    expect(screen.queryByText(/could not be opened/i)).toBeNull();
    expect(screen.queryByRole("button", { name: /start fresh/i })).toBeNull();
    await waitFor(() => expect(restart).toHaveBeenCalled());
  });

  it("says a newer version wrote the files and offers nothing that moves them", async () => {
    // Going back to an older build is not damage. The newer version reads these files as they are,
    // and setting them aside would leave it a fresh, empty profile to find instead — so the screen
    // names the file, says which version opens it, and stops there.
    const newer = {
      code: "storageFromANewerBuild",
      message: "the database was written by a newer build, at version 23",
      path: "/somewhere/profile.sqlite3",
    };
    overview.mockRejectedValue(newer);
    preferences.mockRejectedValue(newer);
    // Refused the same way: everything that asks storage is told what the screen is told, and the
    // sidebar says nothing beside a screen that says it all.
    tree.mockRejectedValue(newer);

    renderApp();

    const screenOf = await screen.findByRole("alert");
    expect(screenOf.textContent).toMatch(
      /a newer version of lyonoyl\smail wrote a file this version does not open/i,
    );
    expect(screenOf.textContent).toContain("/somewhere/profile.sqlite3");
    expect(screenOf.textContent).toMatch(/nothing in it has been changed/i);
    expect(screen.getAllByText("/somewhere/profile.sqlite3")).toHaveLength(1);
    // A path reads left to right in every language.
    expect(
      screen.getByText("/somewhere/profile.sqlite3").getAttribute("dir"),
    ).toBe("ltr");
    // The advice for a damaged file, and the way out that would cost the newer version its files.
    expect(screen.queryByText(/check that the file above is readable/i)).toBeNull();
    expect(screen.queryByRole("button", { name: /start fresh/i })).toBeNull();
    expect(setAside).not.toHaveBeenCalled();
  });

  it("names one account's file a newer version wrote beside a recovery screen up for something else", async () => {
    // The profile opened, so the screen is about whatever else went wrong and says nothing of the
    // account's file. The sidebar is the one place left to name it.
    const damaged = {
      code: "storageCorrupted",
      message: "an account row this build cannot read",
      path: null,
    };
    overview.mockRejectedValue(damaged);
    preferences.mockRejectedValue(damaged);
    tree.mockRejectedValue({
      code: "storageFromANewerBuild",
      message: "the database was written by a newer build, at version 28",
      path: "/somewhere/accounts/3.sqlite3",
    });

    renderApp();

    expect(await screen.findByText("/somewhere/accounts/3.sqlite3")).toBeTruthy();
    expect(
      screen.getByText(
        /a newer version of lyonoyl\smail wrote a file this version does not open/i,
      ),
    ).toBeTruthy();
  });

  it("says what Start fresh costs without naming a store, on a device whose secrets are in a file", async () => {
    // What it leaves behind is the same whichever store the passwords are in, and naming one is
    // naming a keyring to somebody who has none.
    runningOn(LINUX);
    const broken = {
      code: "storageUnavailable",
      message: "The local database could not be opened",
      path: "/somewhere/profile.sqlite3",
    };
    overview.mockRejectedValue(broken);
    preferences.mockRejectedValue(broken);
    tree.mockRejectedValue(broken);
    secretStore.mockResolvedValue("unlocked");

    renderApp();

    expect(
      await screen.findByText(/restarts itself with no accounts/i),
    ).toBeTruthy();
    expect(screen.queryByText(/keyring/i)).toBeNull();
  });

  it("offers to choose the master password from the recovery screen when there is no file yet", async () => {
    runningOn(LINUX);
    const waiting = {
      code: "storageKeyUnavailable",
      message: "The key this device keeps its mail under could not be read.",
      path: "/somewhere/profile.sqlite3",
    };
    overview.mockRejectedValue(waiting);
    preferences.mockRejectedValue(waiting);
    tree.mockRejectedValue(waiting);
    secretStore.mockResolvedValue("needsChoosing");

    renderApp();

    fireEvent.click(
      within(await screen.findByRole("dialog")).getByRole("button", {
        name: /not now/i,
      }),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    const recovery = await screen.findByRole("alert");
    expect(
      await within(recovery).findByRole("button", { name: /set password/i }),
    ).toBeTruthy();
    // There is no file yet and no databases either: no keyring is named, no path, and nothing
    // offers to set aside what is not there.
    expect(within(recovery).queryByText(/system keyring/i)).toBeNull();
    expect(within(recovery).queryByText(/profile\.sqlite3/)).toBeNull();
    expect(
      within(recovery).queryByRole("button", { name: /start fresh/i }),
    ).toBeNull();
  });

  it("says the file holds no key for the databases when it is open, and leaves a way out", async () => {
    // A file of secrets open, and databases beside it written under a key it does not hold. Nothing
    // here can be unlocked again, and no keyring is worth starting: a device with a file reads its
    // key from nowhere else. What is left is setting the files aside.
    runningOn(LINUX);
    const waiting = {
      code: "storageKeyUnavailable",
      message: "The key this device keeps its mail under could not be read.",
      path: "/somewhere/profile.sqlite3",
    };
    overview.mockRejectedValue(waiting);
    preferences.mockRejectedValue(waiting);
    tree.mockRejectedValue(waiting);
    secretStore.mockResolvedValue("unlocked");

    renderApp();

    const recovery = await screen.findByRole("alert");
    expect(
      await within(recovery).findByText(
        /is not in the file your master password opens/i,
      ),
    ).toBeTruthy();
    // The files Start fresh sets aside are named — the sentence calls them the file above — and
    // no keyring is: a device with a file looks for its key nowhere else.
    expect(within(recovery).getByText(/profile\.sqlite3/)).toBeTruthy();
    expect(within(recovery).queryByText(/keyring/i)).toBeNull();
    expect(
      within(recovery).queryByRole("button", { name: /^unlock$/i }),
    ).toBeNull();
    expect(
      within(recovery).getByRole("button", { name: /start fresh/i }),
    ).toBeTruthy();
    expect(
      within(recovery).getByText(/restarts itself with no accounts/i),
    ).toBeTruthy();
  });

  it("opens the window again once the master password has opened what waited for it", async () => {
    // A device with no store of its own keeps the key its databases are written under in the file
    // the master password opens. Until then nothing the window asks for can be read, and once it is
    // open, everything the window read at its start was read from nothing.
    const waiting = {
      code: "storageKeyUnavailable",
      message: "The key this device keeps its mail under could not be read.",
      path: "/somewhere/profile.sqlite3",
    };
    overview.mockRejectedValue(waiting);
    preferences.mockRejectedValue(waiting);
    tree.mockRejectedValue(waiting);
    secretStore.mockResolvedValue("needsUnlocking");

    renderApp();

    const panel = await screen.findByRole("dialog");
    fireEvent.change(within(panel).getByLabelText(/master password/i), {
      target: { value: "correct horse battery staple" },
    });
    fireEvent.click(within(panel).getByRole("button", { name: /^unlock$/i }));

    await waitFor(() => expect(startingAgain).toHaveBeenCalled());
    expect(unlocking).toHaveBeenCalledWith("correct horse battery staple");
  });

  it("opens the window again when the master password is given before the window has its answer", async () => {
    // What the window holds then is nothing yet rather than a failure. The databases open all the
    // same, and the answer it is still waiting for was asked for before they did.
    overview.mockReturnValue(new Promise(() => {}));
    preferences.mockReturnValue(new Promise(() => {}));
    secretStore.mockResolvedValue("needsUnlocking");

    renderApp();

    const panel = await screen.findByRole("dialog");
    fireEvent.change(within(panel).getByLabelText(/master password/i), {
      target: { value: "correct horse battery staple" },
    });
    fireEvent.click(within(panel).getByRole("button", { name: /^unlock$/i }));

    await waitFor(() => expect(startingAgain).toHaveBeenCalled());
  });

  it("offers the master password again from the recovery screen once it was put off", async () => {
    // Put off, the panel goes, and what is left is the recovery screen. On a device with no store
    // of its own its advice about a store is advice about something that is not there, and the one
    // thing that opens the databases is the password.
    runningOn(LINUX);
    const waiting = {
      code: "storageKeyUnavailable",
      message: "The key this device keeps its mail under could not be read.",
      path: "/somewhere/profile.sqlite3",
    };
    overview.mockRejectedValue(waiting);
    preferences.mockRejectedValue(waiting);
    tree.mockRejectedValue(waiting);
    secretStore.mockResolvedValue("needsUnlocking");

    renderApp();

    fireEvent.click(
      within(await screen.findByRole("dialog")).getByRole("button", {
        name: /not now/i,
      }),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    const recovery = await screen.findByRole("alert");
    // What the screen says is about the file this device has, not a keyring it does not — and the
    // key is only closed in it, so nothing offers to set the databases aside.
    expect(
      await within(recovery).findByText(/are in a file on this device/i),
    ).toBeTruthy();
    expect(within(recovery).queryByText(/system keyring/i)).toBeNull();
    expect(within(recovery).queryByText(/allow access/i)).toBeNull();
    expect(within(recovery).queryByText(/profile\.sqlite3/)).toBeNull();
    expect(
      within(recovery).queryByRole("button", { name: /start fresh/i }),
    ).toBeNull();
    fireEvent.click(
      await within(recovery).findByRole("button", { name: /^unlock$/i }),
    );

    expect(await screen.findByRole("dialog")).toBeTruthy();
  });

  it("does not answer a press on settings with silence when there are none to show", async () => {
    // Appearance and language live in the file that will not open, so there is nothing to show.
    // Said by the control being unavailable rather than by it doing nothing.
    const broken = {
      code: "storageUnavailable",
      message: "The local database could not be opened",
      path: null,
    };
    overview.mockRejectedValue(broken);
    preferences.mockRejectedValue(broken);

    renderApp();

    await screen.findByText(/local storage is unavailable/i);
    // Unavailable is only half of it: a person still has to find out why, or the control simply
    // looks broken. The reason is the button's name, which is reachable by a screen reader and by
    // a keyboard — unlike a tooltip on a control that takes no pointer events.
    const gear = screen.getByRole("button", {
      name: /kept in the local database/i,
    });
    expect((gear as HTMLButtonElement).disabled).toBe(true);
    // And a hint for a mouse, which is the one reader the name does not reach: a disabled control
    // takes no pointer events, so the span around it takes them instead.
    const hint = gear.closest("span[data-state]");
    expect(hint).toBeTruthy();
    // The span is what stands in the foot, over the button that folds the sidebar.
    const fold = screen.getByRole("button", { name: "Collapse the sidebar" });
    expect(hint!.parentElement).toBe(fold.parentElement);
    expect(hint!.nextElementSibling).toBe(fold);
  });

  it("opens the inbox, not whichever folder the server named first", async () => {
    // A server lists its mailboxes in whatever order it likes, and the one this is developed
    // against lists INBOX last. Opened by position, a person landed in Archive or Sent -- with
    // mail in it -- while the inbox they came for sat unread and unopened.
    tree.mockResolvedValue(
      sidebar([
        {
          accountId: "acc-1",
          email: "user@example.com",
          displayName: "User",
          collapsed: false,
          place: 0,
          unread: 0,
          answered: true,
          folders: [
            {
              folderId: "10",
              name: "Sent",
              role: "sent",
              total: 2,
              unread: 0,
              draftsHere: 0,
              canReceive: false,
            },
            {
              folderId: "11",
              name: "INBOX",
              role: "inbox",
              total: 16,
              unread: 3,
              draftsHere: 0,
              canReceive: true,
            },
          ],
        },
      ]),
    );

    renderApp();
    await screen.findByRole("button", { name: /connect|add/i });

    // The sidebar names the inbox by its role, so a server that lists it last is still the folder
    // that opens -- which is the order every server this ships against happens to use.
    const inbox = await screen.findByRole("button", { name: /inbox/i });
    await waitFor(() =>
      expect(inbox.getAttribute("aria-current")).toBe("true"),
    );
  });

  it("starts adding a mailbox with the three things a person knows", async () => {
    // Adding one is a short screen first and the long form only for the mailbox nobody can answer
    // for. Opened straight onto twelve fields, it is a window people close at the fourth.
    renderApp();

    fireEvent.click(
      await screen.findByRole("button", { name: /connect|add/i }),
    );

    expect(await screen.findByLabelText(/^Email address$/)).toBeTruthy();
    expect(screen.queryByLabelText(/^Server address$/)).toBeNull();
    // Nothing behind it yet, so nothing to go back to -- but always a way out.
    expect(screen.queryByRole("button", { name: "Back" })).toBeNull();
    expect(
      screen.getByRole("button", { name: "Close this screen" }),
    ).toBeTruthy();
  });

  it("hangs no hint on the settings button once it can be pressed", async () => {
    // A hint on a working button opened whenever the button took the focus -- and the focus is
    // put back on it by hand when the settings close, so a hint appeared over a screen the person
    // had just left and stayed there, with no pointer anywhere near it to take it away.
    renderApp();

    const gear = await screen.findByRole("button", { name: "Settings" });

    expect(gear.closest("span[data-state]")).toBeNull();
  });
});

describe("App, the side column", () => {
  it("takes Compose out of the column while the settings are in front, and puts it back", async () => {
    // The column lists the parts of the settings while they are open, and a button for writing a
    // message above that list would push it down by a button and a gap for nothing: nobody
    // writes from the settings. The list starts at the top, where Compose stands otherwise.
    overview.mockResolvedValue(ONE_ACCOUNT);
    renderApp();

    // Drawn, and unavailable, while the workspace is still being read: a press then does nothing.
    const compose = await screen.findByRole("button", { name: /^compose$/i });
    await waitFor(() =>
      expect((compose as HTMLButtonElement).disabled).toBe(false),
    );
    fireEvent.click(compose);
    await screen.findByLabelText(/^subject$/i);
    // Only the settings take it away: over a message being written it stays, and pressing it
    // there starts a new one.
    expect(screen.getByRole("button", { name: /^compose$/i })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    await screen.findByRole("heading", { name: "Settings" });

    expect(screen.queryByRole("button", { name: /^compose$/i })).toBeNull();
    const column = screen.getByRole("separator", {
      name: "Resize Settings",
    }).parentElement!;
    const first = [...column.children].find(
      (child) => !(child as HTMLElement).hidden,
    );
    expect(
      first?.contains(screen.getByRole("navigation", { name: "Settings" })),
      "the parts of the settings are the first thing in the column",
    ).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));

    // Back to the message, and Compose back above the mailboxes.
    await screen.findByLabelText(/^subject$/i);
    expect(screen.getByRole("button", { name: /^compose$/i })).toBeTruthy();
  });

  it("gives the focus back to Compose when the settings were opened from it by key", async () => {
    // A person who tabbed to Compose and pressed the settings key comes back to where they were
    // standing, not to the gear at the other end of the column.
    overview.mockResolvedValue(ONE_ACCOUNT);
    renderApp();

    const compose = await screen.findByRole("button", { name: /^compose$/i });
    await waitFor(() =>
      expect((compose as HTMLButtonElement).disabled).toBe(false),
    );
    compose.focus();

    fireEvent.keyDown(window, { code: "Comma", metaKey: true });
    await screen.findByRole("heading", { name: "Settings" });
    expect(screen.queryByRole("button", { name: /^compose$/i })).toBeNull();

    fireEvent.keyDown(window, { key: "Escape" });

    await waitFor(() =>
      expect(document.activeElement).toBe(
        screen.getByRole("button", { name: /^compose$/i }),
      ),
    );
  });

  it("puts the settings button and the one that folds the sidebar side by side, beside the activity line", async () => {
    // Rows at the foot of the column -- one saying how things are, the others holding a button
    // each -- would be rows of height taken from the mailboxes. The line takes the width the two
    // buttons leave it, and the foot is one button tall.
    howThings.mockResolvedValue({ ...QUIET, stoppedBy: "offline" });
    renderApp();

    const gear = await screen.findByRole("button", { name: "Settings" });
    const fold = screen.getByRole("button", { name: "Collapse the sidebar" });
    const line = (await screen.findByText(/no connection/i)).closest(
      "[role=status]",
    );

    expect(line, "the activity line is drawn").toBeTruthy();
    // In one row: the gear, and the fold after it.
    const buttons = gear.parentElement!;
    expect(fold.parentElement).toBe(buttons);
    expect(buttons.firstElementChild).toBe(gear);
    expect(buttons.lastElementChild).toBe(fold);
    expect(buttons.className.split(/\s+/)).not.toContain("flex-col");
    // After the line, so the two stand at the end of its row.
    expect(line!.parentElement).toBe(buttons.parentElement);
    expect(buttons.parentElement!.lastElementChild).toBe(buttons);
    // Along the bottom rather than the middle: a line that wraps grows upward, and the buttons
    // stay where they stood at the column's foot rather than climbing with it.
    const foot = buttons.parentElement!.className.split(/\s+/);
    expect(foot).toContain("items-end");
    expect(foot).not.toContain("items-center");
  });

  it("leaves nothing at the foot while the settings are in front and nothing is reported", async () => {
    // The gear goes with the settings in front, there is nothing to fold beside their parts, and the
    // line says nothing when all is well. A foot kept for none of them is a gap under the list.
    renderApp();

    const gear = await screen.findByRole("button", { name: "Settings" });
    const buttons = gear.parentElement!;
    const foot = buttons.parentElement!;
    // Asked, and answered with nothing to say, before anything is counted.
    await waitFor(() => expect(howThings).toHaveBeenCalled());
    expect([...foot.children]).toEqual([buttons]);

    fireEvent.click(gear);
    await screen.findByRole("heading", { name: "Settings" });

    expect(foot.isConnected, "the foot is the same element").toBe(true);
    expect([...foot.children]).toEqual([buttons]);
    expect(screen.queryByRole("button", { name: /the sidebar$/ })).toBeNull();
    // Hidden rather than taken out, so the focus can come back to it.
    expect(buttons.querySelector("button")!.hidden).toBe(true);
  });

  it("gives the focus back to the fold when the settings were opened from it by key", async () => {
    renderApp();

    const fold = await screen.findByRole("button", {
      name: "Collapse the sidebar",
    });
    // The window is out of reach until its settings say nobody is to be greeted.
    await waitFor(() =>
      expect(document.querySelector("[data-onboarding]")).toBeNull(),
    );
    fold.focus();

    fireEvent.keyDown(window, { code: "Comma", metaKey: true });
    await screen.findByRole("heading", { name: "Settings" });
    fireEvent.keyDown(window, { key: "Escape" });

    await waitFor(() => expect(document.activeElement).toBe(fold));
  });

  it("keeps the activity line at the foot while the settings are in front", async () => {
    // What is being read does not stop mattering because the settings are open. The gear leaves
    // and the fold has nothing to fold; the line stays.
    howThings.mockResolvedValue({ ...QUIET, stoppedBy: "offline" });
    renderApp();

    fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
    await screen.findByRole("heading", { name: "Settings" });

    const line = (await screen.findByText(/no connection/i)).closest(
      "[role=status]",
    )!;
    expect(line.parentElement!.className).toContain("mt-auto");
    expect(screen.queryByRole("button", { name: "Settings" })).toBeNull();
    expect(screen.queryByRole("button", { name: /the sidebar$/ })).toBeNull();
  });

});

describe("App, the sidebar folded into a rail", () => {
  /** The column the sidebar is, found by the mailboxes it lists. */
  function theSidebar(): HTMLElement {
    return screen
      .getByRole("navigation", { name: "Mailboxes" })
      .closest("section")!;
  }

  /** Compose, once there is something to write from. */
  async function composeIsReady(name: string | RegExp) {
    const compose = await screen.findByRole("button", { name });
    await waitFor(() =>
      expect((compose as HTMLButtonElement).disabled).toBe(false),
    );
    return compose;
  }

  it("opens on a first start", async () => {
    overview.mockResolvedValue(ONE_ACCOUNT);
    renderApp();

    await waitFor(() => expect(keptFolded).toHaveBeenCalled());
    const compose = await composeIsReady(/^compose$/i);

    // Compose in words, the heading over the mailboxes, and the plus that adds one.
    expect(compose.querySelector("svg")).toBeNull();
    expect(screen.getByText("Mailboxes")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Connect an email account" }),
    ).toBeTruthy();
    expect(screen.getByRole("separator", { name: "Resize Mailboxes" })).toBeTruthy();
  });

  it("folds into a rail by the button under the gear, and keeps it that way", async () => {
    overview.mockResolvedValue(ONE_ACCOUNT);
    renderApp();
    await composeIsReady(/^compose$/i);

    fireEvent.click(screen.getByRole("button", { name: "Collapse the sidebar" }));

    expect(keepingFolded).toHaveBeenCalledExactlyOnceWith(true);
    const rail = theSidebar();
    expect(rail.style.width).toBe("72px");
    // The gear above, and the button that opens it again under the gear: the rail has no width for
    // a row of them.
    const gear = within(rail).getByRole("button", { name: "Settings" });
    const unfold = within(rail).getByRole("button", { name: "Expand the sidebar" });
    expect(unfold.parentElement).toBe(gear.parentElement);
    expect(gear.nextElementSibling).toBe(unfold);
    expect(gear.parentElement!.className.split(/\s+/)).toContain("flex-col");
    // An edge that cannot be dragged has no handle to drag it by.
    expect(screen.queryByRole("separator", { name: "Resize Mailboxes" })).toBeNull();

    // Compose is an icon, and the button that gets mail stands under it.
    const buttons = within(rail).getAllByRole("button");
    const compose = within(rail).getByRole("button", { name: "Compose" });
    expect(compose.querySelector("svg.lucide-square-pen")).toBeTruthy();
    const getMail = within(rail).getByRole("button", { name: "Get new mail" });
    expect(buttons.indexOf(getMail)).toBe(buttons.indexOf(compose) + 1);

    // The plus goes, and the heading in words with it.
    expect(
      screen.queryByRole("button", { name: "Connect an email account" }),
    ).toBeNull();
    expect(screen.queryByText("Mailboxes")).toBeNull();

    // The mailboxes are icons.
    expect(
      within(rail)
        .getByRole("button", { name: /^Inbox/ })
        .querySelector("svg.lucide-inbox"),
    ).toBeTruthy();
  });

  it("opens again by the same button, at the width it was dragged to", async () => {
    paneWidths.mockResolvedValue({ sidebar: 300, list: null });
    keptFolded.mockResolvedValue(true);
    renderApp();

    const unfold = await screen.findByRole("button", {
      name: "Expand the sidebar",
    });
    unfold.focus();
    fireEvent.click(unfold);

    expect(keepingFolded).toHaveBeenCalledExactlyOnceWith(false);
    await waitFor(() => expect(theSidebar().style.width).toBe("300px"));
    expect(screen.getByRole("separator", { name: "Resize Mailboxes" })).toBeTruthy();
    // The same button, named for what it does now, and still where the keyboard left the focus.
    const fold = screen.getByRole("button", { name: "Collapse the sidebar" });
    expect(fold).toBe(unfold);
    expect(document.activeElement).toBe(fold);
  });

  it("draws a sidebar left folded folded from the first frame, before the core answers", () => {
    // The core says it in the address the window opens at, read before it made the window.
    window.history.replaceState(null, "", "/index.html?sidebar=folded");
    keptFolded.mockReturnValue(new Promise<boolean>(() => {}));
    try {
      renderApp();

      expect(
        screen.getByRole("button", { name: "Expand the sidebar" }),
      ).toBeTruthy();
      expect(theSidebar().style.width).toBe("72px");
      expect(theSidebar().className).not.toContain("transition-[width]");
    } finally {
      window.history.replaceState(null, "", "/");
    }
  });

  it("follows what is kept in a window started again since its address was written", async () => {
    // A window started again keeps the address it was made with, which can be older than the fold.
    window.history.replaceState(null, "", "/index.html?sidebar=folded");
    keptFolded.mockResolvedValue(false);
    try {
      renderApp();

      expect(
        await screen.findByRole("button", { name: "Collapse the sidebar" }),
      ).toBeTruthy();
    } finally {
      window.history.replaceState(null, "", "/");
    }
  });

  it("opens folded where it was left folded, without moving", async () => {
    // What was kept arrives after the first draw. Moving to it would be the window folding its
    // own sidebar while the person watches, every time it starts.
    keptFolded.mockResolvedValue(true);
    renderApp();

    await screen.findByRole("button", { name: "Expand the sidebar" });
    const rail = theSidebar();

    expect(rail.style.width).toBe("72px");
    expect(rail.className).not.toContain("transition-[width]");
    expect(rail.style.transitionDuration).toBe("");
    expect((rail.firstElementChild as HTMLElement).style.width).toBe("");
  });

  it("moves the edge for the person who folds it, and stops once it is there", async () => {
    renderApp();

    fireEvent.click(
      await screen.findByRole("button", { name: "Collapse the sidebar" }),
    );
    const rail = theSidebar();
    const inside = rail.firstElementChild as HTMLElement;

    // On its way, and what is inside is cut off at the edge rather than spilling over the list,
    // laid out at the width it is going to.
    expect(rail.className).toContain("transition-[width]");
    expect(rail.className).toContain("overflow-hidden");
    expect(rail.style.transitionDuration).toBe("200ms");
    expect(inside.style.width).toBe("72px");

    await waitFor(() =>
      expect(rail.className).not.toContain("transition-[width]"),
    );
    expect(rail.className).not.toContain("overflow-hidden");
    expect(inside.style.width).toBe("");

    // And the same way back, to the width it opens at.
    fireEvent.click(screen.getByRole("button", { name: "Expand the sidebar" }));
    expect(rail.className).toContain("transition-[width]");
    expect(inside.style.width).toBe("240px");
    await waitFor(() =>
      expect(rail.className).not.toContain("transition-[width]"),
    );
  });

  it("keeps a press made before what was kept arrived", async () => {
    let answer: (folded: boolean) => void = () => {};
    keptFolded.mockReturnValue(
      new Promise<boolean>((resolve) => {
        answer = resolve;
      }),
    );
    renderApp();

    fireEvent.click(
      await screen.findByRole("button", { name: "Collapse the sidebar" }),
    );
    await act(async () => answer(false));

    expect(screen.getByRole("button", { name: "Expand the sidebar" })).toBeTruthy();
    expect(theSidebar().style.width).toBe("72px");
  });

  it("stays folded for the session when it could not be kept", async () => {
    // As a width that could not be kept does: what is on screen is what the person asked for,
    // and a red panel over a sidebar is a large answer to a small thing.
    keepingFolded.mockRejectedValue(new Error("the profile is busy"));
    renderApp();

    fireEvent.click(
      await screen.findByRole("button", { name: "Collapse the sidebar" }),
    );
    await waitFor(() => expect(keepingFolded).toHaveBeenCalled());

    expect(screen.getByRole("button", { name: "Expand the sidebar" })).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("names every icon in a hint", async () => {
    overview.mockResolvedValue(ONE_ACCOUNT);
    keptFolded.mockResolvedValue(true);
    renderApp();
    // Folded first: the column is drawn open until what was kept arrives.
    await screen.findByRole("button", { name: "Expand the sidebar" });
    await composeIsReady("Compose");
    const rail = theSidebar();

    for (const name of [
      "Compose",
      "Get new mail",
      "Inbox",
      "Expand the sidebar",
    ]) {
      const button = within(rail).getByRole("button", {
        name: new RegExp(`^${name}`),
      });
      act(() => button.focus());
      expect((await screen.findByRole("tooltip")).textContent).toContain(name);
      act(() => button.blur());
      await waitFor(() => expect(screen.queryByRole("tooltip")).toBeNull());
    }
  });

  it("names the gear in a hint under the pointer, and not when the focus is handed back to it", async () => {
    keptFolded.mockResolvedValue(true);
    renderApp();
    const gear = await screen.findByRole("button", { name: "Settings" });

    fireEvent.pointerEnter(gear);
    expect((await screen.findByRole("tooltip")).textContent).toBe("Settings");
    fireEvent.pointerLeave(gear);
    await waitFor(() => expect(screen.queryByRole("tooltip")).toBeNull());

    // Opened and closed, the settings give the focus back to the gear, which nobody is pointing at.
    fireEvent.click(gear);
    await screen.findByRole("heading", { name: "Settings" });
    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    await waitFor(() =>
      expect(document.activeElement).toBe(
        screen.getByRole("button", { name: "Settings" }),
      ),
    );
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("opens the column beside the settings with their parts whole, and folds it again when they close", async () => {
    // The parts of the settings are a list of words, and the rail has no room for one: beside the
    // settings the column is open whatever was kept, with nothing to fold, and what was kept is
    // left as it was.
    overview.mockResolvedValue(ONE_ACCOUNT);
    keptFolded.mockResolvedValue(true);
    renderApp();
    await screen.findByRole("button", { name: "Expand the sidebar" });
    await composeIsReady("Compose");
    const column = theSidebar();
    expect(column.style.width).toBe("72px");

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    await screen.findByRole("heading", { name: "Settings" });

    expect(column.style.width).toBe("240px");
    expect(
      within(column).getByRole("navigation", { name: "Settings" }),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: /the sidebar$/ })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));

    expect(
      await screen.findByRole("button", { name: "Expand the sidebar" }),
    ).toBeTruthy();
    expect(column.style.width).toBe("72px");
    expect(keepingFolded).not.toHaveBeenCalled();
  });

  it("opens the sidebar from a mark that stands for what the foot would say", async () => {
    howThings.mockResolvedValue({ ...QUIET, stoppedBy: "offline" });
    keptFolded.mockResolvedValue(true);
    renderApp();

    const mark = await screen.findByRole("button", { name: /no connection/i });
    fireEvent.click(mark);

    expect(keepingFolded).toHaveBeenCalledExactlyOnceWith(false);
    expect(
      (await screen.findByText(/no connection/i)).closest("p"),
      "said in words in the open foot",
    ).toBeTruthy();
  });
});

describe("App, a key the system store would not hand over", () => {
  // Said differently by system, because the way back differs: the keychain and Credential Manager
  // are always there, and a Linux keyring may not be running at all.
  const refused = {
    code: "storageKeyUnavailable",
    message: "The key this device keeps its mail under could not be read.",
    path: "/somewhere/profile.sqlite3",
  };

  it("reads everything again when the key arrives after the window came up without it", async () => {
    // The keychain asks a person before it hands the key to a new build, and they can answer after
    // the start stopped waiting. The core opens the databases with the key that came and says so;
    // a window that went on showing this screen would be telling somebody who had just allowed it
    // that the key could not be had.
    runningOn(MAC);
    overview.mockRejectedValue(refused);
    preferences.mockRejectedValue(refused);
    tree.mockRejectedValue(refused);
    let arrived: (() => void) | undefined;
    keyArrived.mockImplementation(async (handler) => {
      arrived = handler;
      return () => {};
    });
    startingAgain.mockClear();

    renderApp();
    await screen.findByRole("alert");
    expect(startingAgain).not.toHaveBeenCalled();

    expect(arrived).toBeDefined();
    arrived?.();

    expect(startingAgain).toHaveBeenCalledTimes(1);
  });

  it.each([
    ["a Mac", MAC],
    ["Windows", WINDOWS],
  ])(
    "does not offer to set the files aside when a store that is always there refused the key, on %s",
    async (_, agent) => {
      // Two failures reach this screen and only one of them is a file. When the system store will
      // not hand over the key, the files are whole and the key is where it has always been —
      // setting them aside repairs nothing and costs the account settings and everything unsent.
      // Offering it there is offering somebody a way to lose what they wrote for nothing.
      runningOn(agent);
      overview.mockRejectedValue(refused);
      preferences.mockRejectedValue(refused);
      tree.mockRejectedValue(refused);

      renderApp();

      const recovery = await screen.findByRole("alert");
      // What is said instead is something a person can act on — said once the screen has its own
      // answer about the store, so it is said about the store this device has.
      expect(
        await within(recovery).findByText(
          /Allow access when your system asks, and the app opens by itself\. If access was refused, quit the app and open it again$/,
        ),
      ).toBeTruthy();
      // Said once, by that sentence, about the files the path names.
      expect(within(recovery).queryByText(/could not be read/i)).toBeNull();
      expect(within(recovery).getByText(/profile\.sqlite3/)).toBeTruthy();

      // The button, and the sentence under it that explains what it costs.
      expect(
        within(recovery).queryByRole("button", { name: /start fresh/i }),
      ).toBeNull();
      expect(within(recovery).queryByText(/are not carried over/i)).toBeNull();

      // A device with a store of its own has no master password to offer.
      expect(
        within(recovery).queryByRole("button", { name: /^unlock$/i }),
      ).toBeNull();
    },
  );

  it("says to start the keyring the key is in, and leaves a way out if it is gone", async () => {
    // Databases written under a key a Linux keyring holds go on asking that keyring, and a file is
    // never made over them. It may not be running, or be locked — or be gone for good, and a screen
    // that only says to start it is one nobody leaves then.
    runningOn(LINUX);
    overview.mockRejectedValue(refused);
    preferences.mockRejectedValue(refused);
    tree.mockRejectedValue(refused);

    renderApp();

    const recovery = await screen.findByRole("alert");
    expect(
      await within(recovery).findByText(/start or unlock the system keyring/i),
    ).toBeTruthy();
    expect(within(recovery).queryByText(/allow access/i)).toBeNull();
    expect(within(recovery).queryByText(/could not be read/i)).toBeNull();
    expect(within(recovery).getByText(/profile\.sqlite3/)).toBeTruthy();
    expect(
      within(recovery).getByRole("button", { name: /start fresh/i }),
    ).toBeTruthy();
    // What comes after the press, said before it: not that the passwords stay in a keyring that may
    // be gone, but what the next start does if it still does not answer.
    expect(
      within(recovery).getByText(/asks for a master password/i),
    ).toBeTruthy();
    expect(within(recovery).queryByText(/your passwords stay in/i)).toBeNull();
    // Never a master password: a file chosen now would hold everything but that key.
    expect(
      within(recovery).queryByRole("button", {
        name: /^unlock$|set password/i,
      }),
    ).toBeNull();
  });

  it("says nothing about the key until it knows which store this device has", async () => {
    // Every sentence here is about a store, and naming the wrong one is worse than naming none:
    // somebody whose secrets are in a file would be sent to start a keyring.
    runningOn(LINUX);
    let answer: (state: "system") => void = () => {};
    secretStore.mockReturnValue(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    overview.mockRejectedValue(refused);
    preferences.mockRejectedValue(refused);
    tree.mockRejectedValue(refused);

    renderApp();

    const recovery = await screen.findByRole("alert");
    expect(within(recovery).queryByText(/start or unlock the system keyring/i)).toBeNull();
    expect(within(recovery).queryByText(/could not be read/i)).toBeNull();
    expect(within(recovery).queryByText(/profile\.sqlite3/)).toBeNull();
    expect(within(recovery).queryByRole("button")).toBeNull();

    await act(async () => {
      answer("system");
    });

    expect(
      await within(recovery).findByText(/start or unlock the system keyring/i),
    ).toBeTruthy();
  });
});

describe("the application menu's settings item", () => {
  it("opens the settings when the menu asks, as the key does", async () => {
    renderApp();
    await screen.findByRole("button", { name: "Settings" });
    await waitFor(() => expect(menuOpens).toHaveBeenCalled());
    const chosen = menuOpens.mock.calls[menuOpens.mock.calls.length - 1][0];

    act(() => chosen());

    expect(
      await screen.findByRole("heading", { name: "Settings" }),
    ).toBeTruthy();
  });

  it("stops listening to the menu once the window has gone", async () => {
    const stop = vi.fn();
    menuOpens.mockResolvedValue(stop);
    const { unmount } = render(
      <I18nProvider initial="en">
        <App />
      </I18nProvider>,
    );
    await waitFor(() => expect(menuOpens).toHaveBeenCalled());
    await act(async () => {});

    unmount();

    expect(stop).toHaveBeenCalled();
  });
});

describe("App, the onboarding", () => {
  /** A profile that has never been through it: what the window greets. */
  async function firstStart() {
    const stored = await preferences();
    preferences.mockResolvedValue({ ...stored, welcomed: false });
  }

  it("greets nobody who has finished it", async () => {
    renderApp();

    expect(
      await screen.findByRole("button", { name: "Settings" }),
    ).toBeTruthy();
    expect(document.querySelector("[data-onboarding]")).toBeNull();
    expect(document.querySelector("main")?.hasAttribute("inert")).toBe(false);
  });

  it("greets a person at the start, over a window it puts out of reach", async () => {
    await firstStart();
    renderApp();

    expect(
      await screen.findByRole("dialog", { name: "Welcome to Lyonoyl\u00a0Mail" }),
    ).toBeTruthy();
    expect(document.querySelector("main")?.hasAttribute("inert")).toBe(true);
  });

  it("leaves the window's keys alone while it is up", async () => {
    // The settings opening behind it would be the window doing something nobody can see.
    await firstStart();
    renderApp();
    await screen.findByRole("dialog", { name: "Welcome to Lyonoyl\u00a0Mail" });

    fireEvent.keyDown(window, { code: "Comma", metaKey: true });

    expect(screen.queryByRole("heading", { name: "Settings" })).toBeNull();
  });

  it("hides the window from the first frame, while the settings are still being read", async () => {
    // Drawn only once they arrived, it would show the window for the moment before.
    await firstStart();
    preferences.mockReturnValue(new Promise(() => {}));
    renderApp();

    expect(document.querySelector("[data-onboarding]")).not.toBeNull();
    expect(document.querySelector("main")?.hasAttribute("inert")).toBe(true);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("gives the window back once it is finished, its keys with it", async () => {
    await firstStart();
    vi.mocked(client.setLookForUpdates).mockImplementation(async (look) => ({
      ...(await preferences()),
      lookForUpdates: look,
    }));
    vi.mocked(client.setWelcomed).mockImplementation(async (welcomed) => ({
      ...(await preferences()),
      lookForUpdates: "onceADay",
      welcomed,
    }));
    renderApp();
    await screen.findByRole("dialog", { name: "Welcome to Lyonoyl\u00a0Mail" });
    for (const _ of [1, 2, 3]) {
      fireEvent.click(screen.getByRole("button", { name: "Next" }));
    }
    fireEvent.click(screen.getByRole("button", { name: "Finish" }));

    // Gone once it has faded, and not before: the window is under it until then.
    await waitFor(() =>
      expect(document.querySelector("[data-onboarding]")).toBeNull(),
    );
    expect(document.querySelector("main")?.hasAttribute("inert")).toBe(false);
    // The window listens again a moment after the fading ends, which is outside any press: the key
    // is pressed until it answers, rather than once at an instant that may come before it.
    await waitFor(() => {
      fireEvent.keyDown(window, { code: "Comma", metaKey: true });
      expect(screen.getByRole("heading", { name: "Settings" })).toBeTruthy();
    });
  });

  it("does not greet a window that could not open its mail", async () => {
    // The trouble a failed start explains comes first; a welcome over it would hide it.
    await firstStart();
    overview.mockRejectedValue(new Error("the files would not open"));
    renderApp();

    await waitFor(() => expect(overview).toHaveBeenCalled());
    await act(async () => {});
    expect(
      screen.queryByRole("dialog", { name: "Welcome to Lyonoyl\u00a0Mail" }),
    ).toBeNull();
  });
});

describe("leaving a screen by key", () => {
  it("opens the settings on the key every application on the desk uses", async () => {
    renderApp();
    await screen.findByRole("button", { name: "Settings" });

    fireEvent.keyDown(window, { code: "Comma", metaKey: true });

    expect(
      await screen.findByRole("heading", { name: "Settings" }),
    ).toBeTruthy();
  });

  it("leaves the settings on Escape", async () => {
    // The shortcut list has promised this since before anything answered it.
    renderApp();
    fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
    await screen.findByRole("heading", { name: "Settings" });

    fireEvent.keyDown(window, { key: "Escape" });

    await waitFor(() =>
      expect(screen.queryByRole("heading", { name: "Settings" })).toBeNull(),
    );
  });

  it("does not leave while an input method is composing", async () => {
    // Escape is how a half-typed string is cancelled, and there it belongs to the keyboard rather
    // than to this window. It is the one shortcut here that answers while a field has the focus,
    // so nothing else protects it.
    renderApp();
    fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
    await screen.findByRole("heading", { name: "Settings" });

    fireEvent.keyDown(window, { key: "Escape", isComposing: true });
    fireEvent.keyDown(window, { key: "Escape", keyCode: 229 });

    await new Promise((settle) => setTimeout(settle, 40));
    expect(screen.getByRole("heading", { name: "Settings" })).toBeTruthy();
  });

  it("leaves nothing when no screen is open", async () => {
    // Escape with the list in front is a key that has nothing to do, and must not throw.
    renderApp();
    await screen.findByRole("button", { name: "Settings" });

    fireEvent.keyDown(window, { key: "Escape" });

    expect(screen.getByRole("button", { name: "Settings" })).toBeTruthy();
  });
});

describe("App, focus when a screen replaces the pane", () => {
  it("moves the focus into the screen that just opened", async () => {
    // Swapping the pane left focus on a button that no longer exists, so it fell to the body: the
    // new screen was never announced, and Tab started again from the top of the window instead of
    // continuing inside what had just opened.
    renderApp();

    const gear = await screen.findByRole("button", { name: /settings/i });
    fireEvent.click(gear);

    await waitFor(() => {
      const focused = document.activeElement;
      expect(focused?.tagName).toBe("SECTION");
    });
  });

  it("gives the focus back to where it came from when the screen closes", async () => {
    renderApp();

    const gear = await screen.findByRole("button", { name: /settings/i });
    // Pressed, which in a browser also focuses it. The test says so out loud because the DOM used
    // here does not do it, and the thing under test is where the focus goes back to.
    gear.focus();
    fireEvent.click(gear);
    await waitFor(() =>
      expect(document.activeElement?.tagName).toBe("SECTION"),
    );

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));

    // Back to the control that opened it, rather than to the top of the window. Asked for by
    // name rather than held on to: the gear leaves the page while the settings are on it, so the
    // one that comes back is the same control and a different element.
    await waitFor(() =>
      expect(document.activeElement).toBe(
        screen.getByRole("button", { name: /settings/i }),
      ),
    );
  });
});

describe("App, the keys the window answers to", () => {
  it("opens the list of keys, and closes it on Escape", async () => {
    // A shortcut nobody knows about is a shortcut that does not exist.
    renderApp();
    await screen.findByRole("button", { name: /settings/i });

    fireEvent.keyDown(window, { key: "?" });

    const panel = await screen.findByRole("dialog");
    expect(panel.textContent).toMatch(/write a message/i);
    expect(panel.textContent).toMatch(/forward the message being read/i);
    expect(panel.textContent).toMatch(/reply to everybody on the message being read/i);
    // Each with its letter and its chord, as this system writes a chord.
    expect(panel.textContent).toContain("c \u00b7 Ctrl+N");
    expect(panel.textContent).toContain("a \u00b7 Ctrl+Shift+R");
    expect(panel.textContent).toContain("Delete \u00b7 Ctrl+Delete");

    fireEvent.keyDown(panel, { key: "Escape", bubbles: true });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("does not act on a letter typed into a field", async () => {
    // `c` in a subject line must not throw away the message being written and open another.
    renderApp();
    await screen.findByRole("button", { name: /settings/i });

    const field = document.createElement("input");
    document.body.appendChild(field);
    field.focus();
    fireEvent.keyDown(field, { key: "?", bubbles: true });

    expect(screen.queryByRole("dialog")).toBeNull();
    field.remove();
  });

  it("does not act on a letter held with a system key", async () => {
    renderApp();
    await screen.findByRole("button", { name: /settings/i });

    fireEvent.keyDown(window, { key: "?", metaKey: true });

    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("App, the keys a list or a question keeps to itself", () => {
  const USER_SETTINGS = {
    email: "user@example.com",
    displayName: "User",
    imapHost: "imap.example.com",
    imapPort: 993,
    imapEncryption: "tls" as const,
    imapUsername: "user@example.com",
    submission: null,
  };

  it("does not open a message when a letter is typed at a dropdown in the settings", async () => {
    // Typing the first letters of an entry is how a long list is got through. The window answers
    // `c` by opening a message, and the same letter doing both would put one over the settings.
    overview.mockResolvedValue(ONE_ACCOUNT);
    renderApp();

    fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
    const language = await screen.findByRole("combobox", {
      name: "Interface language",
    });
    language.focus();
    for (const key of ["c", "r", "?"]) fireEvent.keyDown(language, { key });

    expect(screen.queryByLabelText(/^subject$/i)).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("heading", { name: "Settings" })).toBeTruthy();
  });

  it("does not act behind the question asked before an account is removed", async () => {
    overview.mockResolvedValue(ONE_ACCOUNT);
    vi.mocked(client.getAccountSettings).mockResolvedValue(USER_SETTINGS);
    renderApp();

    fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
    fireEvent.click(await screen.findByRole("button", { name: "Edit User" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "Remove this account" }),
    );
    const question = await screen.findByRole("alertdialog");
    const keep = within(question).getByRole("button", { name: "Keep it" });
    keep.focus();

    fireEvent.keyDown(keep, { key: "c" });
    fireEvent.keyDown(keep, { key: "?" });
    fireEvent.keyDown(keep, { key: ",", code: "Comma", metaKey: true });

    // Nothing opened behind it: no message, no list of keys, no settings.
    expect(screen.queryByLabelText(/^subject$/i)).toBeNull();
    expect(document.querySelector("[data-settings-body]")).toBeNull();
    expect(screen.getByRole("alertdialog")).toBe(question);
  });

  it("gives each account a form of its own, so nothing typed for one reaches another", async () => {
    // The account screen stays open while the settings turn it to another account. One form for
    // both carried the password typed for the first into the second's fields.
    overview.mockResolvedValue({
      ...ONE_ACCOUNT,
      accounts: [
        ...ONE_ACCOUNT.accounts,
        {
          id: "acc-2",
          email: "zeta@example.com",
          displayName: "Zeta Quorin",
          provider: "imap",
          canSend: true,
        },
      ],
    });
    vi.mocked(client.getAccountSettings).mockImplementation(async (id) =>
      id === "acc-2"
        ? {
            ...USER_SETTINGS,
            email: "zeta@example.com",
            displayName: "Zeta Quorin",
            imapUsername: "zeta@example.com",
          }
        : USER_SETTINGS,
    );
    renderApp();

    fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
    fireEvent.click(await screen.findByRole("button", { name: "Edit User" }));
    const email = () =>
      document.getElementById("account-email") as HTMLInputElement | null;
    const password = () =>
      document.getElementById("account-password") as HTMLInputElement;
    await waitFor(() => expect(email()?.value).toBe("user@example.com"));
    fireEvent.change(password(), { target: { value: "typed for the first" } });

    fireEvent.keyDown(window, { key: ",", code: "Comma", metaKey: true });
    fireEvent.click(
      await screen.findByRole("button", { name: "Edit Zeta Quorin" }),
    );

    await waitFor(() => expect(email()?.value).toBe("zeta@example.com"));
    expect(password().value).toBe("");
    // And the removal it offers is of this account, by this account's address.
    fireEvent.click(
      await screen.findByRole("button", { name: "Remove this account" }),
    );
    expect(
      await screen.findByRole("alertdialog", {
        name: "Remove zeta@example.com from this device?",
      }),
    ).toBeTruthy();
  });
});

describe("App, before anybody has connected a mailbox", () => {
  it("shows the one thing there is to do, and none of the controls that answer nothing", async () => {
    // A search box over nothing to search, a reading pane beside nothing to read and a compose
    // button on an account that does not exist are three controls that do nothing — and a first
    // screen made mostly of those reads as an application that is already broken.
    overview.mockResolvedValue({
      accounts: [],
      isLocalFirst: true,
    });
    tree.mockResolvedValue(sidebar([]));

    renderApp();

    expect(await screen.findByText(/connect your first mailbox/i)).toBeTruthy();

    expect(screen.queryByPlaceholderText(/search/i)).toBeNull();
    expect(screen.queryByRole("button", { name: /^compose$/i })).toBeNull();
    // Named as the catalogue names it. "Refresh" appears nowhere: the button says "Get new mail",
    // and a check for the word in the code rather than the word on screen matches nothing.
    expect(screen.queryByRole("button", { name: "Get new mail" })).toBeNull();
    expect(screen.queryByText(/select a message to read it/i)).toBeNull();
  });

  it("tells a device whose secrets are in a file that a password goes to that file", async () => {
    // The first thing anybody does after choosing a master password is add a mailbox, and the
    // screen for it says where the password goes. Naming the system's store there is the confusion
    // the master password exists to prevent.
    secretStore.mockResolvedValue("unlocked");
    overview.mockResolvedValue({
      accounts: [],
      isLocalFirst: true,
    });
    tree.mockResolvedValue(sidebar([]));

    renderApp();

    fireEvent.click(
      await screen.findByRole("button", { name: "Connect an email account" }),
    );

    expect(
      await screen.findByText(/kept in the file your master password opens/i),
    ).toBeTruthy();
  });

  it("brings them all back once there is a mailbox", async () => {
    // The other half, and the one that matters: hiding a control is only right while it answers
    // nothing, and a check on the empty state alone would pass on a build that hid them for ever.
    overview.mockResolvedValue({
      accounts: [
        {
          id: "acc-1",
          email: "user@example.com",
          displayName: "User",
          provider: "imap",
          canSend: true,
        },
      ],
      isLocalFirst: true,
    });

    renderApp();

    expect(
      await screen.findByRole("button", { name: /^compose$/i }),
    ).toBeTruthy();
    expect(screen.getByPlaceholderText(/search/i)).toBeTruthy();
    expect(
      await screen.findByRole("button", { name: "Get new mail" }),
    ).toBeTruthy();
  });
});

describe("App, the folder for sent mail", () => {
  const SENT = {
    folderId: "9",
    name: "Sent",
    role: "sent" as const,
    total: 1,
    unread: 0,
    draftsHere: 0,
    canReceive: false,
  };

  /** Sent from here a moment ago, and not taken by a server yet. */
  const FROM_HERE = {
    id: 4,
    sender: "user@example.com",
    to: ["kappa@example.net"],
    cc: [],
    bcc: ["sigma@example.org"],
    subject: "About Tuesday",
    body: "Shall we meet at noon?",
    preview: "Shall we meet at noon?",
    files: [],
    writtenAt: 1_700_000_000,
    onItsWay: true,
  };

  /** An account whose folder for sent mail holds one message, with one more sent from here. */
  function withSentMail() {
    overview.mockResolvedValue(ONE_ACCOUNT);
    tree.mockResolvedValue(sidebar([{ ...ACCOUNT, folders: [INBOX, SENT] }]));
    sentFromHere.mockResolvedValue([FROM_HERE]);
    window_.mockImplementation(async (request) =>
      request.scope.kind === "folder" && request.scope.folderId === "9"
        ? {
            rows: [
              {
                inConversation: 1,
                conversationId: null,
                messageId: "30",
                folderId: "9",
                stopped: null,
                subject: "Last week",
                sender: "user@example.com",
                preview: null,
                receivedAt: 1_699_000_000,
                unread: false,
                leaving: false,
                arriving: false,
              },
            ],
            total: 1,
            viewEpoch: "1",
            notAnswering: [],
          }
        : { rows: [], total: 0, viewEpoch: "1", notAnswering: [] },
    );
    renderApp();
  }

  it("shows a message sent from here above what the server's folder holds", async () => {
    withSentMail();

    fireEvent.click(await screen.findByRole("button", { name: /^sent/i }));

    expect(await screen.findByText("About Tuesday")).toBeTruthy();
    expect(sentFromHere).toHaveBeenCalledWith("acc-1");
    // And the server's part under it: the list this window draws for any folder.
    await waitFor(() =>
      expect(window_).toHaveBeenCalledWith(
        expect.objectContaining({ scope: { kind: "folder", folderId: "9" } }),
      ),
    );
    expect(await screen.findByRole("listbox")).toBeTruthy();
    expect(screen.queryByText("This folder is empty")).toBeNull();
  });

  it("asks nothing about what was sent from here in any other folder", async () => {
    withSentMail();

    await screen.findByRole("button", { name: /^sent/i });
    fireEvent.click(await screen.findByRole("button", { name: /^inbox/i }));
    await waitFor(() =>
      expect(window_).toHaveBeenCalledWith(
        expect.objectContaining({ scope: { kind: "folder", folderId: "1" } }),
      ),
    );

    expect(sentFromHere).not.toHaveBeenCalled();
  });

  it("opens a message sent from here in the reader, and one from the list after it", async () => {
    withSentMail();
    fireEvent.click(await screen.findByRole("button", { name: /^sent/i }));

    fireEvent.click(await screen.findByText("About Tuesday"));

    const article = await screen.findByRole("article");
    expect(within(article).getByText("Blind copy to")).toBeTruthy();
    expect(within(article).getByText("sigma@example.org")).toBeTruthy();
    expect(reading).not.toHaveBeenCalled();

    // The server's message, chosen the way the keyboard chooses one.
    fireEvent.keyDown(await screen.findByRole("listbox"), { key: "ArrowDown" });

    await waitFor(() => expect(reading).toHaveBeenCalledWith("30"));
    expect(screen.queryByText("Blind copy to")).toBeNull();
  });

  it("puts the message from here away when another folder is chosen", async () => {
    withSentMail();
    fireEvent.click(await screen.findByRole("button", { name: /^sent/i }));
    fireEvent.click(await screen.findByText("About Tuesday"));
    await screen.findByText("Blind copy to");

    fireEvent.click(screen.getByRole("button", { name: /^inbox/i }));

    await waitFor(() => expect(screen.queryByText("Blind copy to")).toBeNull());
    expect(screen.queryByText("sigma@example.org")).toBeNull();
  });
});

describe("App, the drafts folder", () => {
  /** A drafts folder whose server holds one draft, and nothing kept on this device. */
  const DRAFTS = {
    folderId: "12",
    name: "Drafts",
    role: "drafts" as const,
    total: 1,
    unread: 0,
    draftsHere: 0,
    canReceive: false,
  };

  it("shows the draft the server holds, which the count beside it counts", async () => {
    // Saved by another client and brought in by a sync. The sidebar counts it, so the folder has
    // to show it -- listing only what is kept on this device, it would say it is empty.
    overview.mockResolvedValue(ONE_ACCOUNT);
    tree.mockResolvedValue(sidebar([{ ...ACCOUNT, folders: [INBOX, DRAFTS] }]));
    window_.mockImplementation(async (request) =>
      request.scope.kind === "folder" && request.scope.folderId === "12"
        ? {
            rows: [
              {
                inConversation: 1,
                conversationId: null,
                messageId: "30",
                folderId: "12",
                stopped: null,
                subject: "From the phone",
                sender: "zeta@example.com",
                preview: null,
                receivedAt: 1_700_000_000,
                unread: false,
                leaving: false,
                arriving: false,
              },
            ],
            total: 1,
            viewEpoch: "1",
            notAnswering: [],
          }
        : { rows: [], total: 0, viewEpoch: "1", notAnswering: [] },
    );
    renderApp();

    fireEvent.click(await screen.findByRole("button", { name: /^drafts/i }));

    await waitFor(() =>
      expect(window_).toHaveBeenCalledWith(
        expect.objectContaining({ scope: { kind: "folder", folderId: "12" } }),
      ),
    );
    await waitFor(() => expect(keptHere).toHaveBeenCalledWith("acc-1"));
    expect(screen.queryByText("This folder is empty")).toBeNull();
  });

  it("reads the count again when a draft kept here is thrown away", async () => {
    overview.mockResolvedValue(ONE_ACCOUNT);
    tree.mockResolvedValue(
      sidebar([
        {
          ...ACCOUNT,
          folders: [INBOX, { ...DRAFTS, total: 0, draftsHere: 1 }],
        },
      ]),
    );
    keptHere.mockResolvedValue([
      {
        id: 4,
        recipients: "kappa@example.net",
        copies: "",
        blind: "",
        subject: "About Tuesday",
        body: "",
        answering: null,
        chain: [],
      },
    ]);
    renderApp();

    fireEvent.click(await screen.findByRole("button", { name: /^drafts/i }));
    await screen.findByText("About Tuesday");
    const before = tree.mock.calls.length;

    fireEvent.click(screen.getByRole("button", { name: /discard the draft/i }));

    await waitFor(() => expect(throwingAway).toHaveBeenCalledWith(4));
    await waitFor(() => expect(tree.mock.calls.length).toBeGreaterThan(before));
  });

  it("does not say there is nothing to read beside drafts kept here", async () => {
    // Told only what the server's half held, the pane said "nothing here to read" beside a list
    // of drafts it could see.
    overview.mockResolvedValue(ONE_ACCOUNT);
    tree.mockResolvedValue(
      sidebar([
        {
          ...ACCOUNT,
          folders: [INBOX, { ...DRAFTS, total: 0, draftsHere: 1 }],
        },
      ]),
    );
    keptHere.mockResolvedValue([
      {
        id: 4,
        recipients: "kappa@example.net",
        copies: "",
        blind: "",
        subject: "About Tuesday",
        body: "",
        answering: null,
        chain: [],
      },
    ]);
    renderApp();

    fireEvent.click(await screen.findByRole("button", { name: /^drafts/i }));
    await screen.findByText("About Tuesday");
    // Every answer in, so what the pane says is what it settled on rather than what it said first.
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));

    expect(screen.getByText("Select a message to read it")).toBeTruthy();
    expect(screen.queryByText("Nothing here to read")).toBeNull();
  });

  it("reads the count again when what is being written is first kept", async () => {
    // Kept, it is on this device and in the folder the moment the window closes, so the number
    // beside Drafts counts it from then -- not from whenever a sync next happened to land.
    overview.mockResolvedValue(ONE_ACCOUNT);
    tree.mockResolvedValue(sidebar([{ ...ACCOUNT, folders: [INBOX, DRAFTS] }]));
    renderApp();

    const compose = await screen.findByRole("button", { name: /^compose$/i });
    await waitFor(() =>
      expect((compose as HTMLButtonElement).disabled).toBe(false),
    );
    fireEvent.click(compose);
    const before = tree.mock.calls.length;
    fireEvent.change(await screen.findByLabelText(/^Message$/), {
      target: { value: "about Tuesday" },
    });

    await waitFor(() => expect(keeping).toHaveBeenCalled(), { timeout: 2000 });
    await waitFor(() => expect(tree.mock.calls.length).toBeGreaterThan(before));
  });
});

describe("App, a new message asked for over one being written", () => {
  /** A drafts folder holding one draft kept on this device, and nothing on the server. */
  const KEPT_HERE = {
    folderId: "12",
    name: "Drafts",
    role: "drafts" as const,
    total: 0,
    unread: 0,
    draftsHere: 1,
    canReceive: false,
  };
  /** What an account whose drafts hold as much as they may answers every keep with. */
  const NO_ROOM = { code: "invalidRequest" as const, message: "", path: null };
  const field = (label: RegExp) =>
    screen.getByLabelText(label) as HTMLInputElement | HTMLTextAreaElement;

  /** The draft kept here: to Kappa, about Tuesday, and no text yet. */
  const ABOUT_TUESDAY = {
    id: 4,
    recipients: "kappa@example.net",
    copies: "",
    blind: "",
    subject: "About Tuesday",
    body: "",
    answering: null,
    chain: [],
  };

  /**
   * The window with `draft` opened from the drafts folder, ready for typing. `after` is drawn
   * beside the window, after it.
   */
  async function openTheDraft(draft = ABOUT_TUESDAY, after: ReactNode = null) {
    overview.mockResolvedValue(ONE_ACCOUNT);
    tree.mockResolvedValue(
      sidebar([{ ...ACCOUNT, folders: [INBOX, KEPT_HERE] }]),
    );
    keptHere.mockResolvedValue([draft]);
    render(
      <I18nProvider initial="en">
        <App />
        {after}
      </I18nProvider>,
    );
    fireEvent.click(await screen.findByRole("button", { name: /^drafts/i }));
    fireEvent.click(await screen.findByText(draft.subject || "(no subject)"));
    await waitFor(() => expect(field(/^Message$/).disabled).toBe(false));
  }

  it("waits for a draft being written to leave by its own way out, and loses nothing to keeps the drafts refuse", async () => {
    // Opened over the draft at once, the new message would take everything typed into it since it
    // was opened, without a word. Asked for by the button or by its key, it opens once the draft
    // has left the way the cross leaves it: asked about, and kept open over a keep that failed.
    keeping.mockRejectedValue(NO_ROOM);
    await openTheDraft();
    fireEvent.change(field(/^Message$/), {
      target: { value: "and the rest of it" },
    });

    fireEvent.click(screen.getByRole("button", { name: /^compose$/i }));
    expect(
      await screen.findByRole("alertdialog", { name: "Save this as a draft?" }),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Save as draft" }));

    expect((await screen.findByRole("alert")).textContent).toBe(
      "The app made a request the native core rejected",
    );
    expect(field(/^Message$/).value).toBe("and the rest of it");
    expect(field(/^To$/).value).toBe("kappa@example.net");
    expect(field(/^Subject$/).value).toBe("About Tuesday");

    // The question took the focus with it, so the window's own keys answer next.
    fireEvent.keyDown(window, { key: "c" });
    expect(
      await screen.findByRole("alertdialog", { name: "Save this as a draft?" }),
    ).toBeTruthy();
    expect(throwingAway).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));

    // Thrown away, and the new message in its place.
    await waitFor(() => expect(throwingAway).toHaveBeenCalledWith(4));
    await waitFor(() => expect(field(/^To$/).value).toBe(""));
    expect(field(/^Subject$/).value).toBe("");
    expect(field(/^Message$/).value).toBe("");

    // Asked for again over the new message, the key finds a new message already there.
    fireEvent.change(field(/^Subject$/), {
      target: { value: "a new thought" },
    });
    fireEvent.keyDown(window, { key: "c" });
    await act(async () => {});
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(field(/^Subject$/).value).toBe("a new thought");
  });

  it("waits for Escape to leave a draft being written by its own way out, and loses nothing to keeps the drafts refuse", async () => {
    // Escape takes the way out the cross takes. Closing the composer directly would lose everything
    // typed into the draft since its last kept version, and say nothing.
    keeping.mockRejectedValue(NO_ROOM);
    await openTheDraft();
    fireEvent.change(field(/^Message$/), {
      target: { value: "and the rest of it" },
    });

    // Pressed in the text, which is where the person is when they reach for it.
    fireEvent.keyDown(field(/^Message$/), { key: "Escape" });
    expect(
      await screen.findByRole("alertdialog", { name: "Save this as a draft?" }),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Save as draft" }));

    expect((await screen.findByRole("alert")).textContent).toBe(
      "The app made a request the native core rejected",
    );
    expect(field(/^Message$/).value).toBe("and the rest of it");
    expect(field(/^To$/).value).toBe("kappa@example.net");
    expect(field(/^Subject$/).value).toBe("About Tuesday");
  });

  it("lets go of the note of a draft the compose button leaves kept, and notes the new message under a name of its own", async () => {
    // Left for a new message, the draft stops being written: the core holds what the new one has
    // on screen, and nothing of the draft for the end of the application to write down again.
    keeping.mockResolvedValue(4);
    await openTheDraft();
    fireEvent.change(field(/^Message$/), {
      target: { value: "and the rest of it" },
    });
    expect(noting).toHaveBeenLastCalledWith(
      "acc-1",
      4,
      expect.objectContaining({ body: "and the rest of it" }),
    );
    const left = noting.mock.lastCall?.[2].writing;

    fireEvent.click(screen.getByRole("button", { name: /^compose$/i }));
    fireEvent.click(await screen.findByRole("button", { name: "Save as draft" }));
    await waitFor(() => expect(field(/^To$/).value).toBe(""));

    expect(forgetting).toHaveBeenCalledWith(left);
    fireEvent.change(field(/^Subject$/), {
      target: { value: "a new thought" },
    });
    expect(noting).toHaveBeenLastCalledWith(
      "acc-1",
      null,
      expect.objectContaining({ subject: "a new thought" }),
    );
    expect(noting.mock.lastCall?.[2].writing).not.toBe(left);
  });

  it("lets go of the note of a draft its key leaves thrown away, before the draft goes", async () => {
    keeping.mockResolvedValue(4);
    await openTheDraft();
    fireEvent.change(field(/^Message$/), {
      target: { value: "and the rest of it" },
    });
    const left = noting.mock.lastCall?.[2].writing;

    fireEvent.keyDown(window, { key: "c" });
    fireEvent.click(await screen.findByRole("button", { name: "Discard" }));
    await waitFor(() => expect(throwingAway).toHaveBeenCalledWith(4));
    await waitFor(() => expect(field(/^To$/).value).toBe(""));

    expect(forgetting).toHaveBeenCalledWith(left);
    expect(forgetting.mock.invocationCallOrder[0]).toBeLessThan(
      throwingAway.mock.invocationCallOrder[0] ?? Number.NaN,
    );
  });

  it("weighs a key pressed as a new message is drawn against that message, not the draft it replaced", async () => {
    // The window installs its keys again in an ordinary effect, after the drawing, so a key that
    // lands in between is heard by the listener installed while the draft was open. Weighed against
    // the draft, `c` would take the new message's own way out to open the message already open, and
    // the window would stay on its way out for good: every field refused, and every way out.
    //
    // Pressed here in the commit that draws the new message, from a layout effect drawn after the
    // window's: after everything the window publishes before the paint, before any of its ordinary
    // effects.
    let press: (key: string) => void = () => {};
    /** What To held as the key was pressed: empty is the new message's. */
    let pressedOver: string | undefined;
    function KeyBeforeTheEffects() {
      const [key, setKey] = useState<string | null>(null);
      press = setKey;
      useLayoutEffect(() => {
        if (key === null) return;
        pressedOver = (
          document.getElementById("compose-to") as HTMLInputElement | null
        )?.value;
        window.dispatchEvent(new KeyboardEvent("keydown", { key }));
      }, [key]);
      return null;
    }
    // Asked for in the turn that throws the draft away, so the key and the new message arrive in
    // one commit.
    throwingAway.mockImplementationOnce(async () => press("c"));
    await openTheDraft(ABOUT_TUESDAY, <KeyBeforeTheEffects />);

    fireEvent.click(screen.getByRole("button", { name: /^compose$/i }));
    fireEvent.click(await screen.findByRole("button", { name: "Discard" }));
    await waitFor(() => expect(pressedOver).toBe(""));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    expect(field(/^Subject$/).disabled).toBe(false);
    fireEvent.change(field(/^Subject$/), {
      target: { value: "a new thought" },
    });
    expect(field(/^Subject$/).value).toBe("a new thought");

    // And the cross still takes it away.
    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));
    fireEvent.click(await screen.findByRole("button", { name: "Discard" }));
    await waitFor(() =>
      expect(screen.queryByLabelText(/^Subject$/)).toBeNull(),
    );
  });

  it("leaves a draft opened with no To, Cc or subject where it is when a new message is asked for, whatever has been typed into it since", async () => {
    // Keyed as a new message is keyed, so a new message replaces nothing: nothing is asked, and
    // what the draft holds stays on screen.
    await openTheDraft({
      ...ABOUT_TUESDAY,
      recipients: "",
      subject: "",
      body: "just a body",
    });

    fireEvent.click(screen.getByRole("button", { name: /^compose$/i }));
    await act(async () => {});
    expect(screen.queryByRole("alertdialog")).toBeNull();
    fireEvent.keyDown(window, { key: "c" });
    await act(async () => {});
    expect(screen.queryByRole("alertdialog")).toBeNull();

    expect(field(/^Message$/).value).toBe("just a body");
    expect(throwingAway).not.toHaveBeenCalled();

    // The key is what the draft opened with, so a To and a subject typed into it since change
    // nothing either.
    fireEvent.change(field(/^To$/), { target: { value: "zeta@example.com" } });
    fireEvent.change(field(/^Subject$/), { target: { value: "Lunch" } });
    fireEvent.click(screen.getByRole("button", { name: /^compose$/i }));
    await act(async () => {});
    expect(screen.queryByRole("alertdialog")).toBeNull();
    fireEvent.keyDown(window, { key: "c" });
    await act(async () => {});
    expect(screen.queryByRole("alertdialog")).toBeNull();

    expect(field(/^To$/).value).toBe("zeta@example.com");
    expect(field(/^Subject$/).value).toBe("Lunch");
    expect(field(/^Message$/).value).toBe("just a body");
    expect(throwingAway).not.toHaveBeenCalled();
  });

  it("asks about a draft opened with a To and subject when a new message is asked for, even once both are cleared", async () => {
    // What it opened with decides, not what it holds. Cleared of both, the draft would otherwise
    // pass for a new message, and the text still in it would never be asked about.
    keeping.mockResolvedValue(4);
    await openTheDraft({ ...ABOUT_TUESDAY, body: "see you then" });
    fireEvent.change(field(/^To$/), { target: { value: "" } });
    fireEvent.change(field(/^Subject$/), { target: { value: "" } });

    fireEvent.click(screen.getByRole("button", { name: /^compose$/i }));
    expect(
      await screen.findByRole("alertdialog", { name: "Save this as a draft?" }),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Save as draft" }));

    // Kept under its own number as it stood, and the new message in its place.
    await waitFor(() => expect(field(/^Message$/).value).toBe(""));
    expect(keeping).toHaveBeenLastCalledWith(
      expect.anything(),
      4,
      expect.objectContaining({
        recipients: "",
        subject: "",
        body: "see you then",
      }),
    );
    expect(throwingAway).not.toHaveBeenCalled();
  });

  it("leaves a new message being written as it is when a new message is asked for", async () => {
    // The fields would be filled as they are, so there is nothing to replace, and a question
    // would be about leaving a message nobody asked to leave.
    overview.mockResolvedValue(ONE_ACCOUNT);
    keeping.mockRejectedValue(NO_ROOM);
    renderApp();
    const compose = await screen.findByRole("button", { name: /^compose$/i });
    await waitFor(() =>
      expect((compose as HTMLButtonElement).disabled).toBe(false),
    );
    fireEvent.click(compose);
    fireEvent.change(await screen.findByLabelText(/^Subject$/), {
      target: { value: "half a thought" },
    });

    fireEvent.click(compose);
    await act(async () => {});
    expect(screen.queryByRole("alertdialog")).toBeNull();
    fireEvent.keyDown(window, { key: "c" });
    await act(async () => {});
    expect(screen.queryByRole("alertdialog")).toBeNull();

    expect(field(/^Subject$/).value).toBe("half a thought");
  });
});

describe("reading, as the window does it", () => {
  const ARCHIVE = {
    folderId: "2",
    name: "Archive",
    role: "archive" as const,
    total: 1,
    unread: 0,
    draftsHere: 0,
    canReceive: true,
  };

  /** A row of the list, unread, under the subject given. */
  function row(messageId: string, folderId: string) {
    return {
      inConversation: 1,
      conversationId: null,
      messageId,
      folderId,
      stopped: null,
      subject: `About ${messageId}`,
      sender: "zeta@example.com",
      preview: null,
      receivedAt: 1_700_000_000,
      unread: true,
      leaving: false,
      arriving: false,
    };
  }

  // The list draws only the rows that fit its viewport, and this environment measures every
  // element as nothing: it is given a size here, and has it taken away again after.
  const measured = {
    observer: globalThis.ResizeObserver,
    rect: HTMLElement.prototype.getBoundingClientRect,
    scroll: HTMLElement.prototype.scrollTo,
    heights: (["clientHeight", "offsetHeight"] as const).map(
      (property) =>
        [
          property,
          Object.getOwnPropertyDescriptor(HTMLElement.prototype, property),
        ] as const,
    ),
  };
  beforeAll(() => {
    class MeasuringObserver {
      constructor(private readonly callback: ResizeObserverCallback) {}
      observe() {
        this.callback([], this as unknown as ResizeObserver);
      }
      unobserve() {}
      disconnect() {}
    }
    globalThis.ResizeObserver =
      MeasuringObserver as unknown as typeof ResizeObserver;
    for (const property of ["clientHeight", "offsetHeight"] as const) {
      Object.defineProperty(HTMLElement.prototype, property, {
        configurable: true,
        get: () => 600,
      });
    }
    HTMLElement.prototype.getBoundingClientRect = () =>
      ({
        width: 900,
        height: 600,
        top: 0,
        left: 0,
        right: 900,
        bottom: 600,
        x: 0,
        y: 0,
      }) as DOMRect;
    HTMLElement.prototype.scrollTo = () => {};
  });
  afterAll(() => {
    globalThis.ResizeObserver = measured.observer;
    HTMLElement.prototype.getBoundingClientRect = measured.rect;
    HTMLElement.prototype.scrollTo = measured.scroll;
    for (const [property, was] of measured.heights) {
      if (was) Object.defineProperty(HTMLElement.prototype, property, was);
      else
        delete (HTMLElement.prototype as unknown as Record<string, unknown>)[
          property
        ];
    }
  });

  beforeEach(() => {
    overview.mockResolvedValue(ONE_ACCOUNT);
    tree.mockResolvedValue(
      sidebar([{ ...ACCOUNT, folders: [INBOX, ARCHIVE] }]),
    );
    window_.mockImplementation(async (request) => {
      const folder =
        request.scope.kind === "folder" ? request.scope.folderId : "1";
      const rows =
        folder === "2" ? [row("a-1", "2")] : [row("m-1", "1"), row("m-2", "1")];
      return { rows, total: rows.length, viewEpoch: "1", notAnswering: [] };
    });
  });

  /** The message the reader shows for an identifier, under the subject its row has. */
  function details(messageId: string): MessageDetailsDto {
    return {
      messageId,
      subject: `About ${messageId}`,
      sender: "Zeta Quorin",
      senderAddress: "zeta@example.com",
      receivedAt: 1_700_000_000,
      size: 4096,
      unread: false,
      bodyState: "headersOnly",
      accountId: "acc-1",
      addressedToMore: false,
      named: null,
      chain: [],
      placements: [{ folderId: "1", folderName: "INBOX", folderRole: "inbox" }],
    };
  }

  /** The reader's own Archive, not the folder of that name or a row's button. */
  function theReadersArchive(): HTMLElement {
    const button = screen
      .getAllByRole("button", { name: "Archive" })
      .find(
        (one) => !one.closest("nav") && !one.closest("[role=listbox]"),
      );
    if (!button) throw new Error("the reader offers no Archive");
    return button;
  }

  /** The copy of the reader on its way out, while it is. */
  function leaving(): HTMLElement | null {
    return document.querySelector<HTMLElement>("[data-leaving]");
  }

  it("slides the message filed away out over the next one, which opens under it", async () => {
    reading.mockImplementation(async (messageId: string) => details(messageId));
    archiving.mockResolvedValue(undefined as never);
    renderApp();
    fireEvent.click(await screen.findByText("About m-1"));
    await screen.findByRole("heading", { name: "About m-1" });

    fireEvent.click(theReadersArchive());

    // The one being read leaves whole, as it was drawn, out of reach of anybody's keys or ears.
    await waitFor(() => expect(leaving()).not.toBeNull());
    const layer = leaving()!;
    expect(layer.textContent).toContain("About m-1");
    expect(layer.getAttribute("aria-hidden")).toBe("true");
    expect(layer.hasAttribute("inert")).toBe(true);
    const classes = layer.className.split(/\s+/);
    expect(classes).toContain("slide-out-to-right");
    expect(classes).toContain("rtl:slide-out-to-left");
    // None of it for somebody who asked for less motion, who is shown the next one at once.
    expect(classes).toContain("motion-reduce:hidden");

    // The next one opens under it, and is the one a reader is told about.
    expect(
      await screen.findByRole("heading", { name: "About m-2" }),
    ).toBeTruthy();
    expect(reading).toHaveBeenLastCalledWith("m-2");

    // Gone once it has slid out.
    await waitFor(() => expect(leaving()).toBeNull());
    expect(screen.getByRole("heading", { name: "About m-2" })).toBeTruthy();
  });

  it("opens on the message the core names as it starts, chosen in the list", async () => {
    // The build made for screenshots names one. Every other build names none, and the window
    // opens on nothing, which every other test here starts from.
    opening.mockResolvedValue({ messageId: "m-2", folderId: "1" });
    reading.mockImplementation(async (messageId: string) => details(messageId));
    renderApp();

    expect(
      await screen.findByRole("heading", { name: "About m-2" }),
    ).toBeTruthy();
    expect(reading).toHaveBeenCalledWith("m-2");
    await waitFor(() =>
      expect(
        screen
          .getAllByRole("option")
          .filter((row) => row.getAttribute("aria-selected") === "true")
          .map((row) => row.textContent),
      ).toEqual([expect.stringContaining("About m-2")]),
    );
  });

  it("leaves the message where it is when it is only marked read", async () => {
    reading.mockImplementation(async (messageId: string) => details(messageId));
    renderApp();

    fireEvent.click(await screen.findByText("About m-1"));
    await screen.findByText("Marked read");
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));

    expect(leaving()).toBeNull();
    expect(screen.getByRole("heading", { name: "About m-1" })).toBeTruthy();
  });

  it("leaves the reader as it is when a message other than the one being read is filed", async () => {
    reading.mockImplementation(async (messageId: string) => details(messageId));
    archiving.mockResolvedValue(undefined as never);
    renderApp();
    fireEvent.click(await screen.findByText("About m-1"));
    await screen.findByRole("heading", { name: "About m-1" });

    fireEvent.contextMenu(screen.getByText("About m-2"));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Archive" }));
    await waitFor(() => expect(archiving).toHaveBeenCalledWith("m-2", "1"));
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));

    expect(leaving()).toBeNull();
    expect(screen.getByRole("heading", { name: "About m-1" })).toBeTruthy();
  });

  it("does not answer the message on its way out", async () => {
    // It keeps what it was asked when it left. Handed the window's count as it goes on, a key
    // pressed while it slides would answer a message somebody has just filed away.
    reading.mockImplementation(async (messageId: string) => details(messageId));
    archiving.mockResolvedValue(undefined as never);
    answering.mockReturnValue(new Promise(() => {}));
    renderApp();
    fireEvent.click(await screen.findByText("About m-1"));
    await screen.findByRole("heading", { name: "About m-1" });

    fireEvent.click(theReadersArchive());
    await waitFor(() => expect(leaving()).not.toBeNull());
    fireEvent.keyDown(window, { key: "r" });
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));

    expect(answering).not.toHaveBeenCalledWith("m-1", expect.anything());
  });

  it("passes the message being read on by key, opening the composer with what it carries", async () => {
    reading.mockImplementation(async (messageId: string) => details(messageId));
    forwarding.mockResolvedValue({
      to: ["kappa@example.net"],
      cc: [],
      text: "Shall we?",
      files: [{ token: "t1", name: "report.pdf", bytes: "2048" }],
      leftBehind: [],
    });
    renderApp();
    fireEvent.click(await screen.findByText("About m-1"));
    await screen.findByRole("heading", { name: "About m-1" });

    fireEvent.keyDown(window, { key: "f" });

    const subject = await screen.findByLabelText(/^subject$/i);
    expect(forwarding).toHaveBeenCalledWith("m-1", false);
    expect((subject as HTMLInputElement).value).toBe("Fwd: About m-1");
    expect(screen.getByText("report.pdf")).toBeTruthy();
    expect((screen.getByLabelText(/^message$/i) as HTMLTextAreaElement).value)
      .toContain("---------- Forwarded message ----------");
    expect((screen.getByLabelText(/^to$/i) as HTMLInputElement).value).toBe("");
  });

  it("does not pass the message on, on its way out", async () => {
    // What it was asked to pass on is frozen as it leaves, as what it was asked to answer is.
    reading.mockImplementation(async (messageId: string) => details(messageId));
    archiving.mockResolvedValue(undefined as never);
    forwarding.mockReturnValue(new Promise(() => {}));
    renderApp();
    fireEvent.click(await screen.findByText("About m-1"));
    await screen.findByRole("heading", { name: "About m-1" });

    fireEvent.click(theReadersArchive());
    await waitFor(() => expect(leaving()).not.toBeNull());
    fireEvent.keyDown(window, { key: "f" });
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));

    expect(forwarding).not.toHaveBeenCalledWith("m-1", expect.anything());
  });

  it("answers everybody on the message being read by its letter", async () => {
    reading.mockImplementation(async (messageId: string) => details(messageId));
    answering.mockReturnValue(new Promise(() => {}));
    renderApp();
    fireEvent.click(await screen.findByText("About m-1"));
    await screen.findByRole("heading", { name: "About m-1" });

    fireEvent.keyDown(window, { key: "a", code: "KeyA" });

    await waitFor(() => expect(answering).toHaveBeenCalledWith("m-1", true));
  });

  it("answers by the chords mail applications use, the sender alone or everybody", async () => {
    reading.mockImplementation(async (messageId: string) => details(messageId));
    answering.mockReturnValue(new Promise(() => {}));
    renderApp();
    fireEvent.click(await screen.findByText("About m-1"));
    await screen.findByRole("heading", { name: "About m-1" });

    fireEvent.keyDown(window, {
      key: "R",
      code: "KeyR",
      metaKey: true,
      shiftKey: true,
    });

    await waitFor(() => expect(answering).toHaveBeenCalledWith("m-1", true));
    expect(answering).toHaveBeenCalledTimes(1);
  });

  it("answers the sender alone by the chord without Shift", async () => {
    reading.mockImplementation(async (messageId: string) => details(messageId));
    answering.mockReturnValue(new Promise(() => {}));
    renderApp();
    fireEvent.click(await screen.findByText("About m-1"));
    await screen.findByRole("heading", { name: "About m-1" });

    fireEvent.keyDown(window, { key: "r", code: "KeyR", ctrlKey: true });

    await waitFor(() => expect(answering).toHaveBeenCalledWith("m-1", false));
  });

  it("writes a new message by its chord", async () => {
    renderApp();
    await screen.findByText("About m-1");

    fireEvent.keyDown(window, { key: "n", code: "KeyN", metaKey: true });

    expect(await screen.findByLabelText(/^subject$/i)).toBeTruthy();
  });

  it("throws the message being read away by its chord", async () => {
    reading.mockImplementation(async (messageId: string) => details(messageId));
    deleting.mockReturnValue(new Promise(() => {}));
    renderApp();
    fireEvent.click(await screen.findByText("About m-1"));
    await screen.findByRole("heading", { name: "About m-1" });
    (document.activeElement as HTMLElement | null)?.blur();

    fireEvent.keyDown(window, {
      key: "Backspace",
      code: "Backspace",
      metaKey: true,
    });

    await waitFor(() => expect(deleting).toHaveBeenCalledWith("m-1", "1"));
    expect(deleting).toHaveBeenCalledTimes(1);
  });

  it("throws away once when the list answered the same chord itself", async () => {
    // The list throws its selection away on Delete and Backspace however they are held; the
    // window hearing the same key after it must not throw the message away a second time.
    reading.mockImplementation(async (messageId: string) => details(messageId));
    deleting.mockReturnValue(new Promise(() => {}));
    renderApp();
    fireEvent.click(await screen.findByText("About m-1"));
    await screen.findByRole("heading", { name: "About m-1" });
    const list = screen.getByRole("listbox");
    act(() => list.focus());

    fireEvent.keyDown(list, { key: "Backspace", code: "Backspace", metaKey: true });
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));

    expect(deleting).toHaveBeenCalledTimes(1);
  });

  it("leaves the chords to the text where somebody types", async () => {
    // Cmd+Backspace in a field deletes a line of what is typed, and Ctrl+R there is nothing of
    // the window's either.
    reading.mockImplementation(async (messageId: string) => details(messageId));
    renderApp();
    fireEvent.click(await screen.findByText("About m-1"));
    await screen.findByRole("heading", { name: "About m-1" });
    const field = screen.getByPlaceholderText("Search mail");
    field.focus();

    fireEvent.keyDown(field, { key: "Backspace", code: "Backspace", metaKey: true });
    fireEvent.keyDown(field, { key: "r", code: "KeyR", ctrlKey: true });
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));

    expect(deleting).not.toHaveBeenCalled();
    expect(answering).not.toHaveBeenCalled();
  });

  it("does not throw away or answer everybody on the message on its way out", async () => {
    // What it was asked when it left is frozen as it leaves, as the other keys' counts are. Filed
    // from the list rather than by the reader's own button, so the copy leaving is not one that
    // is already busy filing and would refuse anyway.
    reading.mockImplementation(async (messageId: string) => details(messageId));
    archiving.mockResolvedValue(undefined as never);
    deleting.mockReturnValue(new Promise(() => {}));
    answering.mockReturnValue(new Promise(() => {}));
    renderApp();
    fireEvent.click(await screen.findByText("About m-1"));
    await screen.findByRole("heading", { name: "About m-1" });

    fireEvent.contextMenu(screen.getAllByText("About m-1")[0]);
    fireEvent.click(await screen.findByRole("menuitem", { name: "Archive" }));
    await waitFor(() => expect(archiving).toHaveBeenCalledWith("m-1", "1"));
    await waitFor(() => expect(leaving()).not.toBeNull());
    (document.activeElement as HTMLElement | null)?.blur();
    fireEvent.keyDown(window, { key: "Backspace", code: "Backspace", metaKey: true });
    fireEvent.keyDown(window, { key: "a", code: "KeyA" });
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));

    expect(deleting).not.toHaveBeenCalledWith("m-1", expect.anything());
    expect(answering).not.toHaveBeenCalledWith("m-1", expect.anything());
  });

  it("opens nothing in a mailbox it has just come into", async () => {
    // Nothing is put in front of a person who has only come into a mailbox, and nothing is read
    // for them.
    renderApp();

    await screen.findByText("About m-1");
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));

    expect(reading).not.toHaveBeenCalled();
    expect(marking).not.toHaveBeenCalled();
    expect(screen.getByText("Select a message to read it")).toBeTruthy();
  });

  it("comes back to the message left open in a mailbox", async () => {
    renderApp();

    fireEvent.click(await screen.findByText("About m-2"));
    await waitFor(() => expect(reading).toHaveBeenLastCalledWith("m-2"));

    fireEvent.click(await screen.findByRole("button", { name: /^archive/i }));
    await screen.findByText("About a-1");
    expect(screen.getByText("Select a message to read it")).toBeTruthy();
    const read = reading.mock.calls.length;

    fireEvent.click(screen.getByRole("button", { name: /^inbox/i }));
    await waitFor(() =>
      expect(
        screen
          .getByText("About m-2")
          .closest("[role=option]")
          ?.getAttribute("aria-selected"),
      ).toBe("true"),
    );
    expect(reading.mock.calls.length).toBeGreaterThan(read);
    expect(reading).toHaveBeenLastCalledWith("m-2");
    expect(marking).toHaveBeenCalledTimes(1);
  });

  it("leaves the reader alone when the mailbox already chosen is chosen again", async () => {
    // A search is showing, and the reader holds one of its answers. The mailbox under the search
    // pressed again is not somewhere new, and the message left open there before the search is
    // not what the person is reading.
    renderApp();
    fireEvent.click(await screen.findByText("About m-2"));
    await waitFor(() => expect(reading).toHaveBeenLastCalledWith("m-2"));

    fireEvent.change(screen.getByRole("searchbox", { name: "Search mail" }), {
      target: { value: "About" },
    });
    await waitFor(() =>
      expect(
        window_.mock.calls.some(([asked]) => asked.scope.kind === "looking"),
      ).toBe(true),
    );
    fireEvent.click(await screen.findByText("About m-1"));
    await waitFor(() => expect(reading).toHaveBeenLastCalledWith("m-1"));

    fireEvent.click(screen.getByRole("button", { name: /^inbox/i }));
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));

    expect(reading).toHaveBeenLastCalledWith("m-1");
  });

  it("reads the counts again the moment a change is written down, and leaves the list", async () => {
    // The list already shows what the person did; the numbers beside the folders follow it at
    // once rather than waiting for a server.
    let asked = (_: unknown) => {};
    mailAsked.mockImplementation(async (handle) => {
      asked = handle as (change: unknown) => void;
      return () => {};
    });
    renderApp();
    await screen.findByText("About m-1");
    await waitFor(() => expect(mailAsked).toHaveBeenCalled());
    const counts = tree.mock.calls.length;
    const rows = window_.mock.calls.length;

    act(() => asked({ accountId: "acc-1" }));

    await waitFor(() => expect(tree.mock.calls.length).toBeGreaterThan(counts));
    expect(window_.mock.calls.length).toBe(rows);
  });

  it("brings the dot back the moment a message marked read is put back", async () => {
    // Marked the way the dot marks it, so the dot and the counts move in one frame. Marked from the
    // notice, the counts would follow the core's word while the dot waited for the write to land
    // and the list to read itself again.
    renderApp();

    fireEvent.click(await screen.findByText("About m-1"));
    await screen.findByText("Marked read");
    await waitFor(() =>
      expect(
        screen.getAllByRole("button", { name: "Mark as read" }),
      ).toHaveLength(1),
    );
    marking.mockImplementation(() => new Promise<number>(() => {}));

    fireEvent.click(screen.getByRole("button", { name: "Undo" }));

    await waitFor(() =>
      expect(marking).toHaveBeenLastCalledWith("m-1", "1", false),
    );
    expect(
      screen.getAllByRole("button", { name: "Mark as read" }),
    ).toHaveLength(2);
  });

  it("marks a message put back to unread once, whatever the list does after", async () => {
    // Handed to the list and left standing, it would be marked again by the list the next mailbox
    // draws -- a second instruction to the server, and the counts moved twice.
    renderApp();
    fireEvent.click(await screen.findByText("About m-1"));
    await screen.findByText("Marked read");
    marking.mockClear();

    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() =>
      expect(marking).toHaveBeenCalledWith("m-1", "1", false),
    );
    fireEvent.click(screen.getByRole("button", { name: /^archive/i }));
    await screen.findByText("About a-1");
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));

    expect(marking.mock.calls.filter(([, , read]) => !read)).toHaveLength(1);
  });

  it("reads the list again when a filing is taken back", async () => {
    // Carried out and read back before Undo was pressed, the row has already gone from the list's
    // own answer, and showing it again has nothing to show: only a fresh answer, with the way back
    // laid over it, brings the message back.
    deleting.mockResolvedValue(undefined);
    undoing.mockResolvedValue(true);
    renderApp();

    fireEvent.click(await screen.findByText("About m-1"));
    fireEvent.keyDown(screen.getByRole("listbox"), { key: "Delete" });
    await screen.findByText("Moved to the trash");
    const rows = window_.mock.calls.length;

    fireEvent.click(screen.getByRole("button", { name: "Undo" }));

    await waitFor(() => expect(undoing).toHaveBeenCalledWith("m-1", "1"));
    await waitFor(() =>
      expect(window_.mock.calls.length).toBeGreaterThan(rows),
    );
  });
});
