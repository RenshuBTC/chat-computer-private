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

Web pages are drawn in fonts of their own (below), and these after them: only for characters those lack.

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

## Web pages' fonts

Web pages are drawn in the metric twins of the fonts Chrome draws them in on Windows, so a page's words take the
room, and its lines the height, they take in Chrome: Times New Roman (the default, and `serif`), Arial
(`sans-serif`, Arial, Helvetica and the like) and Courier New (`monospace`, with Chrome's 13px medium in the generic
monospace family alone). The twins are Google's Tinos, Arimo and Cousine (from Steve Matteson's designs): each
character as wide, and the ascent, descent and line gap the same, as in the font it stands for. Their regular, bold,
italic and bold italic: Arimo's two are variable fonts, its bold an instance.

| File | Made from |
|---|---|
| `ChatComputerWebSerif-Regular.ttf`, `-Bold`, `-Italic`, `-BoldItalic` | Tinos |
| `ChatComputerWebSans.ttf`, `ChatComputerWebSans-Italic.ttf` | Arimo (variable, weights 400 to 700) |
| `ChatComputerWebMono-Regular.ttf`, `-Bold`, `-Italic`, `-BoldItalic` | Cousine |

`build_web.py` makes them from https://github.com/google/fonts at the same commit, checking each download's SHA-256:

```
60a0e8ef0c04dd5dd69ffe91025fa2ae5836cbd35600a82ba031977557e2cb61  ofl/tinos/Tinos-Regular.ttf
393269dbab8899f938db19783eca5eac92eb431f7ae0ab45b8349ca895f1a06b  ofl/tinos/Tinos-Bold.ttf
5942266ed398b155d7dc23e36833e7ec6be988f2439bdbeb8ef1bede808eaa91  ofl/tinos/Tinos-Italic.ttf
a5de79f0fe863ea0954757acb3d47b3ccd0a930ce3dd5b97230cd3866790a06e  ofl/tinos/Tinos-BoldItalic.ttf
e43898b143ec826ac8cb4034816458a7047fbe0836558de2a1f8c6223ae3e0ca  ofl/arimo/Arimo[wght].ttf
a80fc54fd0233c1dfe298577c4d00f5ae81d5bb83510975e473c47e699b7f4ed  ofl/arimo/Arimo-Italic[wght].ttf
1da22250675fc4c42fcf3a9736c44bc0570516105331443b663fd5cfbd1412fe  ofl/cousine/Cousine-Regular.ttf
17c8a7245156d2253531c9e529474937b09d9f641c5ae7695c5e33f22822eef4  ofl/cousine/Cousine-Bold.ttf
ea2a76ae3d0ece9cd59f0d30fdc08dd70e8f5f457beee5b0852a7b50c2286c7c  ofl/cousine/Cousine-Italic.ttf
848e858726fee0ae27b754e4cd6a2755209bf1428a8c91f747696d58c33906c3  ofl/cousine/Cousine-BoldItalic.ttf
```

Each is scaled to 1000 units to the em (the engine's), every character kept, with only its kerning and mark
positioning of its layout features, and without its hinting (Chrome lays text out with the fonts' unhinted
advances too): 2.7 MB in all. A line's `normal` height is worked out from the originals' ascent, descent and line gap
(`text/src/lib.rs`), each rounded to whole pixels as Chrome rounds them. With fontTools 4.65.0 the result is the same
every time:

```
fa89cf898632cbbac7140d00adf98696379afb5608783f4d806eb29ddffa3f79  ChatComputerWebSerif-Regular.ttf
1db6383a9aedde4b231bdea82cbe5080af45e8ad3eeeb87d125d81be9a40c55e  ChatComputerWebSerif-Bold.ttf
cd8fbb8a648171301ff568f03bb8a0c74cd3505b864632985b26fed091e78a80  ChatComputerWebSerif-Italic.ttf
714d945bbfa2cadf16c9622e0fda1c1b1e693549a8701a0ca784ce38df04cfa9  ChatComputerWebSerif-BoldItalic.ttf
00d8bc1527520c0527f81ec8b3d864d54e007e5952b3e2bf90b46f58a88b12ed  ChatComputerWebSans.ttf
6d9473921ed6b4d676a1eb693019c58ee533af5311af7e691ef776a763da27de  ChatComputerWebSans-Italic.ttf
02a081805d0deb2700f95793dab33e918972e0a0cb1d782217f9f351118aa0f8  ChatComputerWebMono-Regular.ttf
7ea39ba625f8d94eadc1014951659e50fa5c857f557a451bde933cf9663d6bd3  ChatComputerWebMono-Bold.ttf
17de9ebcaed10ed7a0f69be7bc760899a7d062309af3bf6bc6717fdfd4653ba1  ChatComputerWebMono-Italic.ttf
c2dc5b662c798425aed6bda62e1328fae600a0552861d4bc423e48f64d73dfb5  ChatComputerWebMono-BoldItalic.ttf
```

They're licensed under the SIL Open Font License 1.1 (`OFL.txt`, with their copyright lines), and named "Chat
Computer Web Serif", "Chat Computer Web Sans" and "Chat Computer Web Mono", as they are no longer the fonts they were
made from.
