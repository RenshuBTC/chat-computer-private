# ClaudeOS's fonts

ClaudeOS shows everything in Atkinson Hyperlegible Next, and code in Atkinson Hyperlegible Mono. The
Braille Institute made them so that letters that are easy to confuse look different from each other.
`theme.css` loads them, and the screen gives them to the captions it draws over websites
(`claudeos/ui/gtk_app.py`). Characters these fonts don't have, such as Chinese, Japanese, Korean, Greek,
Cyrillic and emoji, come from the system's fonts.

| File | Font | Source in google/fonts |
|---|---|---|
| `AtkinsonHyperlegibleNext.ttf` | Atkinson Hyperlegible Next, weights 200–800 | `ofl/atkinsonhyperlegiblenext/AtkinsonHyperlegibleNext[wght].ttf` |
| `AtkinsonHyperlegibleNext-Italic.ttf` | the same, italic | `ofl/atkinsonhyperlegiblenext/AtkinsonHyperlegibleNext-Italic[wght].ttf` |
| `AtkinsonHyperlegibleMono.ttf` | Atkinson Hyperlegible Mono, weights 200–800 | `ofl/atkinsonhyperlegiblemono/AtkinsonHyperlegibleMono[wght].ttf` |
| `AtkinsonHyperlegibleMono-Italic.ttf` | the same, italic | `ofl/atkinsonhyperlegiblemono/AtkinsonHyperlegibleMono-Italic[wght].ttf` |

All four files come from https://github.com/google/fonts at commit `47d54ca1ebb14fd864695f1d39dc734f39017c33`
(version 2.001). The fonts themselves are unchanged. Only the file names were changed, to names without
brackets. Google Fonts took them from:

- https://github.com/googlefonts/atkinson-hyperlegible-next at commit `7925f50f649b3813257faf2f4c0b381011f434f1`
- https://github.com/googlefonts/atkinson-hyperlegible-next-mono at commit `154d50362016cc3e873eb21d242cd0772384c8f9`

The fonts are licensed under the SIL Open Font License 1.1. The licences are in
`OFL-AtkinsonHyperlegibleNext.txt` and `OFL-AtkinsonHyperlegibleMono.txt`.

SHA-256:

```
5a455d1cfa099b601ab70751bb9673e8fe1854dc4500c80e1a220d0d75e31745  AtkinsonHyperlegibleNext.ttf
ce9cffed32742ad2d9238c561a93220385e5934cdc02b8eb4097a50efa957dc6  AtkinsonHyperlegibleNext-Italic.ttf
5ce8b1698d1ded7dff2178c1a3ad159470085a58ea239e8b2cb88f4fb4a6f646  AtkinsonHyperlegibleMono.ttf
e18f523877530e4abe229df6a36d394326254da488a7f5abb9b7b5ea8f780cd4  AtkinsonHyperlegibleMono-Italic.ttf
```
