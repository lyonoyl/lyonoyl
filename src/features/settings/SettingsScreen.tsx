import type { MailAccountSummaryDto } from "@/shared/ipc/generated/MailAccountSummaryDto";
import { anchorFor } from "./sections";
import { inRoundNumbers } from "@/shared/ui/bytes";
import { AppIcons } from "@/shared/ui/AppIcons";
import { isDark, useSystemIsDark } from "@/shared/ui/darkness";
import { Checkbox } from "@/shared/ui/Checkbox";
import { Chooser } from "@/shared/ui/Chooser";
import { Button } from "@/shared/ui/primitives/button";
import { Check, Pencil } from "lucide-react";
import { useCallback, useEffect, useId, useState } from "react";

import {
  LANGUAGES,
  LANGUAGES_IN_LIST_ORDER,
  useI18n,
  type TranslationKey,
} from "@/shared/i18n";
import {
  licences,
  whatIsHeld,
  createAccountGroup,
  isCommandError,
  listAccountGroups,
  placeAccountInGroup,
  removeAccountGroup,
  renameAccountGroup,
  appIconsWearable,
  setAppIcon,
  setDarkScheme,
  setLightScheme,
  setNewMailNotice,
  setNoticeShows,
  setDensity,
  setLanguage,
  setShowPictures,
  setLookForUpdates,
  setShareAnonymousData,
  setTheme,
  showWhereTheJournalWrites,
  openTheSupportPage,
  thisVersion,
} from "@/shared/ipc/client";
import type { AccountGroupDto } from "@/shared/ipc/generated/AccountGroupDto";
import type { HeldDto } from "@/shared/ipc/generated/HeldDto";
import type { NoticeDto } from "@/shared/ipc/generated/NoticeDto";
import type { DarkSchemeDto } from "@/shared/ipc/generated/DarkSchemeDto";
import type { LightSchemeDto } from "@/shared/ipc/generated/LightSchemeDto";
import type { NewMailNoticeDto } from "@/shared/ipc/generated/NewMailNoticeDto";
import type { DensityDto } from "@/shared/ipc/generated/DensityDto";
import type { PreferencesDto } from "@/shared/ipc/generated/PreferencesDto";
import type { ShowPicturesDto } from "@/shared/ipc/generated/ShowPicturesDto";
import type { ThemeDto } from "@/shared/ipc/generated/ThemeDto";
import { typedDirection } from "@/shared/ui/direction";

/**
 * Everything this application lets a person decide about how it looks and reads.
 *
 * There is no Save button, and that is the design rather than an omission: every control writes
 * the moment it is used and the window changes with it, so what is on screen is always what is
 * stored. A screen that can be left in an unsaved state is a screen that can lie about what it is
 * showing.
 *
 * The core answers each write with the whole of the resulting settings, and that is what the
 * window then wears — so the interface never reconstructs the new state locally and cannot drift
 * from what actually reached the disk.
 */

const THEMES: ReadonlyArray<{ value: ThemeDto; key: TranslationKey }> = [
  { value: "system", key: "settings.themeSystem" },
  { value: "light", key: "settings.themeLight" },
  { value: "dark", key: "settings.themeDark" },
];

const DENSITIES: ReadonlyArray<{ value: DensityDto; key: TranslationKey }> = [
  { value: "comfortable", key: "settings.densityComfortable" },
  { value: "compact", key: "settings.densityCompact" },
];

const PICTURES: ReadonlyArray<{ value: ShowPicturesDto; key: TranslationKey }> =
  [
    { value: "whenAsked", key: "settings.picturesWhenAsked" },
    { value: "always", key: "settings.picturesAlways" },
  ];

/**
 * One palette as the settings screen offers it: shown by three of its own colours — its ground, its
 * accent and its text — rather than by its name alone.
 *
 * The swatches are written here as well as in the stylesheet on purpose: a person choosing colours
 * is choosing what they can see, and a preview that had to wait for the theme to change would make
 * them try each one to find out. The names are the ones these schemes are known by elsewhere.
 */
type Palette<T extends string> = {
  value: T;
  key: TranslationKey;
  swatch: [string, string, string];
};

/** The dark palettes. */
const DARK_SCHEMES: ReadonlyArray<Palette<DarkSchemeDto>> = [
  {
    value: "graphite",
    key: "settings.schemeGraphite",
    swatch: ["#252525", "#3f3f3f", "#e8e8e8"],
  },
  {
    value: "dracula",
    key: "settings.schemeDracula",
    swatch: ["#282a36", "#bd93f9", "#f8f8f2"],
  },
  {
    value: "nord",
    key: "settings.schemeNord",
    swatch: ["#2e3440", "#88c0d0", "#eceff4"],
  },
  {
    value: "solarizedDark",
    key: "settings.schemeSolarized",
    swatch: ["#002b36", "#268bd2", "#eee8d5"],
  },
  {
    value: "gruvboxDark",
    key: "settings.schemeGruvbox",
    swatch: ["#282828", "#fabd2f", "#ebdbb2"],
  },
  {
    value: "oneDark",
    key: "settings.schemeOneDark",
    swatch: ["#282c34", "#61afef", "#abb2bf"],
  },
];

/** The light palettes. */
const LIGHT_SCHEMES: ReadonlyArray<Palette<LightSchemeDto>> = [
  {
    value: "paper",
    key: "settings.schemePaper",
    swatch: ["#ffffff", "#e5e5e5", "#171717"],
  },
  {
    value: "solarizedLight",
    key: "settings.schemeSolarized",
    swatch: ["#fdf6e3", "#2074af", "#073642"],
  },
  {
    value: "gruvboxLight",
    key: "settings.schemeGruvbox",
    swatch: ["#fbf1c7", "#af3a03", "#3c3836"],
  },
  {
    value: "catppuccinLatte",
    key: "settings.schemeCatppuccinLatte",
    swatch: ["#eff1f5", "#8839ef", "#4c4f69"],
  },
  {
    value: "rosePineDawn",
    key: "settings.schemeRosePineDawn",
    swatch: ["#faf4ed", "#286983", "#464261"],
  },
];

export function SettingsScreen({
  preferences,
  onChanged,
  accounts,
  onOpenAccount,
  onArrangementChanged,
}: {
  preferences: PreferencesDto;
  /** Called with the whole of the settings as the core stored them. */
  onChanged: (preferences: PreferencesDto) => void;
  /** The mailboxes on this device, so there is somewhere to reach each of them from. */
  accounts: MailAccountSummaryDto[];
  /** Opens one account: how it is reached, and taking it off this device. */
  onOpenAccount: (accountId: string) => void;
  /** Called whenever the sidebar's arrangement changed, so it is drawn again. */
  onArrangementChanged?: () => void;
}) {
  const { t, language } = useI18n();
  /**
   * Whether the window wears its dark colours now: the dark palettes are offered only then, and the
   * light ones only otherwise. A palette offered on the side not worn would move a swatch when chosen
   * and nothing else on the screen, which reads as a control that does not work.
   */
  const darkNow = isDark(preferences.theme, useSystemIsDark());
  const [groups, setGroups] = useState<AccountGroupDto[]>([]);
  /**
   * What is held, or why not.
   *
   * Three states rather than two. `null` for both "not asked yet" and "the answer was no" made
   * the screen say "could not be read" for the moment before the answer arrived — on every
   * opening, about a store that was fine.
   */
  const [held, setHeld] = useState<HeldDto[] | "asking" | "refused">("asking");
  /** The notices, read only when somebody asks: four hundred kilobytes is not a thing to load
      into a settings screen nobody opened it for. */
  const [notices, setNotices] = useState<NoticeDto[] | null>(null);
  const [groupError, setGroupError] = useState<string | null>(null);
  /**
   * Whether the last move of a mailbox into a group was refused.
   *
   * Apart from the groups' own failure, because it is said where it happened: the chooser stands
   * in the mailbox's row, and a refusal said at the top of the groups above is out of sight of
   * somebody who scrolled down to the mailboxes -- who sees only a chooser that snapped back.
   */
  const [placementFailed, setPlacementFailed] = useState(false);
  /** Whether the last press opened the log folder. `null` until somebody presses. */
  const [theJournalOpened, setTheJournalOpened] = useState<boolean | null>(null);
  /** Whether the last press opened the issue tracker. `null` until somebody presses. */
  /** This copy's version, once the build has said. A copy that cannot say shows no line at all. */
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    let stillHere = true;
    thisVersion()
      .then((said) => stillHere && setVersion(said))
      .catch(() => {
        /* No version is a line not drawn, which is better than one that is wrong. */
      });
    return () => {
      stillHere = false;
    };
  }, []);
  /** Whether this system lets a running application change its picture, which is whether it is offered. */
  const [iconsWearable, setIconsWearable] = useState(false);
  const iconHeading = useId();
  useEffect(() => {
    let stillHere = true;
    appIconsWearable()
      .then((answer) => stillHere && setIconsWearable(answer))
      .catch(() => {
        /* Not offered: a system that cannot say is treated as one that cannot change it. */
      });
    return () => {
      stillHere = false;
    };
  }, []);
  const reload = useCallback(
    () =>
      listAccountGroups()
        .then((arrangement) => {
          setGroups(arrangement);
          setGroupError(null);
        })
        .catch((reason: unknown) => {
          setGroups([]);
          setGroupError(
            isCommandError(reason) ? reason.code : "storageUnavailable",
          );
        }),
    [],
  );

  useEffect(() => {
    void reload();
  }, [reload]);

  // Read once when the screen opens. Not kept fresh: the number moves as mail arrives, and a
  // figure that ticked upward while somebody read it would be harder to act on than one taken at
  // a moment.
  useEffect(() => {
    whatIsHeld()
      .then(setHeld)
      .catch(() => setHeld("refused"));
  }, []);

  /**
   * Runs one change, redraws the list here, and tells the sidebar to redraw itself.
   *
   * `about` is the part of the screen the change was asked from, which is where a refusal is said.
   */
  const change = useCallback(
    async (
      work: Promise<unknown>,
      about: "groups" | "mailboxes" = "groups",
    ) => {
      let failed = false;
      try {
        await work;
      } catch {
        failed = true;
      }

      // The listing is read again either way, and its own success used to wipe the failure the
      // write had just reported — so a refused rename flashed the wrong sentence for one round
      // trip and then said nothing at all.
      await reload();
      if (about === "mailboxes") setPlacementFailed(failed);
      else if (failed) setGroupError("notSaved");
      onArrangementChanged?.();

      return !failed;
    },
    [onArrangementChanged, reload],
  );

  /** Applies one change, and leaves the window as it was if the core could not store it. */
  function apply(change: Promise<PreferencesDto>) {
    change.then(onChanged).catch(() => {
      /* Nothing changed on disk and nothing changes on screen: the control the person clicked
         stays where it was, which is the honest answer to "it did not happen". */
    });
  }

  return (
    // Room after the last part, so it can be scrolled to the top of the pane like any other.
    // Without it the screen stops with the last few parts still halfway down, and the list beside
    // it never names them: pressing Language scrolled as far as the screen could go, which was
    // not far enough to put Language anywhere the list was looking. The blank is only ever seen
    // by somebody who has gone to the very end on purpose.
    <div
      data-settings-body
      className="mx-auto flex w-full max-w-2xl flex-col gap-10 p-8 pb-[50dvh]"
    >
      <header>
        <h1 className="text-lg font-semibold tracking-tight">
          {t("settings.title")}
        </h1>
        <p className="mt-1 text-sm text-muted-foreground">
          {t("settings.appliesAtOnce")}
        </p>
      </header>

      {/* Groups are made here rather than in the sidebar: the sidebar draws a group only once one
          exists, so the first one would have had nowhere to be made from. Above the mailboxes,
          because every mailbox row below offers the groups made here. */}
      <Section id="groups" title={t("settings.groups")}>
        <p className="text-xs text-muted-foreground">
          {t("settings.groupsHint")}
        </p>

        {groupError !== null && (
          <p role="alert" className="text-sm text-destructive">
            {t(
              groupError === "notSaved"
                ? "settings.groupsNotSaved"
                : "settings.groupsNotRead",
            )}
          </p>
        )}

        <NewGroup onCreate={(name) => change(createAccountGroup(name))} t={t} />

        {groups.length > 0 && (
          <ul className="flex flex-col gap-2">
            {groups.map((group) => (
              <GroupRow
                key={group.id}
                group={group}
                onRename={(name) => change(renameAccountGroup(group.id, name))}
                onRemove={() => change(removeAccountGroup(group.id))}
                t={t}
              />
            ))}
          </ul>
        )}
      </Section>

      {/* Here rather than beside the account in the sidebar: that row is only drawn when there is
          more than one account, so a person with a single mailbox would have had nowhere to reach
          this from at all. */}
      <Section id="accounts" title={t("settings.accounts")}>
        {placementFailed && (
          <p role="alert" className="text-sm text-destructive">
            {t("settings.groupsNotSaved")}
          </p>
        )}

        {accounts.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t("settings.noAccounts")}
          </p>
        ) : (
          <ul className="flex flex-col gap-2">
            {accounts.map((account) => {
              const name = account.displayName.trim() || account.email;
              return (
                <li
                  key={account.id}
                  className="flex items-center justify-between gap-4"
                >
                  <span
                    dir="auto"
                    className="min-w-0 truncate text-sm"
                    title={account.email}
                  >
                    {name}
                  </span>
                  <div className="flex shrink-0 items-center gap-2">
                    {/* In the row of the mailbox it is about. Offered only once a group exists: a
                        list whose one entry is "No group" is a control that cannot do anything.
                        The same width in every row, so the rows line up. */}
                    {groups.length > 0 && (
                      <Chooser
                        label={t("settings.groupOf", { account: name })}
                        size="sm"
                        className="w-40"
                        value={
                          groups.find((group) =>
                            group.accounts.includes(account.id),
                          )?.id ?? null
                        }
                        options={[
                          { value: null, label: t("settings.noGroup") },
                          ...groups.map((group) => ({
                            value: group.id,
                            label: group.name,
                            dir: "auto" as const,
                          })),
                        ]}
                        onChoose={(group) =>
                          void change(
                            placeAccountInGroup(account.id, group),
                            "mailboxes",
                          )
                        }
                      />
                    )}
                    {/* Which mailbox, in the name and in the hint. A column of pencils all
                        called "Edit" is a list a reader cannot choose in. */}
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-sm"
                      aria-label={t("settings.editAccount", { account: name })}
                      title={t("settings.editAccount", { account: name })}
                      onClick={() => onOpenAccount(account.id)}
                    >
                      <Pencil />
                    </Button>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </Section>

      <Section id="notifications" title={t("settings.notifications")}>
        <Choice
          label={t("settings.notifications")}
          hint={t("settings.notificationsHint")}
          options={[
            {
              value: "inbox",
              label: t("settings.noticeInbox"),
              chosen: preferences.newMailNotice === "inbox",
            },
            {
              value: "never",
              label: t("settings.noticeNever"),
              chosen: preferences.newMailNotice === "never",
            },
          ]}
          onChoose={(value) =>
            apply(setNewMailNotice(value as NewMailNoticeDto))
          }
        />

        {/* Offered only where there is a notification to say it. With notices off these three
            decide the wording of something that never appears, which is a control that does
            nothing dressed as one that does. */}
        {preferences.newMailNotice !== "never" && (
          <Switches
            label={t("settings.noticeShows")}
            hint={t("settings.noticeShowsHint")}
            parts={[
              {
                value: "sender",
                label: t("settings.noticeSender"),
                on: preferences.noticeShows.sender,
              },
              {
                value: "subject",
                label: t("settings.noticeSubject"),
                on: preferences.noticeShows.subject,
              },
              {
                value: "preview",
                label: t("settings.noticePreview"),
                on: preferences.noticeShows.preview,
              },
            ]}
            onToggle={(value) =>
              apply(
                setNoticeShows({
                  ...preferences.noticeShows,
                  [value]: !preferences.noticeShows[
                    value as keyof typeof preferences.noticeShows
                  ],
                }),
              )
            }
          />
        )}
      </Section>

      <Section id="storage" title={t("settings.storage")}>
        <p className="text-xs text-muted-foreground">
          {t("settings.storageHint")}
        </p>

        {held === "asking" ? (
          <p className="text-sm text-muted-foreground">
            {t("settings.asking")}
          </p>
        ) : held === "refused" ? (
          <p role="alert" className="text-sm text-destructive">
            {t("settings.storageNotRead")}
          </p>
        ) : held.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t("settings.noAccounts")}
          </p>
        ) : (
          <ul className="flex flex-col gap-2">
            {held.map((one) => (
              <li
                key={one.accountId}
                className="flex items-baseline justify-between gap-4"
              >
                <span
                  dir="auto"
                  className="min-w-0 truncate text-sm"
                  title={one.email}
                >
                  {one.displayName.trim() || one.email}
                </span>
                <span className="shrink-0 text-sm tabular-nums text-muted-foreground">
                  {inRoundNumbers(one.bytes, language, t)}
                </span>
              </li>
            ))}
            <li className="flex items-baseline justify-between gap-4 border-t border-border pt-2">
              <span className="text-sm font-medium">
                {t("settings.storageTotal")}
              </span>
              <span className="shrink-0 text-sm font-medium tabular-nums">
                {inRoundNumbers(
                  held
                    .reduce((sum, one) => sum + BigInt(one.bytes), 0n)
                    .toString(),
                  language,
                  t,
                )}
              </span>
            </li>
          </ul>
        )}

        <div className="flex items-baseline justify-between gap-4 border-t border-border pt-3">
          <div className="min-w-0">
            <p className="text-sm font-medium">{t("settings.theJournal")}</p>
            <p className="text-xs text-muted-foreground">
              {t("settings.theJournalHint")}
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            className="shrink-0"
            onClick={() => {
              setTheJournalOpened(true);
              void showWhereTheJournalWrites().catch(() =>
                setTheJournalOpened(false),
              );
            }}
          >
            {t("settings.showTheJournal")}
          </Button>
        </div>
        {/* A press that does nothing and says nothing is a dead control, and this one can fail:
            a system with nothing registered for a folder answers with a refusal and no window. */}
        {theJournalOpened === false && (
          <p role="alert" className="text-sm text-destructive">
            {t("settings.theJournalWouldNotOpen")}
          </p>
        )}

        {/* Beside the log, because the log is what a report needs. What to send and where is the
            project's page to say. */}
        <div className="flex items-baseline justify-between gap-4 border-t border-border pt-3">
          <p className="min-w-0 text-sm font-medium">{t("settings.report")}</p>
          <Button
            variant="outline"
            size="sm"
            className="shrink-0"
            onClick={() => void openTheSupportPage().catch(() => {})}
          >
            {t("settings.writeAReport")}
          </Button>
        </div>

        {/* Under the report: both are about what goes wrong. Off until somebody ticks it. */}
        <div className="border-t border-border pt-3">
          <Checkbox
            label={t("settings.shareAnonymousData")}
            hint={t("settings.shareAnonymousDataHint")}
            checked={preferences.shareAnonymousData}
            onChange={(share) => apply(setShareAnonymousData(share))}
          />
        </div>
      </Section>

      <Section id="appearance" title={t("settings.appearance")}>
        <Choice
          label={t("settings.theme")}
          hint={t("settings.themeHint")}
          options={THEMES.map((theme) => ({
            value: theme.value,
            label: t(theme.key),
            chosen: preferences.theme === theme.value,
          }))}
          onChoose={(value) => apply(setTheme(value as ThemeDto))}
        />

        {/* One block for both sides: turning from light to dark changes the palettes it offers,
            in the same place and the same height, so nothing on the screen moves. */}
        {darkNow ? (
          <Palettes
            label={t("settings.darkColours")}
            hint={t("settings.darkColoursHint")}
            schemes={DARK_SCHEMES}
            chosen={preferences.darkScheme}
            onChoose={(value) => apply(setDarkScheme(value))}
          />
        ) : (
          <Palettes
            label={t("settings.lightColours")}
            hint={t("settings.lightColoursHint")}
            schemes={LIGHT_SCHEMES}
            chosen={preferences.lightScheme}
            onChoose={(value) => apply(setLightScheme(value))}
          />
        )}

        {/* The picture the application wears: a choice made here is seen in the Dock at once.
            Offered only where the system lets it change. */}
        {iconsWearable && (
          <div>
            <p id={iconHeading} className="text-sm font-medium">
              {t("settings.appIcon")}
            </p>
            <div className="mt-3">
              <AppIcons
                chosen={preferences.appIcon}
                labelledBy={iconHeading}
                onChoose={(value) => apply(setAppIcon(value))}
              />
            </div>
          </div>
        )}
      </Section>

      <Section id="reading" title={t("settings.reading")}>
        <Choice
          label={t("settings.density")}
          hint={t("settings.densityHint")}
          options={DENSITIES.map((density) => ({
            value: density.value,
            label: t(density.key),
            chosen: preferences.density === density.value,
          }))}
          onChoose={(value) => apply(setDensity(value as DensityDto))}
        />

        <Choice
          label={t("settings.pictures")}
          hint={t("settings.picturesHint")}
          options={PICTURES.map((pictures) => ({
            value: pictures.value,
            label: t(pictures.key),
            chosen: preferences.showPictures === pictures.value,
          }))}
          onChoose={(value) => apply(setShowPictures(value as ShowPicturesDto))}
        />
      </Section>

      <Section id="language" title={t("settings.language")}>
        {/* A list rather than a row of buttons: every catalogue dropped into the build is one more
            language, and a row of fifteen wraps into a block nobody can scan. */}
        <div className="flex flex-col gap-2">
          <p className="text-sm font-medium">
            {t("settings.interfaceLanguage")}
          </p>
          <p className="text-xs text-muted-foreground">
            {t("settings.languageHint")}
          </p>
          <Chooser
            label={t("settings.interfaceLanguage")}
            className="mt-1 w-64"
            value={preferences.language}
            options={[
              { value: "system", label: t("settings.languageSystem") },
              // Each language names itself, so somebody who cannot read the current one can still
              // find their own, and the names stand in alphabetical order. Its code stands before
              // the name: `pt-BR` as PT, `zh-Hans` as ZH.
              ...LANGUAGES_IN_LIST_ORDER.map((tag) => ({
                value: tag,
                label: LANGUAGES[tag]?.name ?? tag,
                lang: tag,
                code: tag.split("-")[0],
              })),
            ]}
            onChoose={(value) => apply(setLanguage(value))}
          />
        </div>
      </Section>

      <Section id="updates" title={t("settings.updates")}>
        {version !== null && (
          <p className="text-sm">{t("settings.thisVersion", { version })}</p>
        )}
        {/* One box: looking once a day, or never. Nobody having been asked yet looks for nothing,
            so the box stands empty until somebody ticks it. */}
        <Checkbox
          label={t("settings.checkForUpdates")}
          hint={t("settings.checkForUpdatesHint")}
          checked={preferences.lookForUpdates === "onceADay"}
          onChange={(look) =>
            apply(setLookForUpdates(look ? "onceADay" : "never"))
          }
        />
      </Section>

      {/* Always the last part. It is a reference read once if ever, and nothing in it is a choice,
          so it stands below everything a person comes here to change. */}
      <Section id="licences" title={t("settings.licences")}>
        <p className="text-xs text-muted-foreground">
          {t("settings.licencesHint")}
        </p>

        {notices === null ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="self-start"
            onClick={() => {
              void licences()
                .then(setNotices)
                .catch(() => setNotices([]));
            }}
          >
            {t("settings.licencesShow")}
          </Button>
        ) : (
          <>
            <p className="text-sm text-muted-foreground">
              {t("settings.licencesCount", {
                // Packages, not places a package is named: one under two licences is one.
                packages: new Set(notices.flatMap((notice) => notice.used)).size,
                notices: notices.length,
              })}
            </p>
            <ul className="flex max-h-96 flex-col gap-4 overflow-y-auto">
              {notices.map((notice, at) => (
                // Left to right in every window: the licences are shown in English, as written.
                <li key={at} dir="ltr" className="flex flex-col gap-1">
                  <p className="text-sm font-medium">{notice.licence}</p>
                  <p className="text-xs text-muted-foreground">
                    {notice.used.join(", ")}
                  </p>
                  {/* Word for word. A licence shown with its line breaks taken out is a licence
                      somebody has to guess at, and none of them asked to be tidied. */}
                  <pre className="overflow-x-auto rounded-md border border-border bg-muted p-3 text-[0.6875rem] leading-snug whitespace-pre-wrap">
                    {notice.text}
                  </pre>
                </li>
              ))}
            </ul>
          </>
        )}
      </Section>
    </div>
  );
}

/** Making a group: a name and a button, refusing an empty one before anything is asked of the core. */
function NewGroup({
  onCreate,
  t,
}: {
  onCreate: (name: string) => Promise<boolean>;
  t: (key: TranslationKey, values?: Record<string, string | number>) => string;
}) {
  const [typed, setTyped] = useState("");
  const ready = typed.trim() !== "";

  return (
    <form
      className="flex items-center gap-2"
      onSubmit={(event) => {
        event.preventDefault();
        if (!ready) return;
        // Emptied only once the group exists. Cleared on the press, a name the core refused was
        // gone before anybody was told it had been refused, and had to be typed again from memory.
        void onCreate(typed).then((made) => {
          if (made) setTyped("");
        });
      }}
    >
      <input
        type="text"
        dir={typedDirection(typed)}
        value={typed}
        onChange={(event) => setTyped(event.target.value)}
        placeholder={t("settings.newGroupPlaceholder")}
        aria-label={t("settings.newGroup")}
        maxLength={64}
        className="min-w-0 flex-1 rounded-md border border-border bg-transparent px-2 py-1 text-sm"
      />
      <Button type="submit" variant="outline" size="sm" disabled={!ready}>
        {t("settings.newGroup")}
      </Button>
    </form>
  );
}

/**
 * One group: its name, editable in place, and the button that removes it.
 *
 * The name is written down when the field is left or Enter is pressed, rather than on every
 * keystroke: a rename per letter would be a row of half-typed names in the sidebar beside it.
 */
function GroupRow({
  group,
  onRename,
  onRemove,
  t,
}: {
  group: AccountGroupDto;
  onRename: (name: string) => void;
  onRemove: () => void;
  t: (key: TranslationKey, values?: Record<string, string | number>) => string;
}) {
  const [typed, setTyped] = useState(group.name);

  // The stored name wins when it changes underneath — a rename that the core refused must not
  // leave the field showing what was refused.
  //
  // Compared while rendering rather than in an effect, and that is not a preference. An effect
  // runs after the browser has been painted, so between the paint and the effect there is a window
  // in which the field is on screen and the effect's write is still owed — and that write is
  // unconditional on the first run. A letter typed inside that window is replaced by the stored
  // name a moment later, and `commit` then sees a name equal to the stored one and files nothing:
  // the rename disappears with nothing on screen to say so. The window is short and the row opens
  // straight into it, because this row appears in the very commit that finishes loading the
  // groups, with no later commit to force the effect out first.
  const [stored, setStored] = useState(group.name);
  if (stored !== group.name) {
    setStored(group.name);
    setTyped(group.name);
  }

  function commit() {
    const wanted = typed.trim();
    if (wanted === "" || wanted === group.name) {
      setTyped(group.name);
      return;
    }
    onRename(wanted);
  }

  return (
    <li className="flex items-center justify-between gap-4">
      <input
        type="text"
        dir={typedDirection(typed)}
        value={typed}
        aria-label={t("settings.groupName")}
        maxLength={64}
        onChange={(event) => setTyped(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") event.currentTarget.blur();
          // Only when there is an edit to give up. Taken unconditionally, Escape in a name nobody
          // has touched would do nothing and stop the window leaving the screen — a dead key in
          // one field of one screen, which is worse than either thing it could have done.
          if (event.key === "Escape" && typed !== group.name) {
            event.stopPropagation();
            setTyped(group.name);
          }
        }}
        className="min-w-0 flex-1 rounded-md border border-transparent bg-transparent px-2 py-1 text-sm hover:border-border focus:border-border"
      />
      <Button
        type="button"
        variant="outline"
        size="sm"
        // Which group. A column of buttons all called "Remove" is a list a reader cannot choose in.
        aria-label={t("settings.removeGroupNamed", { group: group.name })}
        onClick={onRemove}
      >
        {t("settings.removeGroup")}
      </Button>
    </li>
  );
}

function Section({
  id,
  title,
  children,
}: {
  /** Its name in `SETTINGS_SECTIONS`, which is what the list beside the screen scrolls to. */
  id: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section
      id={anchorFor(id)}
      aria-label={title}
      // A press in the list beside this screen lands the part a third of the way down rather than
      // hard against the top edge. Flush with the top, a part arrives with nothing above it and
      // nothing to say where it came from; a third of the way down it arrives somewhere, with
      // what precedes it still in view.
      className="flex scroll-mt-[33vh] flex-col gap-6"
    >
      {/* 17px against the 14px of the text under it. These are what a person scans down the
          screen for, and at the same size as everything else they were not headings at all. */}
      <h2 className="text-[1.0625rem] font-semibold tracking-tight text-muted-foreground">
        {title}
      </h2>
      {children}
    </section>
  );
}

/**
 * Several settings that are one question, each turned on or off by itself.
 *
 * Not a [`Choice`]: those are one answer out of several and these are three answers at once, so
 * `aria-pressed` says what each is rather than a radio group saying which one won.
 */
function Switches({
  label,
  hint,
  parts,
  onToggle,
}: {
  label: string;
  hint: string;
  parts: ReadonlyArray<{ value: string; label: string; on: boolean }>;
  onToggle: (value: string) => void;
}) {
  const heading = useId();

  return (
    <div role="group" aria-labelledby={heading} className="mt-4">
      <p id={heading} className="text-sm font-medium">
        {label}
      </p>
      <p className="text-xs text-muted-foreground">{hint}</p>

      <div className="mt-3 flex flex-wrap gap-2">
        {parts.map((part) => (
          <button
            key={part.value}
            type="button"
            aria-pressed={part.on}
            onClick={() => onToggle(part.value)}
            className={`rounded-md border px-3 py-1.5 text-sm transition-colors ${
              part.on
                ? "border-ring bg-muted font-medium"
                : "border-border text-muted-foreground hover:bg-muted/60 hover:text-foreground"
            }`}
          >
            {part.label}
          </button>
        ))}
      </div>
    </div>
  );
}

/**
 * The palettes of the side the window wears.
 *
 * One element for both sides, whichever palettes it holds: turning from light to dark changes its
 * heading and its palettes and leaves it where it stands. Both sides fill the same number of rows,
 * two to a row or three, so its height does not change either. A scheme chosen is then a scheme
 * seen, and the theme is left as it was: following the system stays following it.
 */
function Palettes<T extends string>({
  label,
  hint,
  schemes,
  chosen,
  onChoose,
}: {
  label: string;
  hint: string;
  schemes: ReadonlyArray<Palette<T>>;
  chosen: T;
  onChoose: (value: T) => void;
}) {
  const { t } = useI18n();
  const heading = useId();

  return (
    <div role="group" aria-labelledby={heading}>
      <p id={heading} className="text-sm font-medium">
        {label}
      </p>
      <p className="text-xs text-muted-foreground">{hint}</p>

      <div className="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-3">
        {schemes.map((scheme) => (
          <button
            key={scheme.value}
            type="button"
            aria-pressed={chosen === scheme.value}
            onClick={() => onChoose(scheme.value)}
            className={`flex items-center gap-3 rounded-md border p-2 text-start text-sm transition-colors ${
              chosen === scheme.value
                ? "border-ring bg-muted"
                : "border-border hover:bg-muted/60"
            }`}
          >
            <span
              aria-hidden="true"
              className="flex size-8 shrink-0 overflow-hidden rounded-md border border-border/60"
            >
              {scheme.swatch.map((colour) => (
                <span
                  key={colour}
                  className="flex-1"
                  style={{ backgroundColor: colour }}
                />
              ))}
            </span>
            <span className="min-w-0 flex-1 truncate">{t(scheme.key)}</span>
            {chosen === scheme.value && (
              <Check
                aria-hidden="true"
                className="size-4 shrink-0 text-muted-foreground"
              />
            )}
          </button>
        ))}
      </div>
    </div>
  );
}

/** One setting, offered as the whole of its choices at once rather than hidden behind a menu. */
function Choice({
  label,
  hint,
  options,
  onChoose,
}: {
  label: string;
  hint: string;
  options: ReadonlyArray<{ value: string; label: string; chosen: boolean }>;
  onChoose: (value: string) => void;
}) {
  const heading = useId();

  // A group by its role rather than a fieldset, here and in every group on this screen: WebKit
  // counts a legend's height twice in its fieldset when the screen is first drawn and drops it at
  // the next change inside, so the group shrinks under the eyes and everything below it moves.
  return (
    <div role="group" aria-labelledby={heading}>
      <p id={heading} className="text-sm font-medium">
        {label}
      </p>
      <p className="text-xs text-muted-foreground">{hint}</p>

      <div className="mt-3 flex flex-wrap gap-2">
        {options.map((option) => (
          <button
            key={option.value}
            type="button"
            aria-pressed={option.chosen}
            onClick={() => onChoose(option.value)}
            className={`rounded-md border px-3 py-1.5 text-sm transition-colors ${
              option.chosen
                ? "border-ring bg-muted font-medium"
                : "border-border text-muted-foreground hover:bg-muted/60 hover:text-foreground"
            }`}
          >
            {option.label}
          </button>
        ))}
      </div>
    </div>
  );
}
