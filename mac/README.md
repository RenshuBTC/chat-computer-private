# ClaudeOS for Mac

On a Mac, ClaudeOS is an app that fills the screen. macOS, your apps and your files stay as they are. The
steps for people are in the main `README.md` ("ClaudeOS on a Mac"); this file is for anyone looking under
the hood. **It hasn't been tried on a real Mac yet**: everything here was built and checked on Linux, with
stand-ins for macOS's commands (see "Tests").

| File | What it does |
|---|---|
| `setup.sh` | Setup, run in Terminal (`bash mac/setup.sh`, or double-click `ClaudeOS Setup.command` next to this folder). Written for macOS's bash 3.2. Install or update, sign in to Claude again, choose whether ClaudeOS opens at login, remove ClaudeOS. The log is `~/ClaudeOS/logs/setup.log`; it never contains the sign-in. |
| `requirements-mac.txt` | The Python packages, at the same versions as on Linux, plus PyObjC for the window and the Mac's Wi-Fi, Bluetooth and keys. Intel Macs get onnxruntime 1.23.2, the last version with Intel Mac builds. |
| `ClaudeOS.app` | The app, built and signed ahead of time (ad hoc) and the same for every Mac; setup copies it to `~/Applications` as it is. Nothing is signed on the Mac, since that needs Apple's developer tools. |
| `app/Info.plist` | The app's name, id (`com.claudeos.app`) and the reasons macOS shows when ClaudeOS asks for the microphone, Bluetooth, Location (Wi-Fi names) and Apple events; plain-http websites allowed in its web views. |
| `launcher/launcher.c` | The app's main program, `Contents/MacOS/ClaudeOS`. It loads ClaudeOS's Python library and runs `python -P -m claudeos run` in its own process, so macOS sees one app called ClaudeOS: its Dock icon, and the permissions it asks for, with the reasons in its `Info.plist`. It reads where Python is from `~/ClaudeOS/launch.conf`, which setup writes. |
| `launcher/build.sh` | Builds `ClaudeOS.app`: the program for Apple silicon and for Intel with Zig's C compiler, which can make Mac programs on any computer (`pip install ziglang`), joined into one universal program (`universal.py`), then signed with rcodesign (apple-codesign). `BUILT-WITH.txt` records the versions and SHA-256s. |
| `ClaudeOS.icns`, `make_icon.py` | The app icon: the word ClaudeOS in black on a white tile. |

## What setup installs

| What | Where |
|---|---|
| The app | `~/Applications/ClaudeOS.app` |
| ClaudeOS's code | `~/ClaudeOS/app` |
| Its own Python 3.12 (from uv, Astral's Python installer, checked against a fixed SHA-256) | `~/ClaudeOS/python`, with the packages in `~/ClaudeOS/runtime`; `~/ClaudeOS/launch.conf` tells the app where they are |
| What ClaudeOS keeps (sign-in, settings, voice, memory, notes, logs) | `~/ClaudeOS`, as on a PC |
| Opening at login (if you say yes) | `~/Library/LaunchAgents/com.claudeos.start.plist` |
| Website sign-ins and cookies | WebKit's store for `com.claudeos.app` in `~/Library` |

Requirements: Apple silicon with macOS 14 or newer (the speech packages' Apple silicon builds need it), or
an Intel Mac with macOS 13 or newer; about 5 GB free; the internet.

## Switching back to macOS

* "Switch to macOS" (or Command-H) hides ClaudeOS; the Mac's desktop shows. Saying "Claude", pressing an
  AirPod stem or clicking ClaudeOS in the Dock brings it back. Command-Q quits it.
* "Start macOS when the computer turns on" (or setup's third choice) removes the LaunchAgent, so the Mac
  starts as usual; ClaudeOS stays in Applications.
* Setup's "Remove ClaudeOS from this Mac" (type REMOVE) quits ClaudeOS, signs its own copy of the Claude
  engine out (which also removes that sign-in from the Keychain), and deletes the app, the LaunchAgent,
  ClaudeOS's website data and `~/ClaudeOS`. It can keep what ClaudeOS remembers, its notes and your
  conversations.

## Tests

* `tests/test_mac_setup.py` runs `setup.sh` against a pretend Mac: stand-ins for `sw_vers`, `uname`,
  `sysctl`, `df`, `curl`, `xattr`, `launchctl` and the others, uv and the Claude engine, in a temporary
  home folder (`CLAUDEOS_MAC_ROOT`, `CLAUDEOS_UV`, `CLAUDEOS_ENGINE_CLI`, `CLAUDEOS_SKIP_MODELS`).
* `tests/test_mac_launcher.py` builds `launcher.c` for Linux and runs it with a stand-in ClaudeOS, and
  checks the app's program is one signed universal program, for Apple silicon and Intel, that uses only
  the system library.
* `tests/test_mac.py` covers the Mac engine (`claudeos/platform/mac.py`) with stand-ins for macOS's
  commands, and the tools and instructions Claude gets on a Mac.
