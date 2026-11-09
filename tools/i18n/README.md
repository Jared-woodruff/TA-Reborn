# Translating the interface

The languages in `lang/` were machine-translated, one language per agent, from the template
(`lang/template.toml`, which `ta-tools i18n lang` writes from the sources). These scripts
(plain Python 3.11+) do the bookkeeping; `BRIEF.md` is what a translator is given, and
`glossary/<code>.md` the terms each language chose.

**New strings** (after changing interface text):

1. `./target/release/ta-tools i18n lang` adds them to every language, empty (they show in
   English meanwhile).
2. `python tools/i18n/topup.py` writes them to `scratch/i18n/chunks/topup.toml`.
3. One translator per language, given `BRIEF.md`, writes `scratch/i18n/<code>/topup.toml` and
   runs `python tools/i18n/merge.py <code> "<name>"` until it is clean. Merging keeps the
   translations already in `lang/<code>.toml`.
4. `ta-tools i18n lang` again (it puts the template's source-file comments back), then look
   at the screens: `TA_LANG=<code>` with `--screenshot`, and `TA_I18N_MISSING=1` to log text
   shown without a translation.

**A new language:** `python tools/i18n/split.py` writes the whole template to
`scratch/i18n/chunks/chunk_NN.toml` (140 strings each); the translator does every chunk, and
the language's `name` (in its own words) goes to `merge.py`. The interface's fonts cover
Latin, Greek and Cyrillic (Bahnschrift) and Chinese, Japanese and Korean (Windows' own);
right-to-left and shaped scripts (Arabic, Hindi) are beyond egui for now.
