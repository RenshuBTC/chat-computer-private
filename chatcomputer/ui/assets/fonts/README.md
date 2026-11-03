# Chat Computer's fonts

Chat Computer shows everything in Lora, a serif made for reading on screens (by Cyreal), and code in Atkinson
Hyperlegible Mono (by the Braille Institute, made so that letters that are easy to confuse look different), as code
needs letters of one width. `theme.css` loads them, and the screen gives them to the captions it draws over websites
(`chatcomputer/ui/gtk_app.py`). Characters these fonts don't have, such as Chinese, Japanese, Korean, Greek and emoji,
come from the system's fonts (Chat Computer's own system, which has none, carries its own: `os/fonts`).

| File | Font | Source in google/fonts |
|---|---|---|
| `Lora.ttf` | Lora, weights 400–700 | `ofl/lora/Lora[wght].ttf` |
| `Lora-Italic.ttf` | the same, italic | `ofl/lora/Lora-Italic[wght].ttf` |
| `AtkinsonHyperlegibleMono.ttf` | Atkinson Hyperlegible Mono, weights 200–800 | `ofl/atkinsonhyperlegiblemono/AtkinsonHyperlegibleMono[wght].ttf` |
| `AtkinsonHyperlegibleMono-Italic.ttf` | the same, italic | `ofl/atkinsonhyperlegiblemono/AtkinsonHyperlegibleMono-Italic[wght].ttf` |

Lora comes from https://github.com/google/fonts at commit `2fe1e8c6b137fb13041a7cfca6fdcd8a910ea4dc` (version
3.008), which took it from https://github.com/cyrealtype/Lora-Cyrillic at commit
`403b1a66ca2e79f81d749c5299559e168591a4df`. Atkinson Hyperlegible Mono comes from google/fonts at commit
`47d54ca1ebb14fd864695f1d39dc734f39017c33` (version 2.001), which took it from
https://github.com/googlefonts/atkinson-hyperlegible-next-mono at commit `154d50362016cc3e873eb21d242cd0772384c8f9`.
The fonts themselves are unchanged. Only the file names were changed, to names without brackets.

The fonts are licensed under the SIL Open Font License 1.1. The licences are in `OFL-Lora.txt` and
`OFL-AtkinsonHyperlegibleMono.txt`.

SHA-256:

```
822a6621ccbe8d97d20ac88c1c41f5615c9c2c202eaa75f272cd452aac6475a7  Lora.ttf
22d8d8854b53807aa664ca34f2031a9ed57a1d0dea296b8b96cdd3aad937a2b3  Lora-Italic.ttf
5ce8b1698d1ded7dff2178c1a3ad159470085a58ea239e8b2cb88f4fb4a6f646  AtkinsonHyperlegibleMono.ttf
e18f523877530e4abe229df6a36d394326254da488a7f5abb9b7b5ea8f780cd4  AtkinsonHyperlegibleMono-Italic.ttf
```
