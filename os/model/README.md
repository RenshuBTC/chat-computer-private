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
  tokens it chose.
- `convert.rs` and the program's `convert` command: the model's file from Qwen's published files (config.json,
  tokenizer.json, the chat template, the safetensors weights), a weight at a time.
- `src/bin/chat-computer-model`: the program: `serve` answers this computer's programs over HTTP on 127.0.0.1 as
  OpenAI's chat completions API (streamed or whole, tool calls, the thinking as `reasoning_content`); `convert`.

## The program

    chat-computer-model convert MODEL_FOLDER OUT.ccm [--bits 8|4|16]
    chat-computer-model serve MODEL.ccm [--port 8765] [--threads N] [--allow-origin ORIGIN]...

`serve` answers only requests addressed to this computer by its own name (127.0.0.1, localhost: not a site's name
pointed here by a DNS trick), from programs on it or from Chat Computer's own pages (chatcomputer.ai, the desktop
app's page, and origins given with `--allow-origin`, such as the Chrome extension's); any other page a browser shows is
refused, so no site can use the model behind the person's back. The model only writes: it runs and opens nothing.

## Checks

`cargo test -p model --features std`: on two small models made with random weights (one with experts), reading many
tokens at once gives exactly the scores of reading them one at a time; a snapshot goes back exactly; a session reads
only what's new and answers exactly as a new session does; the kernels (plain, AVX2, AVX-VNNI, AVX-512 VNNI) agree;
the chat template's rules; NFC; JSON.

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
Chat Computer); graphics cards (Vulkan, with the engine's own shaders); keys and values in 16 bits.
