# Interface text

Everything a person reads in the interface lives here. Nothing outside this directory holds copy, and nothing outside it imports a catalog directly—`scripts/check-layering.sh` fails a build that tries.

```
locales/en.json     the source language: keys are defined here
locales/*.json      one catalogue per language, each complete
catalog.ts          builds the registry from locales/*.json — no per-language code anywhere
translate.ts        t(): lookup, {placeholders}, plural forms, fallback chain; listOf(): a list of names
resolve.ts          which catalog to render: stored choice → the system's languages → English
errors.ts           ErrorCode from the native core → a key in the catalog
context.tsx         <I18nProvider> and useTranslation()
```

## Which language a window speaks

A choice made in the settings comes first. Without one the window follows the system: the core reads the person's languages in the order they set them (`system_languages`, from the system on all three platforms), and the first of them with a catalog wins. The web view's own answer is used until the core's arrives, and it is not enough on its own—on macOS it is the first language and nothing after it, so a person whose first language has no catalog would see English rather than their second.

A tag matches its own catalog, or one in the same language **written in the same script**: `pt-PT` takes `pt-BR`, `en-GB` takes `en`, `zh-CN` takes `zh-Hans`—and `zh-TW` does not, because Traditional and Simplified Chinese are different characters. The script is the one `Intl.Locale` fills in where a tag leaves it out. English is where every chain ends.

The settings list every language under the name it gives itself, with its code before the name, in the order of those names—Unicode's root collation: Latin names from A to Z, then the other scripts each in their own order.

## Adding a language

Copy `locales/en.json` to `locales/<tag>.json`, translate the values, and put its `$name` in its place in the order `catalog.test.ts` writes out. The README shows a flag for every catalog: add the language to `LANGUAGE_FLAGS` in `scripts/check-readme.py`, with its flag and English name, and its flag to the README's line of languages in the order of the English names, or `scripts/check-layering.sh` refuses the change. `<tag>` is a BCP-47 tag: `de`, `pt-BR`, `zh-Hans`.

Two keys in the file are not text but metadata:

- `$name`—the language's name **in itself** (`Deutsch`, not `German`), as the Unicode locale data gives it and capitalized where its letters have case. Bahasa Indonesia is the one exception the test knows of.
- `$dir`—`ltr` or `rtl`, the direction its script runs. `rtl` mirrors the whole window: logical sides, mirrored arrows, columns that resize, and arrow keys that move the other way, and the panes beside the sidebar kept clear of the window's buttons on macOS.

**A catalog is not finished when it is written.** Arbiters who read the language as natives and did not write it go through every line—one for meaning against the English and the place it is shown, one for how it reads—and what they find is settled before it lands ([the working contract](../../../AGENTS.md), invariant 18). The same goes for every string added or changed later, in every language at once.

Then run `npm test`. `catalog.test.ts` fails by name on:

- a key missing, a key English no longer has, blank text, a key nothing in the code displays;
- a placeholder dropped, renamed, or invented;
- a plural category the language selects and the file leaves out, or one it never selects;
- a plural form standing for more than one number that does not say `{count}`—Arabic `few` is 3 to 10 and 103 to 110, and French, Portuguese, Hindi, and Bengali `one` is 0 and 1;
- a period at the very end, in any script (`.` `。` `।` `۔`), or an exclamation mark anywhere;
- a count written as one line where English writes it in forms, or the other way round;
- in a language not written in Latin letters, a Latin word other than the names everybody writes in Latin—the product, the protocols and services, the names printed on keys, and the units of a size—or a line with letters and none of its own script;
- in a language written in Latin letters, a line identical to English that no arbiter has written down in `SAME_AS_ENGLISH` as the same word in that language, key names and units included; only the names nobody translates, in `SAME_IN_EVERY_LANGUAGE`, are the same everywhere without it;
- a `$name` that is not the language's own name, a `$dir` that is not its script's direction, and a list of languages out of the order of their names.

## Writing text

- **Never build a sentence from fragments, and never join two.** `t("a") + name + t("b")` cannot be reordered by a translator, and word order is exactly what differs between languages. Use one key with a placeholder: `"{name} replied"`. Two sentences shown together are kept apart—as two elements, or on one line with a separator between them—since no line ends with a stop and run together they read as one. A list of names is joined with `listOf(language, names)`: the language's own way of joining a list, with each name isolated first where the language runs right to left. `Intl.ListFormat` is not called directly—handed to `t()` whole, a list that opens with a Latin name would run left to right with every conjunction beside the wrong name.
- **A value keeps its own direction.** In a catalog written right to left, `t()` isolates every value it puts into a sentence, so a name or an address in Latin letters keeps its order and the punctuation around it stays where the sentence put it.
- **Counts use plural forms**, never `count + " messages"`. English needs two forms, Japanese one, Arabic six—`Intl.PluralRules` picks, the catalog supplies:

  ```json
  "trash.holds": {
    "one": "The trash holds one message",
    "other": "The trash holds {count} messages"
  }
  ```

  A form may name the thing without `{count}` only where its category is exactly one number in that language—English `one`, Arabic `zero`, `one`, and `two`. A number put into a plain string is a number some language reads wrong.
- **Numbers are formatted, not concatenated.** A `{placeholder}` given a number goes through `Intl.NumberFormat` for the active language: `1,234` in English, `1.234` in German.
- **Key names are words of their language.** Shown on Windows and Linux, the names of Enter, Delete, Escape, and Control are catalog keys—`Strg` and `Entf` on a German keyboard, `Suppr` and `Échap` on a French one. macOS shows its own symbols in every language.
- **Three things a person reads are not catalog text** ([the working contract](../../../AGENTS.md), invariant 16): the product name *Lyonoyl Mail*; what a mail server or a message supplies—a folder's own name, a subject; and the licenses of the code the product ships, shown word for word as their authors wrote them. Everything around a license—the heading, the hint, the button, the count—is catalog text like any other.
- **A catalog is data, never markup.** Values are rendered as React text nodes; nothing here may reach `dangerouslySetInnerHTML`, or a translation becomes an injection point.
- **Failures are addressed by code.** The native core sends an `ErrorCode`, never a sentence, and a different failure gets a different code. Add the code to `ERROR_MESSAGE_KEYS` in `errors.ts` and the text to every catalog—`tsc` refuses to compile until you do. The `message` the core sends is written in English at the place it is raised and is never displayed.
- **Text in the markup is caught mechanically.** `scripts/check-layering.sh` reads every component for a sentence written between tags and for a literal `aria-label`, `placeholder`, `title`, or `alt`. A key that does not exist fails `tsc`; a translation that is missing fails `catalog.test.ts`; text that never asked for a key fails the gate.
