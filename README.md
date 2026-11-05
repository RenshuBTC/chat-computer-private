# Chat Computer

**A computer you talk to, with an operating system of its own and Meta's Muse Spark at its heart.**

Our goal is a chat-centred computer: one conversation as the whole user experience of a full computer. You say what
you want, and it finds your files, opens sites, brings your messages from every chat app together and changes its
own settings, with everything it did listed so you can revert it. Underneath, the goal is an AI-native operating
system that is free and open source.

Chat Computer is an operating system written from scratch in Rust. It has its own kernel, drivers, network stack,
TLS 1.3, file systems and model engine, and its whole interface is one conversation. This is our entry to the
**Meta Global AI Developer Hackathon**.

**Chat Computer is still a work in progress.** The own system runs in virtual PCs so far; real PCs, more tools and
more of the everyday requests in our tests come next.

**Demo video (1:39):** [watch it](https://renshubtc.github.io/chat-computer/media/), or download
[media/chat-computer-demo.mp4](media/chat-computer-demo.mp4). It shows the full screen: your messages from every
chat app on the left, every action on the right (each one can be reverted), and websites like google.com opened by
typing them in the chat box.

## How it uses Meta's Muse Spark

With a key for [Meta's Model API](https://dev.meta.ai), Muse Spark (`muse-spark-1.3`) is the computer's brain.

* **The kernel talks to Meta itself.** Each round of an answer is a POST of the conversation, with the operating
  system's tools, to `api.meta.ai/v1/chat/completions`. The kernel makes it over its own TCP/IP and TLS 1.3, with no
  Linux, browser or HTTP library in between. It checks Meta's certificate against Mozilla's root store and asks
  again when the API reports a temporary error.
* **Muse Spark runs the computer through tool calls.** It can:
  * check how the computer is doing and change the text size;
  * set timers and reminders, and change the volume;
  * list, find and read files on the PC's drives (Windows' C: too, even with BitLocker);
  * search the web and read pages, and show pictures;
  * remember things about you, and start a new conversation;
  * restart or turn off the PC.

  Each tool is part of the operating system. Its result goes back to Muse Spark until the answer is done.
* **You can talk to it.** Press Windows+H and speak. Whisper turns your words into the message, and Muse Spark
  answers. Whisper runs on the PC in the system's own engine, from a model file on the PC's drive.
* **Local first.** Without a key, an open model (Qwen) answers on the PC itself, offline. With one, what you say
  goes to Meta, and the status line says so.

The code: [assistant.rs](os/kernel/src/assistant.rs) (`meta_step`, `meta_body`, `meta_answer`),
[web.rs](os/kernel/src/web.rs) (HTTPS requests), [http.rs](os/web/src/http.rs), [tls](os/tls),
[net](os/net), [tools.json](os/kernel/src/tools.json).

## Muse Spark on everyday requests

[evals/meta](evals/meta) tests how well Muse Spark runs the own system. It was built the way Hamel Husain's
[evals FAQ](https://hamel.dev/blog/posts/evals-faq/) recommends:

* 120 everyday requests (and 40 targeted ones), asked exactly as the kernel asks them, with a stand-in PC
  answering the tools;
* every trace read and labelled;
* what the labels showed fixed in the system;
* checks for what persists: code where a rule can tell, and one LLM judge, for unneeded tool calls. On held-out
  labels it catches 89% of real failures and passes 97% of good traces.

| On the 120 requests | Before | Now |
| --- | --- | --- |
| Passes every check | 38 | 111 |
| Words written with tool calls shown in the middle of the answer | 80 | 0 |
| Tool calls the request didn't need | 20 | 5 |
| Asks, or gives up, when its tools had the answer | 4 | 0 |
| Claims or suggests what can't be done here | 4 | 1 (now caught by the kernel) |
| "Turning it off now." with no tool call, in 50 power requests | 6 | 0 |

[evals/meta/README.md](evals/meta/README.md) has what was fixed, how far to trust each check, and what's still
open, such as which currency "300 dollars" means.

## Try it

In a virtual PC (Linux with QEMU and OVMF; the script builds the system with the Rust toolchain that
`os/rust-toolchain.toml` names):

```sh
META_API_KEY=your-key os/try-muse-spark.sh
```

Type a message and press Enter. To make it read your files, attach a disk image with QEMU.

Chat Computer's full screen is also on [chatcomputer.ai](https://chatcomputer.ai), in the browser. It shows your
messages from every app on the left and Actions on the right, where what was changed can be reverted. Websites open
when you type their address. Press the spark beside the message box and sign in with your Meta key, and Meta's
models (Muse Wonder or Muse Spark) answer there too.

On a PC with Chat Computer's own system installed, add this line to `\EFI\chatcomputer-own\settings.txt`:

```
meta-api-key = your-key
```

## What's inside

| Folder | What it is |
| --- | --- |
| [os/boot](os/boot) | The UEFI start-up program, which carries the kernel |
| [os/kernel](os/kernel) | The kernel: memory, interrupts, all processor cores, drivers (NVMe, SATA, USB, HD Audio, network cards), the chat screen, the assistant and its tools |
| [os/net](os/net), [os/tls](os/tls), [os/web](os/web) | TCP/IP, DHCP and DNS; TLS 1.3 with X.509 checks; HTTP and pages |
| [os/model](os/model) | The model engine: Qwen's open models (with vision) and Whisper speech recognition, AVX2 and AVX-512 |
| [os/ntfs](os/ntfs), [os/fat](os/fat), [os/exfat](os/exfat), [os/bitlocker](os/bitlocker) | Reading Windows' and USB drives |
| [os/check.py](os/check.py) | Checks that boot the system in virtual PCs and test each part |
| [evals](evals) | Tests of the models on everyday requests ([evals/meta](evals/meta): Meta's models) |
| [apps](apps), [docs](docs), [windows](windows) | The Windows and Mac app, the own system's setup, and the website |
| [chatcomputer](chatcomputer), [linux](linux) | The earlier Chat Computer, an app on a thin Linux base |

[os/README.md](os/README.md) describes the system in full, with its roadmap.

## Built with frontier models from Anthropic, OpenAI and Meta

Chat Computer is built with today's frontier models, and it works with each of them:

* **Anthropic's Claude** writes and reviews much of the code with us (Claude Code), and answers in Chat Computer
  with an Anthropic API key.
* **OpenAI's models** help build it too, and answer in Chat Computer with an OpenAI API key, or in the app when you
  sign in with your ChatGPT plan.
* **Meta's Muse Spark and Muse Wonder** are the own system's brain through Meta's Model API, and answer on
  [chatcomputer.ai](https://chatcomputer.ai). Muse Spark is tested in [evals/meta](evals/meta), and is the judge
  there for one check, validated against human labels.

Without a key, an open model (Qwen) answers on the computer itself, offline.

## Contributors

* RenshuBTC, the author
* Claude (Anthropic)
* OpenAI's GPT models
* Muse Spark and Muse Wonder (Meta)

GitHub's own contributor list shows only accounts tied to the commits, so the models are listed here.

## Licence

Apache License 2.0: see [LICENSE](LICENSE). [NOTICE](NOTICE) lists what comes from others (fonts, Mozilla's root
certificates, Unicode data).
