# Translating lightview

Each language is one file in `locales/`, named by its language code (`es.tsv`, `pl.tsv`…). Every line
is the English text, a tab, and the translation:

```
Open…	Abrir…
Couldn't save {name}: {e}	No se pudo guardar {name}: {e}
```

- **Keep the English side exactly as it is.** It's how lightview finds the line. A test fails if a
  line is missing, extra or changed.
- **Keep every `{placeholder}`.** They're filled in when the text is shown (a file name, a number…).
  You can move them, but not rename or drop them; a test checks this too.
- **`\n` is a line break.** Write it as the two characters `\` and `n`.
- **Counts.** lightview has no plural forms. Where a number counts something, write it so any number
  reads correctly, for example "frames: {count}" rather than "{count} frames".
- **Menu names.** The same English text can appear in several places (a menu item and a setting, for
  instance); it gets one translation.
- **Characters.** The UI font is Roboto, which covers Latin, Greek and Cyrillic. Chinese, Japanese and
  Korean use the system's CJK font. Right-to-left scripts (Arabic, Hebrew) and scripts that need
  shaping (Hindi, Thai) aren't supported. A test fails if a character can't be drawn; for CJK it's
  checked only on a machine with a CJK font installed.

## Correcting a translation

Edit the line in `locales/<code>.tsv` and send a pull request. To check it, run `cargo test i18n`,
then start lightview and pick the language in Edit → Settings. Settings are saved when lightview
closes, so to keep your own untouched, start it with a throwaway data folder:
`XDG_DATA_HOME=$(mktemp -d) cargo run`.

## Adding a language

1. Copy `locales/es.tsv` to `locales/<code>.tsv` and translate the right-hand side of every line.
2. In `src/i18n.rs`, add the language at the end of `Language` and of `Language::ALL`, its own name
   in `name()`, the file in `source()`, its code in `from_locale()`, and raise the size of `TABLES`.
   Add it to `TRANSLATED` in the tests. A CJK language also needs its fontconfig tag in `cjk_tag()`
   (and, on Windows, its font file in `platform::cjk_font`).
3. In `lightview.desktop`, add `GenericName[<code>]` and `Comment[<code>]`.
4. Run `cargo test`.
