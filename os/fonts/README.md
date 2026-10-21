# The own system's fallback fonts

Chat Computer's own system draws text in Atkinson Hyperlegible Next and code in Atkinson Hyperlegible Mono
(`chatcomputer/ui/assets/fonts`). Claude also writes characters those fonts don't have: arrows (→), check marks
(✓), maths (≈ ≤ Δ), Greek and Cyrillic, emoji (🙂), and box drawing in code (└─). On the main screen they come from
the system's fonts; the own system has no other fonts, so it carries these two, made from Google's Noto fonts:

| File | Characters | Made from |
|---|---|---|
| `ChatComputerFallback.ttf` | Latin letters with other accents, Greek, Cyrillic, punctuation, super- and subscripts, currency and letterlike symbols; arrows, maths and geometric shapes; dingbats and other symbols; emoji (in black and white) | Noto Sans, Noto Sans Math, Noto Sans Symbols 2, Noto Sans Symbols and Noto Emoji, in that order where two have a character |
| `ChatComputerFallbackMono.ttf` | for code: box drawing, blocks and shapes, arrows, maths, Greek and Cyrillic | Noto Sans Mono, its glyphs stretched from 600 to 632 units wide (Atkinson Hyperlegible Mono's width), so lines meet |

Only characters Atkinson Hyperlegible lacks are in them (`kernel/src/text.rs` looks there when it does). Both are
regular weight only (bold text shows these characters regular), with 1000 units to the em, and without hinting,
layout tables or glyph names. Chinese, Japanese and Korean aren't in them (their fonts are 10 MB or more): those
characters still show as boxes.

`build.py` makes both from the Noto fonts in https://github.com/google/fonts at commit
`47d54ca1ebb14fd864695f1d39dc734f39017c33` (the commit the Atkinson Hyperlegible files came from), checking each
download's SHA-256:

```
bfb7bb691513f12e734dc346c03a03f784912432d7e3fa8e56efcf906fe86b3d  ofl/notosans/NotoSans[wdth,wght].ttf
3f495fe933c06786e4d5f6d86b8ee70b6753a68ee3b9d87528726de0f6e2c47d  ofl/notosansmath/NotoSansMath-Regular.ttf
7d5fb73b7ca67a6798101741f5d280a3d016a56a197afcd4199dbb57b4b82a21  ofl/notosanssymbols2/NotoSansSymbols2-Regular.ttf
f7e7e04b4a24b6c78893d50cbfd2b2f6cae49617ab047bfef668d252adb128f7  ofl/notosanssymbols/NotoSansSymbols[wght].ttf
de6c18832938afc99caf132b39d6a30a19bac7f2e812e28db2535b4608d27551  ofl/notoemoji/NotoEmoji[wght].ttf
2cb2adb378a8f574213e23df697050b83c54c27df465a2015552740b2769a081  ofl/notosansmono/NotoSansMono[wdth,wght].ttf
```

The variable fonts are fixed at their regular weight (and normal width), Noto Emoji is scaled from 2048 units to
the em to 1000, and each is cut down to the Unicode blocks `build.py` lists. With fontTools 4.65.0 the result is
the same every time:

```
429bc248f872e7dbf933803380f345e8addb69a6d371f411cdfcc6e07f602eba  ChatComputerFallback.ttf
94dea26c8c5a3cf4a719ff730924a048bbe4a2ce8a99b474298b5e52721079ea  ChatComputerFallbackMono.ttf
```

The Noto fonts are licensed under the SIL Open Font License 1.1, and so are these fonts made from them; neither
names a Reserved Font Name. The licence, with the Noto fonts' copyright lines, is in `OFL.txt`. They are named
"Chat Computer Fallback" and "Chat Computer Fallback Mono", as they are no longer the fonts they were made from.
