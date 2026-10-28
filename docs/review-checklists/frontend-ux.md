# Frontend, UX, and responsiveness review

Applies to screens, the message list, virtualization, keyboard navigation, design tokens, and any change affecting perceived speed.

Read first: [the response budgets](../performance/response-budgets.md).

Two guiding questions: **does this feel fast**, and **can it be operated by keyboard and by a screen reader?**

## Checklist

1. **Budgets.** Does the change break any response budget? Does a spinner appear before 250 ms? Does the network block the UI anywhere?
2. **Optimism.** Does the user wait for the server to see the result of archive/delete/flag/read? Is anything rolled back silently?
3. **Virtualization.** Is row height fixed per density mode? Is scroll position restored by index rather than by pixels? Does range selection work across not-yet-loaded data? Does a `view_epoch` change reload the whole list instead of re-anchoring on the topmost visible row?
4. **Keyboard.** Is `aria-activedescendant` used (roving tabindex is physically impossible under virtualization—the focused row unmounts)? Is any action reachable by mouse only?
5. **IPC.** Is there an `invoke` or `listen` outside `src/shared/ipc/`? Is there a hand-written TS DTO instead of a generated one?
6. **Styles.** Has a rule appeared outside `@layer`? A hard-coded color instead of a token? An edit to a file in `shared/ui/primitives/` instead of a wrapper?
7. **Accessibility beyond the keyboard.** What happens at 200% system scaling, in forced-colors (Windows high contrast) and with Increase Contrast on macOS? A dense UI with fixed row height breaks first, and a meaning carried by color alone is lost there.
8. **Screen states.** Are all four defined: empty, loading, error, offline? Is the empty state different for "this folder is empty," "nothing unread here," and "nothing here matches that"?
9. **Multi-account.** Is there an account marker on the row in unified view, and is the correct From preselected? "Replied from the wrong address" is irreversible.
10. **Interface text.** Is every string a person reads in every catalog in `src/shared/i18n/locales/`, added, reworded, or removed there in the same commit, and read through `t()`? Is a sentence built by concatenating fragments—which no translator can reorder? Does a count use a plural entry rather than `n + " items"`? Is a failure rendered from `CommandError.code`, never from its `message`? Does the layout survive text ~40% longer than English, and a number formatted with a different group separator? Does every new or changed string read as section 8 of [the working contract](../../AGENTS.md) asks—true, one reading, somebody acting, the reader's words, native rather than translated, never clipped—and was every language measured in the real window at 960 × 640, with the box sized to the text?
11. **Leaving a screen, and the pointer.** Does the pane's cross call what the screen itself does on the way out, at the same cost as every other route? Is there a hand cursor anywhere but on a link in the message frame?

## Reporting

Cite file and line for every finding, most severe first. For each, give a concrete failure scenario—inputs and the resulting behavior—and the minimal fix. If nothing is found, say so; do not pad the list.
