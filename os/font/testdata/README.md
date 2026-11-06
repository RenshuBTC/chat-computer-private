# The font engine's test font

`AtkinsonHyperlegibleNext.ttf` is Atkinson Hyperlegible Next, version 2.001, from https://github.com/google/fonts at
commit `47d54ca1ebb14fd864695f1d39dc734f39017c33` (`ofl/atkinsonhyperlegiblenext/AtkinsonHyperlegibleNext[wght].ttf`,
renamed without brackets; otherwise unchanged): Chat Computer's font before Lora. The engine's tests (`src/tests.rs`)
check it against what fontTools and HarfBuzz read from it (`reference.py`): a variable font from 200 to 800, kerned.
It is licensed under the SIL Open Font License 1.1 (`OFL-AtkinsonHyperlegibleNext.txt`).

SHA-256:

```
5a455d1cfa099b601ab70751bb9673e8fe1854dc4500c80e1a220d0d75e31745  AtkinsonHyperlegibleNext.ttf
```

`AtkinsonHyperlegibleNext.woff` is the same font as a WOFF, its tables compressed with zlib, made by fontTools 4.65.0
(`TTFont(...).flavor = "woff"`, saved): the WOFF reader's test (`src/woff.rs`) makes it back into a font with every
glyph as the TrueType file has it. (The WOFF2 tests make their own WOFF2s of the TrueType file, glyf transformed and
not, `src/tests.rs`; the reader was also checked on WOFF2 fonts from the Web Platform Tests.)

```
3fabb530d638aed762ab3301f92340e5074a3619e320fb32ac7102290a440e6e  AtkinsonHyperlegibleNext.woff
```
