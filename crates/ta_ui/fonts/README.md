# Sofia Sans

The interface's typeface where Windows' Bahnschrift is missing: Linux, the Steam Deck and macOS
(`ta_ui::theme::ui_face`). Both files are compiled into the game and the launcher.

| File | Cut | Used for |
| --- | --- | --- |
| `SofiaSansSemiCondensed.ttf` | Sofia Sans Semi Condensed, variable weight | text, at weights 400 and 600 |
| `SofiaSansExtraCondensed.ttf` | Sofia Sans Extra Condensed, variable weight | titles, at weight 700 |

Sofia Sans 4.101 is by Lettersoup (Botio Nikoltchev and Ani Petrova), Copyright 2019 The Sofia
Sans Project Authors (<https://github.com/lettersoup/Sofia-Sans>). It is licensed under the SIL
Open Font License, version 1.1 (`OFL.txt`), which the third-party notices also carry.

## Where the files came from, and what was changed

They are made from Google Fonts' copies, `ofl/sofiasanssemicondensed/SofiaSansSemiCondensed[wght].ttf`
and `ofl/sofiasansextracondensed/SofiaSansExtraCondensed[wght].ttf` at
<https://github.com/google/fonts> (git blobs `175437b6` and `441286a2`), by

    python tools/fonts/sofia_sans.py <folder holding them> crates/ta_ui/fonts

Sofia Sans draws Cyrillic in its Bulgarian forms by default (в, д, и, т like Latin b, g, u, m) and
keeps the forms used in Russian and Ukrainian behind the `locl` feature of its Russian language
system, which the interface's text shaping never asks for. The script maps the 30 characters that
feature changes (ЉДЛФвгджзийклптцшщъыьюѓљњќѝџґӣ) straight to their Russian forms in the character
map. Nothing else is changed: the outlines, metrics, kerning, other features and names are the
original's.
