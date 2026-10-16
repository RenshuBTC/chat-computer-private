# ClaudeOS

ClaudeOS turns your computer into one you have a conversation with. You talk through your AirPods, or
type in the Claude app on your phone (the phone is the keyboard). It's one conversation either way. The
screen shows only your words and Claude's, plus whatever Claude puts there for you: websites, YouTube,
documents and your files.

* **On a Windows PC** it sits next to Windows. When you press the power button, ClaudeOS starts by
  itself. Windows stays installed, and you can go back to it whenever you like.
* **On a Mac** it takes over the whole Mac from the moment you log in: none of macOS shows, no Dock and
  no menu bar. macOS, your apps and your files stay as they are, and "switch to macOS" brings them back
  at once. It works on every Mac with Apple silicon, including the newest, and on Intel Macs with macOS 13
  or newer. See [ClaudeOS on a Mac](#claudeos-on-a-mac).

## What you need on a Windows PC

* This Windows PC. It must start in UEFI mode, which every PC sold in the last ten years does.
* About 70 GB of free disk space, or a spare internal disk.
* The internet. A network cable is best; Wi-Fi works if it uses a normal password.
* A Claude subscription (Pro or Max).
* Your AirPods, paired with Windows. ClaudeOS takes over the pairing.
* The Claude app on your phone (iPhone or Android), signed in with the same account, for typing.
* A keyboard and mouse for the setup. After that you won't need them.

## Installing on a Windows PC

1. **Unzip the ClaudeOS folder.** Right-click the download, choose **Extract All**, and open the
   extracted folder.
2. **Double-click `ClaudeOS Setup`.** If Windows says "Windows protected your PC", click
   **More info**, then **Run anyway**. Then click **Yes** when Windows asks for permission.
3. **Answer setup's questions.**
   * **Where ClaudeOS goes.** Normally that's next to Windows: setup shrinks drive C: to make room,
     like Windows' own "Shrink volume", and all your files stay.
   * **Signing in to Claude.** Your browser opens, and you sign in with your subscription account.
     Setup's window then shows a code that starts with `sk-ant-`. It may run over two lines: copy all of
     it, paste it back into setup and press Enter.
   * **Typing from your phone.** Setup asks whether to set up typing from the Claude app on your
     phone. Say yes: your browser opens once more; approve it with the same account.

   Nothing on your disks changes until you answer **y** to **Go ahead?**.
4. **Wait while setup prepares the installer.** Setup downloads the official Ubuntu installer, about
   4 GB. Ubuntu is what ClaudeOS uses to talk to your Wi-Fi, Bluetooth, sound and screen. Setup checks
   that the download is genuine, then puts the installer in the last 6 GB of ClaudeOS's space, a small
   temporary partition, and adds **ClaudeOS setup** to the PC's power-on menu. No USB stick is needed.
5. **Restart.** Setup asks to restart. The PC then starts the installer by itself, once. If Windows
   starts instead, restart and press the boot-menu key while the PC starts (often F12, F11, F8, F9 or
   Esc), then choose **ClaudeOS setup**.
6. **Let it install.** ClaudeOS installs itself without any questions, using only the free space.
   This usually takes well under an hour, depending on your internet. Don't turn the PC off. If the
   internet drops out near the end, the install still finishes; ClaudeOS completes the rest the next
   time the PC starts (see "Finishing the installation" below).
7. **Start ClaudeOS.** The PC restarts into ClaudeOS by itself. The first time it starts, ClaudeOS
   deletes the installer's temporary partition, takes over its space, and takes **ClaudeOS setup** out
   of the power-on menu. The installer never installs twice: if it starts again before that, its menu
   says **Start ClaudeOS** and starts it after 5 seconds.
8. **Wait for the voice download.** The first time ClaudeOS starts, it downloads its voice, about
   600 MB. Then it says hello.

Setup saves a **recovery password** in your Windows Documents folder, in
`ClaudeOS recovery password.txt`. You won't need it day to day; it's only for repairs.

## Using ClaudeOS

* **Talk or type.** Just speak, or type in the Claude app on your phone (see below). To wake ClaudeOS
  after it has gone to sleep, say "Claude" or press an AirPod stem.
* **Interrupt.** You can talk over Claude to stop it or change your mind.
* **Ask for anything.** For example:
  * "Play some calm piano music on YouTube"
  * "Find the tax letter I got in March"
  * "Connect to my phone's hotspot"
  * "Make the text bigger"
  * "I sit about two metres from the screen"
  * "Go to sleep"
* **Risky things need a yes.** Before anything risky, like deleting, buying, sending, installing
  or restarting, Claude asks and waits for your yes: out loud when you talked, in the chat when you
  typed. Most file changes can be undone: say "undo that".
* **Your files.** On a PC, everything on Windows' drive is there to open, and ClaudeOS offers to bring
  the folders you use over, so your files live in ClaudeOS from then on (Windows keeps its copies). If
  Windows' drive is encrypted with BitLocker, setup gives ClaudeOS a key of its own for it. On a Mac, it's your home folder.

## Typing: your phone is the keyboard

Open the Claude app on your phone, tap **Code**, and pick **ClaudeOS**. Type what you need. The PC does
the work, and the answer appears both on the PC's screen and in the app. Typed answers aren't spoken,
so it works the same at your desk or away from home. You can also attach photos and files.

* It's the same conversation as talking: switch between the two whenever you like.
* When something needs your OK, Claude asks in the chat. Answer **yes** or **no**.
* The PC has to be on and running ClaudeOS. If it was asleep or offline, the app reconnects when
  it's back.
* While the phone is connected, Anthropic keeps a copy of the conversation on its servers so your
  devices stay in step (this is how the Claude app's Remote Control works). To switch it off, say
  "turn off typing from my phone".
* Didn't set it up, or it stopped working? Run ClaudeOS Setup (on Windows, or in Terminal on a Mac),
  choose **Sign ClaudeOS in to Claude again**, and say yes to typing from your phone. ClaudeOS picks up
  the new sign-in when it next starts.

## Going back to Windows

You can go back for one restart, for every day, or for good.

* **For one restart:** say **"switch to Windows"**. The PC restarts straight into Windows. Restarting
  again from Windows brings you back to ClaudeOS. You can also choose **Windows** in the menu that shows
  for 3 seconds when the PC starts.
* **Windows every day, ClaudeOS still there:** say **"start Windows when the PC turns on"**, or run
  ClaudeOS Setup in Windows and choose **Choose what starts when the PC turns on**. To use ClaudeOS
  again, run ClaudeOS Setup and choose **Restart into ClaudeOS now, just this once**, or make it the
  first choice again the same way.
* **For good:** run ClaudeOS Setup in Windows and choose **Remove ClaudeOS from this PC**, then type
  REMOVE. Setup deletes ClaudeOS's partition and its boot files (and its installer, if it's still
  there), takes it out of the power-on menu, gives its space back to C: when it sits right after C:,
  and deletes what setup left on Windows (the download and the sign-in copies) and the BitLocker key it
  gave ClaudeOS. Windows, your apps and your Windows files stay as they are. Anything
  you saved only inside ClaudeOS is deleted with it.

If Windows uses BitLocker, Windows may ask for its recovery key once after ClaudeOS is installed.
Setup showed you this key and asked you to save it.

## If something goes wrong on a PC

* **Windows starts instead of the installer.** Restart, press the boot-menu key while the PC starts
  (often F12, F11, F8, F9 or Esc) and choose **ClaudeOS setup**. If it isn't listed, or the PC refuses
  to start it, look in the PC's firmware settings: some PCs need "Allow Microsoft third-party UEFI CA"
  (under Secure Boot) turned on, and "Fast Boot" turned off. To give up instead, run ClaudeOS Setup and
  choose **Remove ClaudeOS from this PC**: it deletes the installer and gives C: its space back.
* **Setup stops with an error after "Go ahead".** Setup takes back what it did (the installer, the
  power-on entry, and C:'s space), and says if anything is left. Run it again later.
* **The installer shows an error.** Windows and your files are untouched; the installer only ever
  uses free space. Take a photo of the screen, restart into Windows and run setup again.
* **The screen says "Finishing the installation".** The installer couldn't finish everything,
  usually because the internet dropped out or the PC was turned off. Leave the PC on: it keeps
  trying and restarts into ClaudeOS when it's done. If it keeps saying it didn't work, check that
  your Wi-Fi router is on and connected to the internet.
* **ClaudeOS shows a recovery screen.** Press Enter to try again. The screen lists what went wrong.
* **The PC starts Windows instead of ClaudeOS.** A big Windows update can put Windows first in the
  power-on menu again. Run ClaudeOS Setup and choose **Choose what starts when the PC turns on**.
* **The Claude app doesn't show ClaudeOS.** Check that the PC is on and in ClaudeOS, and that the app
  is signed in with the same account. If ClaudeOS showed a note that typing from the app can't reach
  it, sign in again as described under "Typing: your phone is the keyboard".
* **Claude says it isn't signed in.** A sign-in lasts about a year. To renew it:
  1. Say "switch to Windows".
  2. Run ClaudeOS Setup and choose **Sign ClaudeOS in to Claude again**.
  3. Restart into ClaudeOS.

## Removing ClaudeOS from a PC

Run ClaudeOS Setup in Windows and choose **Remove ClaudeOS from this PC** (see "Going back to Windows").
If setup can't do it, you can do it by hand in Windows:

1. Open **Disk Management** (right-click the Start button).
2. Delete the ClaudeOS partition. It's the large partition with no drive letter right after C:. If a
   6 GB partition called CLAUDEOS is there too (an install that didn't finish), delete it as well.
3. Right-click C: and choose **Extend Volume** to take the space back.
4. Optionally remove its start-up entries. In an administrator Terminal, run
   `bcdedit /enum firmware`, find the entries named "ubuntu" and "ClaudeOS setup", and run
   `bcdedit /delete {its id}` for each.

## ClaudeOS on a Mac

On a Mac, ClaudeOS takes over the whole Mac from the moment you log in. It fills the screen, and none
of macOS shows: no Dock, no menu bar, no app switcher, and the screen stays on while ClaudeOS is in front
(if it went dark, the Mac would lock, and you can't talk your way past a lock screen). Talking, typing from
your phone and asking for things work as described above. Your Mac's files are there for ClaudeOS to find.

macOS stays installed underneath, with your apps and files, and handles the Wi-Fi, the sound and your
AirPods. That's why it works on every Mac, including the newest: Apple's newest Macs can't start another
system yet, so on a Mac ClaudeOS doesn't replace macOS at power-on the way it replaces Windows on a PC. If
FileVault is on (it usually is), you type your Mac's password once when it starts, like a PC with
BitLocker; then ClaudeOS appears by itself.

ClaudeOS for Mac has been tried on GitHub's virtual Macs (Apple silicon and Intel, macOS 26): setup, the
app, a scripted conversation, switching to macOS and back, and removing it. It hasn't been tried with
AirPods on a physical Mac yet. If something doesn't work, the details are in the `logs` folder inside the
`ClaudeOS` folder in your home folder.

### What you need on a Mac

* A Mac with Apple silicon (M1 or newer) and macOS 14 Sonoma or newer, or an Intel Mac with macOS 13
  Ventura or newer.
* About 5 GB of free space, and the internet.
* A Claude subscription (Pro or Max).
* Your AirPods, connected to the Mac.
* The Claude app on your phone, signed in with the same account, for typing.

### Installing on a Mac

1. **Unzip the download.** Double-click it; you get a folder called `ClaudeOS` (usually in Downloads).
2. **Open Terminal** (in Applications, then Utilities), type `bash ~/Downloads/ClaudeOS/mac/setup.sh`
   and press Return. If the folder is somewhere else, type `bash ` and drag `setup.sh` from the folder's
   `mac` folder into Terminal. You can also double-click `ClaudeOS Setup.command` in the folder; if macOS
   won't open it because it came from the internet, use Terminal.
3. **Answer setup's questions.**
   * **Signing in to Claude.** Your browser opens, and you sign in with your subscription account. Setup
     then shows a code that starts with `sk-ant-`. It may run over two lines: copy all of it, paste it
     back into Terminal (it stays hidden) and press Return.
   * **Typing from your phone.** Say yes: your browser opens once more; approve it with the same account.
   * **Opening when you log in.** Say yes: that's what makes ClaudeOS take over the Mac from the moment you
     log in.

   Nothing changes on the Mac until you answer **y** to **Go ahead?**. Setup then installs ClaudeOS with
   its own copy of Python and its voice in the `ClaudeOS` folder in your home folder (about 3 GB), and
   makes the ClaudeOS app in the Applications folder in your home folder.
4. **Start ClaudeOS.** macOS asks a few questions the first time. Choose **Allow** for:
   * the microphone, so it can hear you;
   * your Desktop, Documents and Downloads folders and iCloud Drive, so it can find your files;
   * Location, when you first ask about Wi-Fi (macOS shows Wi-Fi names only to apps allowed to use it).

   To let it click and type on websites for you, turn on ClaudeOS in **System Settings**, **Privacy &
   Security**, **Accessibility** (macOS offers to take you there the first time it's needed).

### Going back to macOS

* **Right now:** say **"switch to macOS"**, or press Command-H. ClaudeOS steps aside and macOS shows, with
  everything as you left it. Say **"Claude"**, press an AirPod stem or click ClaudeOS in the Dock to come
  back. Command-Q quits ClaudeOS. If ClaudeOS ever stops responding, Option-Command-Esc (Force Quit) still
  works.
* **Every day:** say **"start macOS when the computer turns on"**, or run setup again and choose
  **Choose whether ClaudeOS opens when you log in**. ClaudeOS stays in Applications for when you want it.
* **For good:** run setup again, choose **Remove ClaudeOS from this Mac** and type REMOVE. Setup deletes
  the app, its Python and voice, its sign-in and its website sign-ins. It asks whether to keep what
  ClaudeOS remembers about you, its notes and your conversations. macOS, your apps and your files stay as
  they are.

### If something goes wrong on a Mac

* **Setup says the Mac is too old.** Update macOS in System Settings, then run setup again.
* **ClaudeOS doesn't open, or says it couldn't start.** Run setup again and choose **Update ClaudeOS**.
* **ClaudeOS doesn't hear you.** In **System Settings**, **Privacy & Security**, **Microphone**, turn on
  ClaudeOS. Check that your AirPods are connected to the Mac.
* **Claude says it isn't signed in.** Run setup and choose **Sign ClaudeOS in to Claude again**, then
  quit ClaudeOS (Command-Q) and open it again.
* **Notifications from other apps pop up over ClaudeOS.** Turn on a Focus (Control Center, **Focus**, for
  example **Do Not Disturb**) while you use ClaudeOS.

## For the curious

* `ARCHITECTURE.md` explains how it fits together.
* `windows/` holds the PC setup tool, `linux/` the thin Ubuntu base, `mac/` the Mac setup and app, and
  `claudeos/` ClaudeOS itself.
