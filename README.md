# ClaudeOS

ClaudeOS turns your computer into one you have a conversation with. You talk through your AirPods, or
type in the Claude app on your phone (the phone is the keyboard). It's one conversation either way. The
screen shows only your words and Claude's, plus whatever Claude puts there for you: websites, YouTube,
documents and your files.

* **On a Windows PC** it sits next to Windows. When you press the power button, ClaudeOS starts by
  itself. Windows stays installed, and you can go back to it whenever you like.
* **On a Mac with Apple silicon (M1, M2 or M3)** it sits next to macOS the same way: when the Mac
  starts, ClaudeOS starts. macOS, your apps and your files stay, one restart away. See
  [ClaudeOS on a Mac](#claudeos-on-a-mac).

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
  Windows' drive is encrypted with BitLocker, setup gives ClaudeOS a key of its own for it. On a Mac,
  setup copies the folders you choose onto a drive called ClaudeOS, which macOS opens too; in ClaudeOS
  they're in your Mac folder.

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
* Didn't set it up, or it stopped working? Run ClaudeOS Setup (in Windows, or in macOS on a Mac),
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

On a Mac with Apple silicon, ClaudeOS sits next to macOS the way it sits next to Windows on a PC. When the
Mac starts, ClaudeOS starts; macOS, your apps and your files stay, one restart away. Talking, typing from
your phone and asking for things work as described above.

On a Mac, ClaudeOS is built on Fedora Asahi Remix, the Linux for Apple silicon Macs made by the Asahi Linux
project and Fedora, and setup installs it with their official installer. That decides which Macs it works
on:

* **M1 and M2 Macs:** everything ClaudeOS needs works.
* **M3, M3 Pro and M3 Max Macs:** ClaudeOS works with limits for now: no graphics acceleration (pages and
  videos are slower), no brightness control, and no second screen. On the iMac with M3, the speakers and
  microphones don't work yet; AirPods do.
* **Not yet:** Macs with M4 or M5, the Mac Studio with M3 Ultra, and Intel Macs.
* A separate screen (a Mac mini's, say) must be connected to the Mac's HDMI port: screens connected over
  USB-C or Thunderbolt don't work in Asahi Linux yet.

ClaudeOS can't open macOS's own files. On a Mac with Apple silicon they're always encrypted, FileVault or
not, with keys kept by the Mac's Secure Enclave, and Linux can't use those yet. So setup copies the folders
you choose onto a drive called **ClaudeOS** that both systems open. In ClaudeOS they're
in your Mac folder; in macOS, the ClaudeOS drive shows in Finder. What you save there in one system is
there in the other. To bring more folders over later, run setup in macOS again.

ClaudeOS for Mac hasn't been tried on a real Mac yet. Its setup has been tested on a pretend Mac and, in
parts, on GitHub's virtual Macs (which can't start Linux). Its first start and installation have been tested
on an emulated Mac running the real Fedora image, through to ClaudeOS's screen. The step in the Mac's startup
options, Wi-Fi, sound and the Mac's own hardware can only be tried on a real Mac. Whether AirPods' microphone
works in Asahi Linux isn't known yet; the built-in microphones of M1, M2 and M3 laptops do work.

### What you need on a Mac

* A Mac with M1, M2 or M3 (see above) and macOS 13.5 or newer.
* About 80 GB of free space in macOS, plus the size of the folders you bring. ClaudeOS gets 60 GB when
  there's room (30 GB at least), the ClaudeOS drive gets your folders plus 10 GB, and macOS keeps 38 GB
  free for its updates.
* Wi-Fi with a normal password: ClaudeOS finishes installing itself from the internet the first time it
  starts.
* A Claude subscription (Pro or Max).
* The Claude app on your phone, signed in with the same account, for typing.
* Your AirPods. In ClaudeOS, put them in pairing mode (lid open, button on the back held until the light
  flashes white) and say or type "pair my AirPods". You may have to pair them again in macOS afterwards.

### Installing on a Mac

1. **Unzip the download.** Double-click it; you get a folder called `ClaudeOS` (usually in Downloads).
2. **Open Terminal** (in Applications, then Utilities), type `bash ~/Downloads/ClaudeOS/mac/setup.sh`
   and press Return. If the folder is somewhere else, type `bash ` and drag `setup.sh` from the folder's
   `mac` folder into Terminal. You can also double-click `ClaudeOS Setup.command` in the folder; if macOS
   won't open it because it came from the internet, use Terminal.
3. **Answer setup's questions.**
   * **Your files.** Which of your folders to bring over: Desktop, Documents, Downloads, Pictures, Movies
     and Music. If macOS asks whether Terminal may open them, choose **OK**.
   * **Signing in to Claude.** Your browser opens, and you sign in with your subscription account. Setup
     then shows a code that starts with `sk-ant-`. It may run over two lines: copy all of it, paste it
     back into Terminal (it stays hidden) and press Return.
   * **Typing from your phone.** Say yes: your browser opens once more; approve it with the same account.
   * **Wi-Fi.** Setup shows the Wi-Fi the Mac uses and asks for its password. Press Return to let setup
     look it up in the Mac's keychain instead (macOS may ask for your Mac's password).

   Nothing changes on the Mac until you answer **y** to **Go ahead?**.
4. **Let setup install.** Setup downloads the Asahi installer and Fedora Asahi Remix (about 1.2 GB), checks
   that both are genuine, and runs the installer, answering its questions. It makes room next to macOS by
   shrinking macOS's partition, like Disk Utility does, and installs ClaudeOS's base there. Type your Mac's
   password when asked (it stays hidden). This takes 10 to 20 minutes, and the Mac may seem to freeze while
   macOS's partition shrinks. Then setup makes the ClaudeOS drive and copies your folders onto it.
5. **Do the one step Apple requires.** Setup shows these steps and shuts the Mac down. Take a photo of
   them:
   1. When the Mac has turned off, wait 25 seconds.
   2. Press and hold the power button. Let go when "Loading startup options..." appears.
   3. Click **ClaudeOS**, then **Continue**.
   4. If it asks you to "Select a volume to recover", choose your usual macOS (Macintosh HD), click
      **Next** and type your Mac's password.
   5. A window "ClaudeOS installer (second step)" opens. Press Enter, then type your Mac's user name and
      password when asked (it asks twice). The note about the security level is about ClaudeOS only:
      macOS keeps its own.
   6. At "Installation complete! Press enter to reboot.", press Enter.

   If the Mac starts macOS instead, start again from the first of these steps.
6. **Let ClaudeOS set itself up.** On its first start, ClaudeOS makes your account, joins your Wi-Fi and
   restarts. It then installs the rest from the internet (20 to 40 minutes), restarts once more, downloads
   its voice (about 600 MB) and says hello.

Setup saves a **recovery password** in your Documents folder in macOS, in `ClaudeOS recovery password.txt`.
You won't need it day to day; it's only for repairs.

### Going back to macOS

* **For one restart:** say **"switch to macOS"**. ClaudeOS asks, then the Mac restarts into macOS.
  Restarting from macOS brings you back to ClaudeOS. You can also hold the power button while the Mac
  starts and choose **Macintosh HD**.
* **macOS every day, ClaudeOS still there:** say **"start macOS when the Mac turns on"**, or run setup in
  macOS and choose **Choose what starts when the Mac turns on**. To use ClaudeOS then, hold the power button
  while the Mac starts and choose **ClaudeOS**, or make it the first choice again the same way.
* **For good:** run setup in macOS, choose **Remove ClaudeOS from this Mac** and type REMOVE. If the ClaudeOS
  drive has files on it, setup first offers to copy them into a folder called `ClaudeOS files` in your home
  folder. It then makes macOS the start-up disk, deletes ClaudeOS, its partitions and the ClaudeOS drive,
  and gives their space back to macOS. macOS, your apps and the files in your macOS folders stay as they
  are.

### If something goes wrong on a Mac

* **Setup says this Mac can't run ClaudeOS.** ClaudeOS starts on its own only on Macs with M1, M2 or M3
  (not M3 Ultra). Asahi Linux, which it's built on, doesn't support newer Macs yet.
* **Setup says macOS's partition couldn't be made smaller.** Time Machine's local snapshots can hold on to
  space. See https://alx.sh/tmcleanup, then run setup again.
* **The installer stopped part way.** Run setup again and choose **Remove ClaudeOS from this Mac** to
  clean up, then install again.
* **The Mac starts macOS instead of ClaudeOS.** Do the startup-options steps above again, or run setup and
  choose **Choose what starts when the Mac turns on**.
* **The screen says "Finishing the installation".** ClaudeOS needs the internet to finish. Leave the Mac on
  near your Wi-Fi: it keeps trying and restarts into ClaudeOS when it's done.
* **The Mac shows Fedora's own setup questions instead of ClaudeOS.** ClaudeOS's first start didn't work
  three times in a row. Restart into macOS (hold the power button, choose Macintosh HD), run setup, remove
  ClaudeOS and install it again.
* **Claude says it isn't signed in.** Say "switch to macOS", run setup and choose **Sign ClaudeOS in to
  Claude again**, then restart into ClaudeOS.

## For the curious

* `ARCHITECTURE.md` explains how it fits together.
* `windows/` holds the PC setup tool, `mac/` the Mac setup, `linux/` the thin Linux base (Ubuntu on a PC,
  Fedora Asahi Remix on a Mac), and `claudeos/` ClaudeOS itself.
