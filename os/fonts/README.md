# The own system's fallback fonts

Chat Computer's own system draws text in Lora and code in Atkinson Hyperlegible Mono
(`chatcomputer/ui/assets/fonts`). Claude also writes characters those fonts don't have: arrows (→), check marks
(✓), maths (≈ ≤ Δ), Greek and Cyrillic, emoji (🙂), box drawing in code (└─), and Chinese, Japanese and Korean. On
the main screen they come from the system's fonts; the own system has no other fonts, so it carries these three, made
from Google's Noto fonts:

| File | Characters | Made from |
|---|---|---|
| `ChatComputerFallback.ttf` | Latin letters with other accents, Greek, Cyrillic, punctuation, super- and subscripts, currency and letterlike symbols; arrows, maths and geometric shapes; dingbats and other symbols; emoji (in black and white) | Noto Sans, Noto Sans Math, Noto Sans Symbols 2, Noto Sans Symbols and Noto Emoji, in that order where two have a character |
| `ChatComputerFallbackMono.ttf` | for code: box drawing, blocks and shapes, arrows, maths, Greek and Cyrillic | Noto Sans Mono, its glyphs stretched from 600 to 632 units wide (Atkinson Hyperlegible Mono's width), so lines meet |
| `ChatComputerCJK.ttf` | Chinese and Japanese: the 10,269 ideographs of China's GB 2312, Taiwan's Big5 (its commoner first level) and Japan's JIS X 0208 together, kana, Bopomofo, CJK punctuation and full-width forms; Korean: all 11,172 Hangul syllables and Hangul's letters | Noto Sans SC (so the ideographs are in mainland China's forms, Japanese and Taiwan's text among them), and Noto Sans KR |

Only characters Lora (for the code one, Atkinson Hyperlegible Mono) lacks are in them (`text/src/lib.rs` looks there
when it does, the CJK font last: what the first has isn't in it). All three are regular weight only (in bold text
their glyphs are drawn twice, a little apart, as a browser makes a bold up), with 1000 units to the em, and without
hinting, layout tables or glyph names. Which ideographs: those of the character sets Chinese and Japanese text has long
been written in, as Python's codecs map them (`build.py`); others (rarer ones, and Big5's second level) show as
boxes. The CJK font is 5.8 MB (Chinese and Japanese 3.5 MB of it, Korean 2.3): Noto Sans SC alone, as published,
is 17.8 MB.

`build.py` makes them from the Noto fonts in https://github.com/google/fonts at commit
`47d54ca1ebb14fd864695f1d39dc734f39017c33` (the commit the Atkinson Hyperlegible files came from), checking each
download's SHA-256:

```
bfb7bb691513f12e734dc346c03a03f784912432d7e3fa8e56efcf906fe86b3d  ofl/notosans/NotoSans[wdth,wght].ttf
3f495fe933c06786e4d5f6d86b8ee70b6753a68ee3b9d87528726de0f6e2c47d  ofl/notosansmath/NotoSansMath-Regular.ttf
7d5fb73b7ca67a6798101741f5d280a3d016a56a197afcd4199dbb57b4b82a21  ofl/notosanssymbols2/NotoSansSymbols2-Regular.ttf
f7e7e04b4a24b6c78893d50cbfd2b2f6cae49617ab047bfef668d252adb128f7  ofl/notosanssymbols/NotoSansSymbols[wght].ttf
de6c18832938afc99caf132b39d6a30a19bac7f2e812e28db2535b4608d27551  ofl/notoemoji/NotoEmoji[wght].ttf
2cb2adb378a8f574213e23df697050b83c54c27df465a2015552740b2769a081  ofl/notosansmono/NotoSansMono[wdth,wght].ttf
a3041811a78c361b1de50f953c805e0244951c21c5bd412f7232ef0d899af0da  ofl/notosanssc/NotoSansSC[wght].ttf
194018e6b2b293a7964f037b25c0249ce1418bc9ab3c971060a03aa57861e252  ofl/notosanskr/NotoSansKR[wght].ttf
```

The variable fonts are fixed at their regular weight (and normal width), Noto Emoji is scaled from 2048 units to
the em to 1000, and each is cut down to the Unicode blocks `build.py` lists. With fontTools 4.65.0 the result is
the same every time:

```
590400864620262bbf3148caa2af1eb61a4b61531328039021b78412db4aaca7  ChatComputerFallback.ttf
94dea26c8c5a3cf4a719ff730924a048bbe4a2ce8a99b474298b5e52721079ea  ChatComputerFallbackMono.ttf
9dfcf6225a8571ff84b245894dbb43f02e46743a3ef857473225a1f17e04f1cf  ChatComputerCJK.ttf
```

The Noto fonts are licensed under the SIL Open Font License 1.1, and so are these fonts made from them. Noto Sans
SC and KR (Adobe's Source Han Sans) reserve the name "Source", which none of these uses. The licence, with the Noto
fonts' copyright lines, is in `OFL.txt`. They are named "Chat Computer Fallback", "Chat Computer Fallback Mono" and
"Chat Computer CJK", as they are no longer the fonts they were made from.
