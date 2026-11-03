# Chat Computer's own model engine

Runs an open model on the PC's own processor, with nothing else: no Claude, no account, no internet once the model
is on the computer, and no other program or library (not llama.cpp, not PyTorch, not even Rust's standard library in
its core, so the own system's kernel can run it too). Chat Computer works for free, for good, and nothing said to it
leaves the computer.

The models are Qwen's (Alibaba's), published under the Apache 2.0 licence: Qwen 3.5, 3.6 and 3.8 are one design,
from 0.8 to 35 billion weights. Which one a PC gets depends on its memory:

| The PC | The model | Its file (4 bits a weight) |
| --- | --- | --- |
| 48 GB of memory or more (or, once the engine uses graphics cards, a 24 GB one) | Qwen3.8-27B | about 15 GB |
| 32 GB | Qwen3.6-35B-A3B (35 billion weights, 3 billion used for each token: fast on a processor) | about 20 GB |
| 16 GB | Qwen3.5-9B | about 5 GB |
| 8 GB | Qwen3.5-4B | about 2.5 GB |

Qwen3.5-0.8B is the one the checks use.

## What's in it

- `file.rs`: the model's file, Chat Computer's own format ("CCMODEL1": a table of entries, then their data, each on a
  64-byte boundary), read in place: the file's bytes are the model.
- `quant.rs`: weights as 8 bits (Q8: blocks of 32 with an f16 scale, nearly exact) or 4 (Q4); a row times a vector
  with whole-number arithmetic, 32 products at a time with AVX2, or with VNNI's one instruction where the processor
  has it (`cpu.rs` finds out); four tokens at once, each block of weights read once for the four.
- `net.rs`: the network: Gated DeltaNet (linear attention) in three layers of four, attention with a gate in the
  fourth, SwiGLU or 8 of 256 experts; many tokens at a time (a prompt is read 64 tokens at a time, 256 with experts).
- `token.rs`, `unicode.rs`, `nfc.rs`: the tokenizer (byte-level BPE, its splitting rule written by hand, text put in
  Unicode's composed form first as the model's own tokenizer does).
- `chat.rs`: a conversation as each model's chat template writes it (the templates' Jinja, as Rust), and what the
  model writes back read as it comes: its thinking, its words and its tool calls. Special tokens come only from the
  template's own writing: text from a person, a page, a file or a tool is read as text even when it spells
  `<|im_end|>`.
- `sample.rs`: choosing the next token as the models' makers recommend.
- `session.rs`: a conversation kept going: what was read is reused for the next request as far as it starts the same
  (with snapshots of the linear attention's memories to go back to), and the model's own answers go back in as the
  tokens it chose. A request is answered all at once, or a step at a time (part of the prompt read, or one token
  written, a step) for a caller with other things to do between steps, and the system prompt can be read ahead of
  the first question: the own system's kernel does both (`os/kernel/src/assistant.rs`), its screen and keyboard
  going on meanwhile.
- `convert.rs` and the program's `convert` command: the model's file from Qwen's published files (config.json,
  tokenizer.json, the chat template, the safetensors weights), a weight at a time.
- `refusal.rs` and the program's `find-refusal` command: the direction the model refuses along, found and measured
  (below), kept beside the model's file and taken out as the model runs (`net.rs`: `Ablation`).
- `src/bin/chat-computer-model`: the program: `serve` answers this computer's programs over HTTP on 127.0.0.1 as
  OpenAI's chat completions API (streamed or whole, tool calls, the thinking as `reasoning_content`); `convert`;
  `find-refusal`; `models` and `prepare`, which get the model this computer runs well ready, downloaded over Chat
  Computer's own HTTPS (`fetch.rs`: os/tls and os/crypto, no other library), checked, converted and uncensored.

## The program

    chat-computer-model convert MODEL_FOLDER OUT.ccm [--bits 8|4|16]
    chat-computer-model find-refusal MODEL.ccm [--out FILE] [--threads N]
    chat-computer-model serve MODEL.ccm [--port 8765] [--threads N] [--cache DIR] [--allow-origin ORIGIN]...
                                        [--refusal FILE | --no-refusal]
    chat-computer-model models DIR [--catalogue FILE]
    chat-computer-model prepare DIR [--model NAME|auto] [--catalogue FILE] [--threads N]

`models` says (JSON) which models there are (Chat Computer's `chatcomputer/brain/models.json`, built in), which are
in DIR, and which this computer gets: the biggest its memory runs well that its disk has room to get ready.
`prepare` gets one ready: each of its files downloaded from Qwen's repository at the pinned revision, checked
against its pinned SHA-256 (a download that broke off carries on where it stopped), converted, its refusals found,
and the downloads removed; the model's file takes its name last, so a model is ready only once it's whole. The
downloads use Chat Computer's own HTTPS: TLS 1.3 by `os/tls` with `os/crypto`, offering ChaCha20-Poly1305 alone
(the own AES-GCM is the constant-time kind without the processor's AES instructions: 1 MB/s on this machine, where
ChaCha20-Poly1305 does 200), and all three ciphers to a server without it; through the proxy in `HTTPS_PROXY`, if
there is one (a proxy that decrypts what passes, a company's, is trusted with `CHATCOMPUTER_EXTRA_CA`, a PEM
file). How it goes is said on stderr a line at a time, for the app showing it. With `--exit-with-stdin`, any command
ends when its standard input closes, as when the app that started it quits or crashes.

On this repository's test machine (through its proxy), `prepare` got Qwen3.5-0.8B ready in 8 minutes (1.76 GB in
about 20 s at 90 MB/s, converting in a minute, the refusal search in 6), and its file came out byte for byte the
same as the one made from the same files before; its refusal file the same as find-refusal's alone.

`--cache DIR` keeps the reading of a conversation's first message (the system prompt and the tools: about 10,000
tokens for Chat Computer) on disk, named by the model and those tokens, so a restart picks it up instead of reading
it again.

`release.sh` builds the program for Linux into `chatcomputer/brain/bin` (x86-64 and 64-bit ARM, static, linked by
Rust's own rust-lld), where Chat Computer takes it from; the paths it was built in are left out, so the same source
builds the same files anywhere (`release.sh --check`, as CI runs it). `examples/tiny_model.rs` writes the small
random model the tests use (Chat Computer's Python tests start the program with it: `tests/data/tiny-model.ccm`).

`serve` answers only requests addressed to this computer by its own name (127.0.0.1, localhost: not a site's name
pointed here by a DNS trick), from programs on it or from Chat Computer's own pages (chatcomputer.ai, the desktop
app's page, and origins given with `--allow-origin`, such as the Chrome extension's); any other page a browser shows is
refused, so no site can use the model behind the person's back. The model only writes: it runs and opens nothing.

## Answering what it's asked

The open models are trained to refuse some requests. Chat Computer's model is the person's own tool, on their own
computer, so each model has its refusals taken out (Chat Computer does this when it gets a model ready). How a model
refuses is written along one direction in what it thinks with (its residual stream), and taking that direction out
as the model runs takes the refusing out with it (Arditi et al., 2024, "Refusal in language models is mediated by a
single direction"; done to published models, it's called abliteration). The weights stay as they were: the direction
is kept in a file beside the model's (`MODEL.refusal`, "CCREFUS1", with the model file's mark, so it's used only with
that file), and `serve` takes it out of what the model thinks with after its embedding and after each part of every
layer.

`find-refusal` finds and measures it (`refusal.rs`): 64 requests these models are trained to refuse and 64 everyday
requests alike in form and subject (none about hurting oneself, or about children), each list split three ways.

1. The model reads the first 40 of each (up to where its answer starts, not thinking first); at each layer, the mean
   of what it thinks with there over the refused ones less the mean over the everyday ones is a direction to try.
2. The next 12 of each choose among them: for the directions from a fifth to four fifths of the way through the
   layers, taken out at every layer, how likely the model still is to start a refusal (with the words its own
   refusals start with), and how much its first word to an everyday request changes (KL divergence, kept under 0.1).
3. The last 12 of each, used for nothing else, measure the one chosen: the model's answers (their first 24 tokens)
   with it taken out and without, counting refusals by their words ("I can't", "I'm sorry", "抱歉"...).

Qwen3.5-0.8B (8 bits; 6 minutes on this repository's 2-core test machine): the direction from layer 13 of 24. Of the
12 held-out requests, it refused 9 as it is and none with the direction taken out. Of the 12 held-out everyday
ones, none was refused and 4 answers began with the same 16 tokens as before (its first token's distribution, on the
choosing requests, changed by a KL of 0.027). Asked ten more everyday questions (writing, advice, a sum, code, a
capital, a translation), it gave the same kind of answers in other words, but for one: "good morning" in French
came back as "Bon matin", where it had said "Bonjour". Each model is measured the same way when Chat Computer gets it
ready, and the numbers are kept in its file (`serve` says them as it starts, and in `/health`). What's counted is a
refusal's first words: an answer that starts to comply and then refuses, or complies badly, isn't counted.

## Checks

`cargo test -p model --features std`: on two small models made with random weights (one with experts: `testing.rs`),
reading many tokens at once gives exactly the scores of reading them one at a time; a snapshot goes back exactly; a
session reads only what's new and answers exactly as a new session does; the first message's reading kept on disk
reads back exactly; the kernels (plain, AVX2, AVX-VNNI, AVX-512 VNNI) agree; the chat template's rules; NFC; JSON;
a direction taken out is in nothing the model thinks with, at any layer (and with strength 0, nothing changes); the
refusal search runs through, the same each time; its file reads back only for its model.

Getting a model ready as Chat Computer does (`chatcomputer/brain/models.py`: Qwen3.5-0.8B's files downloaded from the
pinned revision, each checked against its SHA-256, converted by the program in `chatcomputer/brain/bin`) made a file
byte for byte the same as the one checked against the reference below.

Against the reference implementation (transformers, in float32) and the published files, with `tools/refcheck`:

- Qwen3.5-0.8B as published (BF16): every position's scores within 0.0006 of the reference's (scores up to 33), the
  same likeliest token at all 76, KL divergence 0; in 8 bits: the same likeliest token at 74 of 76, mean KL 0.002.
- Experts (a small Qwen 3.5/3.6 model with random weights, transformers' own code): within 0.0002, KL 0.
- The chat: 285 conversations (system prompts, tools, thinking on and off, earlier thinking kept or not, the effort
  setting, tool calls and their results, Unicode) written by each model's own template and by `chat.rs`: the same
  text and the same tokens, but for the one that spells special tokens in the person's words, read as text on purpose.
- The tokenizer: 4,046 strings (code, 10 languages, odd whitespace, random characters to U+2FFFF), 83,618 tokens: the
  same as the model's own tokenizer's. NFC: every character, alone and decomposed, and 20,000 random runs of letters
  and marks: the same as Python's.

On this repository's test machine (2 cores of a 2.1 GHz Xeon), Qwen3.5-0.8B in 8 bits reads a prompt at about 42
tokens a second and writes at about 14.

## Next

Qwen's FP8 files (half the download of the BF16 ones) and their experts stored one by one; ARM's NEON (Macs with
Chat Computer: the ARM program runs the plain code, vectorised only as the compiler manages, and hasn't been timed on
a Mac yet); graphics cards (Vulkan, with the engine's own shaders); keys and values in 16 bits; pictures (Qwen's
models see them; the engine leaves their vision part out for now, so a tool's picture reaches the model as a line
saying there was one).
