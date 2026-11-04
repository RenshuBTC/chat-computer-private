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
