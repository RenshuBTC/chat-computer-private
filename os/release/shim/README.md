# shim, for Secure Boot on a PC that hasn't approved Chat Computer's key

A PC sold with Windows starts, with Secure Boot on, only programs signed by a key in its firmware's allowed list:
Microsoft's. Chat Computer's own start-up file is signed by a key Windows makes on the PC itself, so the firmware
refuses it until that key is approved there. Linux distributions get past this with shim, a small first stage that
Microsoft signs: it starts the next stage once its key is in shim's own list (MokList), which the person approves
once, on MokManager's blue screen at a restart. Chat Computer's setup on Windows does the same
(`windows/lib/OwnOs.ps1`): it puts these files in `\EFI\chatcomputer-own`, Chat Computer's signed start-up file beside
them as `grubx64.efi` (the name shim starts), and asks for its key to be enrolled (the MokNew and MokAuth variables,
as mokutil writes them).

| File | What it is |
| --- | --- |
| `shimx64.efi` | shim 15.8, signed by Microsoft (Microsoft Corporation UEFI CA 2011): Ubuntu's `shimx64.efi.signed.latest` |
| `mmx64.efi` | MokManager, from the same shim, signed by Canonical (shim checks it with the certificate inside it) |

Both are taken unchanged from Ubuntu 24.04's package `shim-signed`, version `1.58+15.8-0ubuntu1`
(`/usr/lib/shim/` in it); `SHA256SUMS` has their SHA-256. Their SBAT generations (`shim,4` and `shim.ubuntu,1`) are
current. Chat Computer's start-up file says its own in its `.sbat` section (`os/boot/src/main.rs`), which shim
requires.

`./check.py --shim` starts Chat Computer through them in a virtual PC with Microsoft's keys and Secure Boot on: with
its key in MokList, without it (refused), and asked for as setup asks, approved on the blue screen.

shim is under the BSD 2-clause licence (`LICENSE`).
