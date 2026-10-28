import type { NoticeDto } from "./generated/NoticeDto";
import type { OfferedDto } from "./generated/OfferedDto";
import type { WhatChangedDto } from "./generated/WhatChangedDto";
import type { InstalledDto } from "./generated/InstalledDto";
import type { BeingReadDto } from "./generated/BeingReadDto";
import type { HowFarDto } from "./generated/HowFarDto";
import type { AttachingDto } from "./generated/AttachingDto";
import type { NewMailNoticeDto } from "./generated/NewMailNoticeDto";
import type { NoticeShowsDto } from "./generated/NoticeShowsDto";
import type { ShowPicturesDto } from "./generated/ShowPicturesDto";
import type { LookForUpdatesDto } from "./generated/LookForUpdatesDto";
import type { PresetDto } from "./generated/PresetDto";
import type { HeldDto } from "./generated/HeldDto";
import type { FolderIdDto } from "@/shared/ipc/generated/FolderIdDto";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { MailboxTreeDto } from "./generated/MailboxTreeDto";
import type { AccountSettingsDto } from "./generated/AccountSettingsDto";
import type { CommandError } from "./generated/CommandError";
import type { AccountGroupDto } from "./generated/AccountGroupDto";
import type { AppIconDto } from "./generated/AppIconDto";
import type { OpeningDto } from "./generated/OpeningDto";
import type { PlacedDto } from "./generated/PlacedDto";
import type { SetAsideDto } from "@/shared/ipc/generated/SetAsideDto";
import type { FoundSettingsDto } from "@/shared/ipc/generated/FoundSettingsDto";
import type { ConnectionReportDto } from "./generated/ConnectionReportDto";
import type { EncryptionDto } from "./generated/EncryptionDto";
import type { MailAccountSummaryDto } from "./generated/MailAccountSummaryDto";
import type { MailAsked } from "./generated/MailAsked";
import type { MailChanged } from "./generated/MailChanged";
import type { DarkSchemeDto } from "@/shared/ipc/generated/DarkSchemeDto";
import type { HowThingsAreDto } from "@/shared/ipc/generated/HowThingsAreDto";
import type { LightSchemeDto } from "@/shared/ipc/generated/LightSchemeDto";
import type { DensityDto } from "./generated/DensityDto";
import type { LanguageDto } from "./generated/LanguageDto";
import type { PaneDto } from "./generated/PaneDto";
import type { PaneWidthsDto } from "./generated/PaneWidthsDto";
import type { PreferencesDto } from "./generated/PreferencesDto";
import type { SecretStoreStateDto } from "./generated/SecretStoreStateDto";
import type { BodyFormDto } from "./generated/BodyFormDto";
import type { LinkAsked } from "./generated/LinkAsked";
import type { AnswerDto } from "./generated/AnswerDto";
import type { ForwardDto } from "./generated/ForwardDto";
import type { DraftDto } from "./generated/DraftDto";
import type { KeptDraftDto } from "./generated/KeptDraftDto";
import type { SentHereDto } from "./generated/SentHereDto";
import type { OutgoingDto } from "./generated/OutgoingDto";
import type { SavedMessageDto } from "@/shared/ipc/generated/SavedMessageDto";
import type { MessageBodyDto } from "./generated/MessageBodyDto";
import type { MessageDetailsDto } from "./generated/MessageDetailsDto";
import type { SyncReportDto } from "./generated/SyncReportDto";
import type { ViewScopeDto } from "./generated/ViewScopeDto";
import type { ViewWindowDto } from "./generated/ViewWindowDto";
import type { ListRowDto } from "@/shared/ipc/generated/ListRowDto";
import type { ThemeDto } from "./generated/ThemeDto";
import type { WorkspaceOverviewDto } from "./generated/WorkspaceOverviewDto";

/**
 * The single place in the frontend that talks to the native core.
 *
 * Every command is wrapped here, so a rename in Rust surfaces in one file and errors keep their
 * typed shape.
 */

/** The core rejects with a `CommandError`; any other shape means the bridge itself failed. */
export function isCommandError(value: unknown): value is CommandError {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    typeof (value as { code: unknown }).code === "string"
  );
}

export function getWorkspaceOverview(): Promise<WorkspaceOverviewDto> {
  return invoke<WorkspaceOverviewDto>("get_workspace_overview");
}

export function getPreferences(): Promise<PreferencesDto> {
  return invoke<PreferencesDto>("get_preferences");
}

/** One preference per call. The core returns the full resulting state. */
export function setTheme(theme: ThemeDto): Promise<PreferencesDto> {
  return invoke<PreferencesDto>("set_preference", { change: { theme } });
}

export function setDensity(density: DensityDto): Promise<PreferencesDto> {
  return invoke<PreferencesDto>("set_preference", { change: { density } });
}

/** Which colours the interface wears when it is dark. */
export function setDarkScheme(
  darkScheme: DarkSchemeDto,
): Promise<PreferencesDto> {
  return invoke<PreferencesDto>("set_preference", { change: { darkScheme } });
}

/** Which colours the interface wears when it is light. */
export function setLightScheme(
  lightScheme: LightSchemeDto,
): Promise<PreferencesDto> {
  return invoke<PreferencesDto>("set_preference", { change: { lightScheme } });
}

/** Puts on another picture for the application, and keeps it. */
export function setAppIcon(appIcon: AppIconDto): Promise<PreferencesDto> {
  return invoke<PreferencesDto>("set_preference", { change: { appIcon } });
}

/**
 * Whether this system lets the running application change its picture: the Dock on macOS, the
 * window and its taskbar button on Windows. Where it cannot, the choice is not offered.
 */
export function appIconsWearable(): Promise<boolean> {
  return invoke<boolean>("app_icons_wearable");
}

/**
 * Marks a message read, or unread, in the folder the row stands for.
 *
 * Answers as soon as the intention is written down, not when the server has been told: the two are
 * different moments, and the point of writing it down is that a click works with no network. What
 * it answers with is where the write stands among everything asked for, which a read of the sidebar
 * answers with too.
 */
export function markMessageRead(
  messageId: string,
  folderId: FolderIdDto,
  read: boolean,
): Promise<number> {
  return invoke<number>("mark_message_read", { messageId, folderId, read });
}

/**
 * Files a message in another folder.
 *
 * Written down here and told to the server later, like everything else a person asks for.
 */
export function moveMessage(
  messageId: string,
  folderId: FolderIdDto,
  toFolderId: FolderIdDto,
): Promise<void> {
  return invoke("move_message", { messageId, folderId, toFolderId });
}

/**
 * Files a message where the account keeps mail it has finished with.
 *
 * Which folder that is, is the core's business: an interface that had to know would be wrong on the
 * day an account gains one.
 */
export function archiveMessage(
  messageId: string,
  folderId: FolderIdDto,
): Promise<void> {
  return invoke("archive_message", { messageId, folderId });
}

/**
 * Throws a message away, in the sense the person meant.
 *
 * `permanently` is the difference between something they can get back and something they cannot.
 */
/**
 * Throws away everything one folder holds, for good, and answers with how many were asked for.
 *
 * One instruction per message, queued the same way a single deletion is: nothing here is a second
 * path for the same act. There is no way back from these -- what the trash keeps is what a person
 * already decided to be rid of, and this is them saying it a second time.
 */
export function emptyFolder(folderId: FolderIdDto): Promise<number> {
  return invoke("empty_folder", { folderId });
}

export function deleteMessage(
  messageId: string,
  folderId: FolderIdDto,
  permanently = false,
): Promise<void> {
  return invoke("delete_message", { messageId, folderId, permanently });
}

/**
 * Takes back what was just asked for, while the server has not heard about it.
 *
 * Answers whether there was anything to take back: something already carried out is undone by
 * asking for the opposite instead.
 */
export function undoAction(
  messageId: string,
  folderId: FolderIdDto,
): Promise<boolean> {
  return invoke("undo_action", { messageId, folderId });
}

/**
 * Puts an instruction that stopped back in the queue.
 *
 * What a person does with the explanation on the row. The core starts it over rather than
 * continuing where it left off: asking again is a decision, not the next retry of an old one.
 */
export function tryActionAgain(
  messageId: string,
  folderId: FolderIdDto,
): Promise<void> {
  return invoke("try_action_again", { messageId, folderId });
}

/**
 * Whether the mail is current, and whether what was asked for has landed.
 *
 * Two questions in one answer, because they are the two a person has about a mail client that does
 * anything in the background.
 */
/**
 * How many mailboxes are being read from a server at this moment.
 *
 * Its own call because it is asked often and costs nothing: a number held in memory, no database.
 * `howThingsAre` walks several tables, so it is asked twice a minute — and a round lasts seconds,
 * which is why a line waiting on it never saw one.
 */
export function whatIsBeingRead(): Promise<BeingReadDto> {
  return invoke("what_is_being_read");
}

export function howThingsAre(): Promise<HowThingsAreDto> {
  return invoke<HowThingsAreDto>("how_things_are");
}

/** How wide each pane was left. `null` means the pane opens at its default width. */
export function getPaneWidths(): Promise<PaneWidthsDto> {
  return invoke<PaneWidthsDto>("get_pane_widths");
}

/** Stores how wide a pane was dragged. The core clamps it to a width that can be grabbed again. */
export function setPaneWidth(pane: PaneDto, width: number): Promise<void> {
  return invoke<void>("set_pane_width", { pane, width });
}

/**
 * Whether the sidebar was left folded, as the core read it before it made the window: the address
 * the window opens at says so, and the first frame is drawn from it. Nothing said is open.
 */
export function sidebarFoldedAtStart(): boolean {
  return new URLSearchParams(window.location.search).get("sidebar") === "folded";
}

/** Whether the sidebar was left folded into its rail. Nothing stored is open. */
export function getSidebarFolded(): Promise<boolean> {
  return invoke<boolean>("get_sidebar_folded");
}

export function setSidebarFolded(folded: boolean): Promise<void> {
  return invoke<void>("set_sidebar_folded", { folded });
}

/** `"system"` or a language tag. The core validates the shape. */
export function setLanguage(language: LanguageDto): Promise<PreferencesDto> {
  return invoke<PreferencesDto>("set_preference", { change: { language } });
}

/** Whether the system is told that mail arrived. */
export function setNewMailNotice(
  newMailNotice: NewMailNoticeDto,
): Promise<PreferencesDto> {
  return invoke<PreferencesDto>("set_preference", { change: { newMailNotice } });
}

/**
 * What a notification may say about the mail it announces.
 *
 * All three at once, because they are one answer to one question and because every change to a
 * preference comes back as the whole set — so the caller always holds the other two.
 */
export function setNoticeShows(
  noticeShows: NoticeShowsDto,
): Promise<PreferencesDto> {
  return invoke<PreferencesDto>("set_preference", { change: { noticeShows } });
}

/**
 * Opens the folder this run writes its journal into.
 *
 * No path is passed and none comes back: which folder it is belongs to the core, and a log nobody
 * can find is a log nobody sends.
 */
export function showWhereTheJournalWrites(): Promise<void> {
  return invoke("show_where_the_journal_writes");
}

/**
 * Opens the project's page for problems and questions in the system's browser. The address is the
 * core's: this window names nothing to open.
 */
export function openTheSupportPage(): Promise<void> {
  return invoke("open_the_support_page");
}

/**
 * The languages the person set on this system, in their order. The web view reports only the
 * first; the core asks the system for all of them.
 */
export function systemLanguages(): Promise<string[]> {
  return invoke<string[]>("system_languages");
}

/** The version of this copy, as it was built. */
export function thisVersion(): Promise<string> {
  return getVersion();
}

/** Whether a message fetches what it points at elsewhere without being asked each time. */
export function setShowPictures(
  showPictures: ShowPicturesDto,
): Promise<PreferencesDto> {
  return invoke<PreferencesDto>("set_preference", { change: { showPictures } });
}

/** Whether this copy shares anonymous reports about what fails in it. */
export function setShareAnonymousData(
  shareAnonymousData: boolean,
): Promise<PreferencesDto> {
  return invoke<PreferencesDto>("set_preference", {
    change: { shareAnonymousData },
  });
}

/** That the onboarding has been finished, so the window does not show it again. */
export function setWelcomed(welcomed: boolean): Promise<PreferencesDto> {
  return invoke<PreferencesDto>("set_preference", { change: { welcomed } });
}

/** Whether this copy looks for a newer version on its own, once a day. */
export function setLookForUpdates(
  lookForUpdates: LookForUpdatesDto,
): Promise<PreferencesDto> {
  return invoke<PreferencesDto>("set_preference", {
    change: { lookForUpdates },
  });
}

/**
 * Called when a background pass changed something: mail arrived, a flag moved, a message went.
 *
 * The core only speaks when there is something to say, so hearing this is reason enough to read
 * again. Answers with a function that stops listening.
 */
export function onMailChanged(
  handle: (change: MailChanged) => void,
): Promise<() => void> {
  return listen<MailChanged>("mail:changed", (event) => handle(event.payload));
}

/**
 * Called the moment a person's change is written down, before any server has heard of it.
 *
 * What the change looks like is already the answer every read gives, so this is the signal to read
 * the counts around the list again. The list itself already shows it. Answers with a function that
 * stops listening.
 */
export function onMailAsked(
  handle: (asked: MailAsked) => void,
): Promise<() => void> {
  return listen<MailAsked>("mail:asked", (event) => handle(event.payload));
}

/**
 * Whether a newer version is out.
 *
 * `aloud` is the difference between the two ways this is asked, and it is not cosmetic. On a timer
 * it is silent: a machine that is offline answers `null`, because nobody asked and there is nothing
 * to do about it. Pressing a button passes `true`, and then a failure comes back as one — a control
 * that reports nothing is a control that looks broken.
 *
 * `null` also covers a build with no signing key, which is every build made from a plain checkout.
 */
export function lookForAnUpdate(aloud: boolean): Promise<OfferedDto | null> {
  return invoke<OfferedDto | null>("look_for_an_update", { aloud });
}

/**
 * Brings the offered version down, checks its seal, and puts it in place.
 *
 * Resolves when the application has to be restarted to use what was installed. Progress arrives
 * through {@link onUpdateComing} rather than here: a download of tens of megabytes that says
 * nothing until it finishes is a dead control.
 */
export function installTheUpdate(): Promise<InstalledDto> {
  return invoke<InstalledDto>("install_the_update");
}

/** Told how much of an update has arrived, as it arrives. */
export function onUpdateComing(
  handle: (how: HowFarDto) => void,
): Promise<() => void> {
  return listen<HowFarDto>("update:coming", (event) => handle(event.payload));
}

/**
 * Told that the download is over and the swap has started.
 *
 * A different wait from the one before it, and the one where the application is about to be
 * replaced under somebody — so it gets its own word rather than a finished bar left on screen.
 */
export function onUpdatePutting(handle: () => void): Promise<() => void> {
  return listen<null>("update:putting", () => handle());
}

/**
 * What changed in the version now running, where nobody has read it yet.
 *
 * `null` on every ordinary start. It answers something exactly once: the first start after an
 * update this application installed itself.
 */
export function whatChanged(): Promise<WhatChangedDto | null> {
  return invoke<WhatChangedDto | null>("what_changed");
}

/**
 * Starts the application again, so the version just installed is the one running.
 *
 * Not a reload: the swap replaced the bundle on disk, and the process doing the asking is still the
 * old binary. Nothing comes back, because nothing returns.
 */
export function restartIntoTheNewVersion(): Promise<void> {
  return invoke<void>("restart_into_the_new_version");
}

/** Read, and therefore not shown again. Called by the cross that leaves the screen. */
export function changesSeen(): Promise<void> {
  return invoke<void>("changes_seen");
}

/**
 * Reads the sidebar: every account on this device and the folders under it, with how many
 * instructions a person had asked for when the counts were read.
 */
export function getMailboxTree(): Promise<MailboxTreeDto> {
  return invoke<MailboxTreeDto>("get_mailbox_tree");
}

/**
 * Reads one account: which mailboxes it has, then what is in each of them.
 *
 * Reading only: nothing is created, renamed, moved or removed, on the server or here.
 */
export function syncAccount(accountId: string): Promise<SyncReportDto> {
  return invoke<SyncReportDto>("sync_account", { accountId });
}

/** Folds an account's folders away, or brings them back. Remembered with the account. */
export function setAccountCollapsed(
  accountId: string,
  collapsed: boolean,
): Promise<void> {
  return invoke<void>("set_account_collapsed", { accountId, collapsed });
}

/**
 * Reads one window of the message list.
 *
 * `offset` and `limit` address positions in the list, so the virtualizer can jump anywhere without
 * the core holding a cursor on its behalf. `total` describes the whole set and `viewEpoch` changes
 * when that set changes.
 */
/**
 * The messages of the conversation a row stands for, oldest first.
 *
 * Asked by the message the row shows: a conversation has no identifier of its own that crosses.
 */
export function getConversation(messageId: string): Promise<ListRowDto[]> {
  return invoke<ListRowDto[]>("get_conversation", { messageId });
}

export function getViewWindow(request: {
  scope: ViewScopeDto;
  unreadOnly: boolean;
  offset: number;
  limit: number;
}): Promise<ViewWindowDto> {
  return invoke<ViewWindowDto>("get_view_window", request);
}

/**
 * Takes an account off this device: its mail, its secrets and what it was asked to do.
 *
 * There is nothing to undo afterwards and nothing to describe, so nothing comes back.
 */
export function removeAccount(accountId: string): Promise<void> {
  return invoke("remove_account", { accountId });
}

/** The settings of an account already here, to fill the form with. Never its password. */
export function getAccountSettings(
  accountId: string,
): Promise<AccountSettingsDto> {
  return invoke<AccountSettingsDto>("get_account_settings", { accountId });
}

/** The port a server is expected on, offered by the form when the encryption changes. */
export function defaultPortFor(encryption: EncryptionDto): Promise<number> {
  return invoke<number>("default_port_for", { encryption });
}

/** Starts the application again. Nothing comes back: the process is replaced. */
export function restartAfterSettingAside(): Promise<void> {
  return invoke("restart_after_setting_aside");
}

/**
 * Moves aside the databases that will not open, so the application can start.
 *
 * Nothing is deleted: every byte stays, under a name with the moment beside it, and what comes
 * back is where each file went. Restarting afterwards is the person's to do — a window rebuilt
 * under itself loses the one sentence they needed to read.
 */
export function setAsideLocalStorage(): Promise<SetAsideDto> {
  return invoke<SetAsideDto>("set_aside_local_storage");
}

/**
 * Looks for the servers behind an address, storing nothing.
 *
 * Asked for when a person presses the button and never while they type: a search on every
 * keystroke would send half-finished domains to whoever owns them, and a half-finished one is not
 * even this person's.
 *
 * An account already on this device at the same domain is the first answer, so a second mailbox
 * there is added without its servers being asked for again. `correcting` names the account a
 * screen is correcting, whose own servers are no answer about where its mail lives.
 */
export function findServerSettings(
  email: string,
  correcting?: string,
): Promise<FoundSettingsDto> {
  return invoke<FoundSettingsDto>("find_server_settings", {
    email,
    correcting: correcting ?? null,
  });
}

/**
 * Hands over the sentence a notification says, in the language this window is showing.
 *
 * Rust holds no interface text — every string lives in one catalogue — but a notification is
 * raised at a moment this window may be asleep. So it is handed over once, and again whenever the
 * language changes.
 */
export function setNotificationWords(newMail: string): Promise<void> {
  return invoke("set_notification_words", { newMail });
}

/**
 * Hands over the word the application menu's settings item says, in the language this window is
 * showing. Until it has been, the menu has no such item: Rust holds no interface text.
 */
export function setMenuWords(settings: string): Promise<void> {
  return invoke("set_menu_words", { settings });
}

/**
 * Called when the application menu's settings item is chosen. Answers with a function that stops
 * listening.
 */
export function onOpenSettings(handle: () => void): Promise<() => void> {
  return listen("settings:open", () => handle());
}

/**
 * Everything this application ships with, and what each of it is licensed under.
 *
 * Word for word, never summarised: a summary is not a licence. Grouped by the notice rather than
 * by the package, because sixty crates under one copyright line is one notice.
 */
export function licences(): Promise<NoticeDto[]> {
  return invoke<NoticeDto[]>("licences");
}

/**
 * The providers offered by name on the account screen, and the settings each fills in.
 *
 * Recognition and nothing else: pressing one fills in the servers and stops. Nothing here or
 * anywhere else branches on which was pressed.
 */
export function accountPresets(): Promise<PresetDto[]> {
  return invoke<PresetDto[]>("account_presets");
}

/**
 * What to tell somebody about their provider before they type a password, or nothing.
 *
 * A catalogue key, and a lookup in a table inside the application rather than a question anybody
 * is asked over a network — so it is safe to ask the moment an address is typed, which is the only
 * moment it is any use.
 */
export function accountHint(email: string): Promise<string | null> {
  return invoke<string | null>("account_hint", { email });
}

/**
 * How much of this device the mail on it is taking, per account, largest first.
 *
 * Nothing is cleared by asking. The number is what makes clearing a thing somebody can choose.
 */
export function whatIsHeld(): Promise<HeldDto[]> {
  return invoke<HeldDto[]>("what_is_held");
}

/**
 * The groups the sidebar is arranged into, each with the accounts in it.
 *
 * The accounts arrive as identifiers rather than as accounts: the sidebar already holds them, and
 * a second copy would give it two versions of the same unread count.
 */
export function listAccountGroups(): Promise<AccountGroupDto[]> {
  return invoke<AccountGroupDto[]>("list_account_groups");
}

/** Makes a group, empty and last, and answers with the identifier it was given. */
export function createAccountGroup(name: string): Promise<string> {
  return invoke<string>("create_account_group", { name });
}

export function renameAccountGroup(
  groupId: string,
  name: string,
): Promise<void> {
  return invoke("rename_account_group", { groupId, name });
}

/** Removes the group. The accounts in it take its place at the top of the sidebar. */
export function removeAccountGroup(groupId: string): Promise<void> {
  return invoke("remove_account_group", { groupId });
}

/**
 * Puts an account in a group, or — with `null` — at the top of the sidebar, directly below the group
 * it was in.
 *
 * One call rather than add-and-remove, because an account is in at most one group: there is no
 * moment where it is in both, and none where it is in neither by accident.
 */
/** Folds a group away, or opens it again. Kept beside the group, so it holds on any device. */
export function setAccountGroupCollapsed(
  groupId: string,
  collapsed: boolean,
): Promise<void> {
  return invoke("set_account_group_collapsed", { groupId, collapsed });
}

/**
 * Writes down the order accounts were dragged into.
 *
 * The whole run in the order it should be drawn, rather than one account and a place: what a place
 * means depends on every other account, and two windows dragging at once would each be right about
 * a list neither of them has.
 */
export function reorderAccounts(accountIds: string[]): Promise<void> {
  return invoke("reorder_accounts", { accountIds });
}

/**
 * Writes down the order of everything at the top of the sidebar: the groups and the accounts in no
 * group, as one run, for the reason `reorderAccounts` takes a whole run.
 */
export function arrangeSidebar(order: PlacedDto[]): Promise<void> {
  return invoke("arrange_sidebar", { order });
}

export function placeAccountInGroup(
  accountId: string,
  groupId: string | null,
): Promise<void> {
  return invoke("place_account_in_group", { accountId, groupId });
}

export function testAccountConnection(
  settings: AccountSettingsDto,
  password: string,
): Promise<ConnectionReportDto> {
  return invoke<ConnectionReportDto>("test_account_connection", {
    settings,
    password,
  });
}

/** Adds the account. The password goes to this device's secret store, never to the database. */
export function addAccount(
  settings: AccountSettingsDto,
  password: string,
): Promise<MailAccountSummaryDto> {
  return invoke<MailAccountSummaryDto>("add_account", { settings, password });
}

/**
 * Changes an account that is already here: where its mail is fetched from, and the password.
 *
 * Separate from {@link addAccount} because the two refuse opposite things — adding refuses an
 * address already stored, changing refuses one that is not. The identifier is kept: it names the
 * account's folders, its mail and its place in the operation log.
 */
export function changeAccount(
  accountId: string,
  settings: AccountSettingsDto,
  password: string,
): Promise<MailAccountSummaryDto> {
  return invoke<MailAccountSummaryDto>("change_account", {
    accountId,
    settings,
    password,
  });
}

/** Reads one message. `null` means it is no longer in the local index. */
/**
 * The message the window opens on, with the folder it is read in, or `null`: it is `null` in every
 * build but the one made for screenshots, whose scene may name one. Asked once, as the window starts.
 */
export function openingMessage(): Promise<OpeningDto | null> {
  return invoke<OpeningDto | null>("opening_message");
}

export function getMessage(
  messageId: string,
): Promise<MessageDetailsDto | null> {
  return invoke<MessageDetailsDto | null>("get_message", { messageId });
}

/**
 * Where this device keeps its secrets, and whether anything has to be done about it.
 *
 * `system` where this device's secrets are in the system's own store: macOS, Windows, and a Linux
 * desktop whose first start found a Secret Service. The other answers happen only on a Linux
 * desktop whose first start found none, and go on once a keyring appears.
 */
export function getSecretStoreState(): Promise<SecretStoreStateDto> {
  return invoke("get_secret_store_state");
}

/** Chooses the master password for the first time and makes the file. */
export function chooseMasterPassword(password: string): Promise<void> {
  return invoke("choose_master_password", { password });
}

/** Opens the file for this session. */
export function unlockSecrets(password: string): Promise<void> {
  return invoke("unlock_secrets", { password });
}

/**
 * Opens the body of a message and answers with the address the frame fetches it from.
 *
 * The body itself never comes back here: it is served on a scheme of its own, under a policy of its
 * own, to a frame that can do nothing with it. The address answers until this body is closed: when
 * the message is left, or a redraw replaces it.
 */
export function openMessageBody(
  messageId: string,
  form: BodyFormDto = "sent",
  pictures = false,
  surface: { background: string; foreground: string } | null = null,
): Promise<MessageBodyDto> {
  return invoke("open_message_body", {
    messageId,
    form,
    pictures,
    background: surface?.background ?? null,
    foreground: surface?.foreground ?? null,
  });
}

/**
 * The colours this window is wearing right now.
 *
 * Read from the document rather than from the stored settings, so it is whatever the stylesheet
 * actually resolved — the chosen dark scheme, or the light one, or what the system said when the
 * choice is to follow it. A message that says nothing about colour is shown on these, and looks
 * like part of the window instead of a black or white rectangle inside it.
 *
 * The core accepts them only if both are plainly colours, so a value from a stylesheet that says
 * something unexpected costs nothing: the message is then shown on white, which is what a message
 * with no colours of its own was written for.
 */
export function whatTheWindowWears(): {
  background: string;
  foreground: string;
} | null {
  if (typeof window === "undefined") return null;

  const style = window.getComputedStyle(document.documentElement);
  const background = style.getPropertyValue("--background").trim();
  const foreground = style.getPropertyValue("--foreground").trim();

  return background && foreground ? { background, foreground } : null;
}

/**
 * Asks which file to attach and keeps it until the message is sent.
 *
 * This window never names a file and never sees one: the system's own panel asks, and what comes
 * back is a name, a size and the token to send it by. `null` is the person closing the dialog.
 *
 * `beside` is the tokens of the files already on screen. A file that would make them heavier than
 * one message may be is refused with `fileTooLarge`, and nothing is kept.
 */
export function attachFile(beside: string[]): Promise<AttachingDto | null> {
  return invoke<AttachingDto | null>("attach_file", { beside });
}

/** Forgets a file attached and thought better of. */
export function forgetAttachment(token: string): Promise<void> {
  return invoke("forget_attachment", { token });
}

/**
 * Writes a message down so it can be sent.
 *
 * Nothing is sent here. The message is committed first — it is the one thing on this device that is
 * not a copy of something a server holds — and a round hands it over within a second or two. What
 * comes back is the name it will carry for ever, decided before the first attempt so that a server
 * which takes a message and loses the answer leaves something recognisable behind.
 *
 * The draft the message names as the one it was kept as is thrown away in the same step as the
 * outbox takes it, and the note its composer gave the core is let go of in that step too.
 */
export function sendMessage(
  accountId: string,
  message: OutgoingDto,
): Promise<string> {
  return invoke("send_message", { accountId, message });
}

/**
 * Writes down what is in the composer, under the number it was written down as before.
 *
 * Answers that number, so the next keep replaces this one rather than leaving a row behind every
 * keystroke.
 */
export function keepDraft(
  accountId: string,
  under: number | null,
  draft: DraftDto,
): Promise<number> {
  return invoke("keep_draft", { accountId, under, draft });
}

/**
 * Holds what is on screen in the composer, under the number it is kept under, for the end of the
 * application to write down.
 *
 * Quitting takes none of the composer's ways out, and once it has begun the window cannot be asked
 * anything: what was typed since the last keep is in the newest note or nowhere. Refused like a
 * keep when it holds more than a message may carry, and the note before it stays.
 */
export function noteDraft(
  accountId: string,
  under: number | null,
  draft: DraftDto,
): Promise<void> {
  return invoke("note_draft", { accountId, under, draft });
}

/**
 * Lets go of what the composer `writing` noted: the message was thrown away or kept on the way out,
 * the composer has gone, or it holds nothing a keep would write down. Another composer's note
 * stays. A message sent is let go of by the core, in the same step as the outbox takes it.
 */
export function forgetNotedDraft(writing: string): Promise<void> {
  return invoke("forget_noted_draft", { writing });
}

/**
 * Everything this account has written down and not sent, newest first.
 *
 * What the drafts folder shows. Drafts live on this device rather than on the server, so no
 * listing of mail will ever find them.
 */
export function keptDrafts(accountId: string): Promise<KeptDraftDto[]> {
  return invoke("kept_drafts", { accountId });
}

/**
 * What an account sent from this device and its folder for sent mail does not hold yet, newest
 * first: the folder shows these above what the server's copy of it holds.
 */
export function sentHere(accountId: string): Promise<SentHereDto[]> {
  return invoke("sent_here", { accountId });
}

/** The last thing this account was writing, if it was writing anything. */
export function latestDraft(accountId: string): Promise<KeptDraftDto | null> {
  return invoke("latest_draft", { accountId });
}

/**
 * The files a kept draft carries, attached again for the composer opening it.
 *
 * Each comes back as a file attached a moment ago does — a name, a size and a fresh token — so it
 * is listed, taken off and sent the same way. The bytes stay in the core.
 */
export function reopenDraft(id: number): Promise<AttachingDto[]> {
  return invoke("reopen_draft", { id });
}

/**
 * Forgets it: the message was thrown away. A message sent takes the draft it was kept as with it,
 * in the core.
 */
export function discardDraft(id: number): Promise<void> {
  return invoke("discard_draft", { id });
}

/**
 * Who an answer to this message goes to.
 *
 * Asked rather than worked out here: which addresses belong to the person — every one of them, not
 * only the account they are writing from — is a question with one answer and one place that knows
 * it. `everybody` is the difference between answering the writer and answering the room.
 */
export function answerAudience(
  messageId: string,
  everybody: boolean,
): Promise<AnswerDto> {
  return invoke("answer_audience", { messageId, everybody });
}

/**
 * What passing this message on starts with: who it was for, what it says, and its files — or, with
 * `whole`, the message itself as one file, an exact copy, and nothing else.
 *
 * The files are held in the core before this answers, each under a token, as a file attached from
 * the disk is; the bytes never cross. Whoever asked lets go of every token once nothing will show
 * it — when the composer given them goes, or at once if there is no composer to give them to.
 */
export function forwardMessage(
  messageId: string,
  whole: boolean,
): Promise<ForwardDto> {
  return invoke("forward_message", { messageId, whole });
}

/**
 * Saves one message as the file it arrived as.
 *
 * No path is sent and none comes back. The message is named; where the file goes is asked in the
 * system's own panel, in Rust, and what returns is the name it ended up with. `saved: false` is the
 * person closing that panel, which is an answer rather than a failure.
 */
/**
 * Saves one file that came with a message.
 *
 * Asked by naming the message and where the file sits in it. Never a path: where it goes is
 * decided in the system's own dialog, and a path from here is a path this window could choose.
 */
export function saveAttachment(
  messageId: string,
  at: number,
): Promise<SavedMessageDto> {
  return invoke("save_attachment", { messageId, at });
}

export function saveMessage(messageId: string): Promise<SavedMessageDto> {
  return invoke("save_message", { messageId });
}

/**
 * Forgets one body, by the name it was served under.
 *
 * Addressed rather than "close whatever is open": this call and the ones that open a body race
 * each other, and a close that arrives late would otherwise take the body that replaced the one it
 * meant.
 */
export function closeMessageBody(name: string): Promise<void> {
  return invoke("close_message_body", { name });
}

/**
 * Hears when a link in the open message was followed.
 *
 * The frame cannot open anything itself — following a link takes it to this application instead,
 * which asks. What arrives here is the host to show and the name to send back; the address is only
 * ever a thing to read.
 */
export function onMessageLink(
  handler: (link: LinkAsked) => void,
): Promise<() => void> {
  return listen<LinkAsked>("message:link", (event) => handler(event.payload));
}

/**
 * Hears when the frame asked for a picture of the open message and it was not there.
 *
 * The counts a reader is shown are worked out when the message is opened; the frame comes for the
 * pictures after that. A picture can therefore be counted as arrived and be gone by the time it is
 * wanted, and without this the message simply rendered with holes and said everything was well.
 */
export function onPictureMissing(handler: () => void): Promise<() => void> {
  return listen("message:picture-missing", () => handler());
}

/**
 * Hears when the key the databases are written under arrived after the start stopped waiting for
 * it.
 *
 * The system asks a person before it hands that key to a build it does not know, and they can
 * answer after the window came up saying the key could not be had. The databases are open by the
 * time this is heard, and everything the window read before then was read from nothing.
 */
export function onTheKeyArrived(handler: () => void): Promise<() => void> {
  return listen("storage:key-arrived", () => handler());
}

/** Opens a link the person said yes to, by the name it is known by. */
export function openLink(name: string): Promise<void> {
  return invoke("open_link", { name });
}

/**
 * Asks about an address in a message sent from here and not read back yet, by the text it takes up
 * in that message. The core finds it there again and answers as it does for a link that was
 * followed: with the name to send back and what to show.
 */
export function askAboutAWrittenLink(
  accountId: string,
  id: number,
  written: string,
): Promise<LinkAsked> {
  return invoke<LinkAsked>("ask_about_a_written_link", {
    accountId,
    id,
    written,
  });
}
