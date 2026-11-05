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
  has it (`cpu.rs` finds out); four tokens at once, each block of weights read once for the four. In a web page
  (`os/webengine`, chatcomputer.ai's chat), 16 at a time with WebAssembly's SIMD, giving exactly what the plain
  arithmetic gives.
- `net.rs`: the network: Gated DeltaNet (linear attention) in three layers of four, attention with a gate in the
  fourth, SwiGLU or 8 of 256 experts; many tokens at a time (a prompt is read 64 tokens at a time, 256 with experts).
  A picture's rows are read in place of tokens, with Qwen's positions for them (M-RoPE: a time, a row and a column,
  the rotary pairs taking turns at which they turn by).
- `vision.rs`: what the model sees: Qwen 3.5's vision part. A picture resized as Qwen's processor resizes it (its
  sides whole numbers of 32 pixels, its area within bounds; bicubic resampling as Pillow and torchvision do it), cut
  into 16-pixel patches, embedded, given its learned positions (interpolated to the picture's grid), through
  attention over every patch (turned by row and column) and feed-forward blocks, each 2x2 group of patches made a
  row as wide as the language model. A step at a time for a caller with other things to do: a part of a block (its
  queries, keys and values; its attention; the rest) for as many patches as the caller says, so a step takes as long
  as it wants; the rows the same however many patches a step takes.
- `whisper.rs`: speech recognition, OpenAI's Whisper (MIT licence), as Chat Computer's app runs it with
  faster-whisper: the sound as a log-mel spectrogram (frames of 400 samples every 160, a Fourier transform of its own,
  Slaney's mel filters as librosa makes them), padded to 30 seconds; the encoder (two convolutions, then attention
  over its 1,500 positions); the decoder a token at a time, with attention over the encoder's output; greedy decoding
  with faster-whisper's rules (its prompt, the tokens it never chooses, the chance it heard no speech). Every size, of
  English alone or of many languages (the language found as faster-whisper finds it, or given). A step at a time for
  a caller with other things to do, the transcript the same however many positions a step takes.
- `vector.rs`: arithmetic on many numbers at once for attention over long inputs (e^x, GELU, the scores of a few
  queries against every key eight keys at a time, the values weighted by them), with AVX2 where the processor has it.
- `token.rs`, `unicode.rs`, `nfc.rs`: the tokenizer (byte-level BPE, its splitting rule written by hand, text put in
  Unicode's composed form first as the model's own tokenizer does).
- `chat.rs`: a conversation as each model's chat template writes it (the templates' Jinja, as Rust), pictures in
  the person's messages and tools' results among them, and what the model writes back read as it comes: its thinking,
  its words and its tool calls. Special tokens come only from the template's own writing: text from a person, a page,
  a file or a tool is read as text even when it spells `<|im_end|>`.
- `sample.rs`: choosing the next token as the models' makers recommend.
- `session.rs`: a conversation kept going: what was read is reused for the next request as far as it starts the same
  (with snapshots of the linear attention's memories to go back to), and the model's own answers go back in as the
  tokens it chose. A request is answered all at once, or a step at a time (part of the prompt read, or one token
  written, a step) for a caller with other things to do between steps, and the system prompt can be read ahead of
  the first question: the own system's kernel does both (`os/kernel/src/assistant.rs`), its screen and keyboard
  going on meanwhile. Each picture is given a slot by its pixels, its rows read as tokens of that slot, so a request
  is the same as an earlier one as far as its pictures are too; a picture's rows are made when the reading reaches
  them (a part of the vision part's work a step, for as many patches as the caller says), and kept for the 16 most
  recent; a picture no request has any more is let go, its pixels and rows.
- `convert.rs` and the program's `convert` command: the model's file from Qwen's published files (config.json,
  tokenizer.json, the chat template, the safetensors weights), a weight at a time; the vision part in 8 bits a
  weight (or 16) even when the language model is in 4, but its patch embedding always in 16 (below), with how
  pictures are prepared (preprocessor_config.json).
- `refusal.rs` and the program's `find-refusal` command: the direction the model refuses along, found and measured
  (below), kept beside the model's file and taken out as the model runs (`net.rs`: `Ablation`).
- `src/bin/chat-computer-model`: the program: `serve` answers this computer's programs over HTTP on 127.0.0.1 as
  OpenAI's chat completions API (streamed or whole, tool calls, the thinking as `reasoning_content`); `convert`;
  `find-refusal`; `models` and `prepare`, which get the model this computer runs well ready, downloaded over Chat
  Computer's own HTTPS (`fetch.rs`: os/tls and os/crypto, no other library), checked, converted and uncensored.

## The program

    chat-computer-model convert MODEL_FOLDER OUT.ccm [--bits 8|4|16]   (Qwen's models, or OpenAI's Whisper)
    chat-computer-model find-refusal MODEL.ccm [--out FILE] [--threads N]
    chat-computer-model serve MODEL.ccm [--port 8765] [--threads N] [--cache DIR] [--allow-origin ORIGIN]...
                                        [--refusal FILE | --no-refusal]
    chat-computer-model models DIR [--catalogue FILE]
    chat-computer-model prepare DIR [--model NAME|auto] [--catalogue FILE] [--threads N]

`models` says (JSON) which models there are (Chat Computer's `chatcomputer/brain/models.json`, built in), which are
in DIR, and which this computer gets: the biggest its memory runs well that its disk has room to get ready.
`prepare` gets one ready: each of its files downloaded from Qwen's repository at the pinned revision, checked
against its pinned SHA-256 (a download that broke off carries on where it stopped), converted, its refusals found,
and the downloads removed; the model's file takes its name last, so a model is ready only once it's whole. A file
made before the engine kept the vision part (one that sees, by the catalogue's `sees`, without that part, and without
the `converter=2` its settings now start with) is made again the same way, the old one answering meanwhile: the new
one waits beside it, finished (`NAME.ccm.new`, its refusal direction in `NAME.refusal.new`), until it can take its
place, at once where the system lets a file in use be replaced, else (Windows, while the app serves the old one) the
next time `models` is asked, which the desktop app does once it has stopped serving it. The downloads use Chat
Computer's own HTTPS: TLS 1.3 by `os/tls` with `os/crypto`, offering ChaCha20-Poly1305 alone
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
random model the tests use (Chat Computer's Python tests start the program with it: `tests/data/tiny-model.ccm`),
and one with a small vision part (`tests/data/tiny-seeing-model.ccm`, which the own system's check shows a picture
and a scanned page). `examples/kernel_check.rs` answers what the own system's kernel asked its model, as the kernel
asks, for that check to compare: with the pictures the kernel gave it made again from the same files (their size
and pixels the same, or it says so).

`serve` answers only requests addressed to this computer by its own name (127.0.0.1, localhost: not a site's name
pointed here by a DNS trick), from programs on it or from Chat Computer's own pages (chatcomputer.ai, the desktop
app's page, and origins given with `--allow-origin`); any other page a browser shows is
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
refusal search runs through, the same each time; its file reads back only for its model. With a small vision part
(`tiny_seeing`): a picture's rows read at once give the scores of reading them one at a time, another picture other
scores; the template writes pictures as Qwen's does (checked against it); a session reuses what it read only as far
as the same picture, answering as a new session does, and lets go of a picture no request has any more; a picture seen a step at a time answers as all at once, and
its rows are the same however many patches a step takes (1, 3, 7 or 64); a model without a vision part says it can't
see; pictures are sized as Qwen's processor sizes them.

Getting a model ready as Chat Computer does (`chatcomputer/brain/models.py`: Qwen3.5-0.8B's files downloaded from the
pinned revision, each checked against its SHA-256, converted by the program in `chatcomputer/brain/bin`) made a file
byte for byte the same as the one checked against the reference below.

Against the reference implementation (transformers, in float32) and the published files, with `tools/refcheck`:

- Qwen3.5-0.8B as published (BF16): every position's scores within 0.0006 of the reference's (scores up to 33), the
  same likeliest token at all 76, KL divergence 0; in 8 bits: the same likeliest token at 74 of 76, mean KL 0.002.
- Experts (a small Qwen 3.5/3.6 model with random weights, transformers' own code): within 0.0002, KL 0.
- Pictures, with `tools/refcheck/vision_reference.py` and `picture_reference.py` (examples/vision_check.rs and
  picture_check.rs): a picture of 500 by 375 pixels is read at 512 by 384 as the processor reads it (its resizing
  the same but for 118 of 589,824 bytes, each by 1), and becomes 192 rows; as published (16 bits), every row within
  0.00017 of the reference's (numbers up to 4.1). In 8 bits, every row's cosine with the reference's at least 0.90,
  within 0.73 at most (0.0055 on average); with the patch embedding in 8 bits too, at least 0.83 and within 1.69,
  which is why it's kept in 16 (any one other kind of weight kept in 16 helped about as much, but for a third to
  one and a half times more time). Asked about it,
  Qwen3.5-0.8B as published gives the same prompt (220 tokens), scores within 0.0003 at its last 16 positions (KL 0),
  and the same answer, token for token ("This image features a colorful geometric composition... 1. **Yellow
  Circle**"); in 8 bits, KL 0.005, the same likeliest token at all 16, its answer worded otherwise from its fourth
  token ("This image features several geometric shapes with distinct colors... on a colorful gradient background");
  its vision part in 16 bits and the rest in 8, the same answer as the reference's. Asked to read a scanned-looking
  invoice (900 by 520 pixels, slightly turned, specks on it: 448 rows), in 8 bits, the same answer as the
  reference's for its first 24 tokens, and the whole invoice read without a mistake. The vision part took 5.4 seconds
  for the first picture on this machine in 8 bits, 30 in 16 (whose arithmetic isn't vectorised).
- Speech recognition, with `tools/refcheck/whisper_reference.py` (Whisper written out in numpy from the published
  weights, with faster-whisper's own features, prompt and suppressed tokens; its tokens the same as faster-whisper's
  itself) and `examples/listen_check.rs`: whisper-tiny.en as published (16 bits) heard a recording of John F.
  Kennedy (11 seconds) as the same 23 tokens, its chance of no speech 0.0641932 against the reference's 0.0641933, its
  features within 0.000006 of faster-whisper's and its encoder's output within 0.00006 (numbers up to 11.9); in 8
  bits, the same tokens. whisper-small.en, Chat Computer's app's model, in 8 bits: the same tokens as the reference
  for Kennedy and for a sentence eSpeak said ("Hello chat computer, please set a timer for 10 minutes and tell me the
  weather in Singapore."). whisper-tiny, of many languages, in 16 bits: the same language found and the same tokens
  as the reference for French, Chinese and English. On this repository's test machine (2 cores), tiny.en took 1.2
  seconds for a recording (every recording is 30 seconds to the encoder), small.en 10.
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
a Mac yet); graphics cards (Vulkan, with the engine's own shaders); keys and values in 16 bits; pictures given to
`serve` (OpenAI's image_url, for Chat Computer's app: the engine sees them, the program doesn't take them yet);
videos.
