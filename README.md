# Chat Computer

**A computer you talk to, with an operating system of its own and Meta's Muse Spark at its heart.**

Chat Computer is an operating system written from scratch in Rust. It has its own kernel, drivers, network stack,
TLS 1.3, file systems and model engine, and its whole interface is one conversation. You type or speak, and the
computer answers and acts. This is our entry to the **Meta Global AI Developer Hackathon**.

**Demo video:** [media/chat-computer-demo.mp4](media/chat-computer-demo.mp4) (3 minutes)

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

## Try it

In a virtual PC (Linux with QEMU and OVMF; the script builds the system with the Rust toolchain that
`os/rust-toolchain.toml` names):

```sh
META_API_KEY=your-key os/try-muse-spark.sh
```

Type a message and press Enter. To make it read your files, attach a disk image with QEMU.

On a PC with Chat Computer's own system installed, add this line to `\EFI\chatcomputer-own\settings.txt`:

```
meta-api-key = your-key
```

So far the own system has run in virtual PCs (QEMU). Real PCs come next on the roadmap.

## What's inside

| Folder | What it is |
| --- | --- |
| [os/boot](os/boot) | The UEFI start-up program, which carries the kernel |
| [os/kernel](os/kernel) | The kernel: memory, interrupts, all processor cores, drivers (NVMe, SATA, USB, HD Audio, network cards), the chat screen, the assistant and its tools |
| [os/net](os/net), [os/tls](os/tls), [os/web](os/web) | TCP/IP, DHCP and DNS; TLS 1.3 with X.509 checks; HTTP and pages |
| [os/model](os/model) | The model engine: Qwen's open models (with vision) and Whisper speech recognition, AVX2 and AVX-512 |
| [os/ntfs](os/ntfs), [os/fat](os/fat), [os/exfat](os/exfat), [os/bitlocker](os/bitlocker) | Reading Windows' and USB drives |
| [os/check.py](os/check.py) | Checks that boot the system in virtual PCs and test each part |
| [apps](apps), [docs](docs), [windows](windows) | The Windows app and setup, and the website |
| [chatcomputer](chatcomputer), [linux](linux) | The earlier Chat Computer, an app on a thin Linux base |

[os/README.md](os/README.md) describes the system in full, with its roadmap.

## Licence

Apache License 2.0: see [LICENSE](LICENSE). [NOTICE](NOTICE) lists what comes from others (fonts, Mozilla's root
certificates, Unicode data).
