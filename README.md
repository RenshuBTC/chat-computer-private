# Chat Computer

Chat Computer turns your computer into one you have a conversation with. You touch the screen as you would an
iPad's, type on a keyboard or in the Claude app on your phone (the phone is the keyboard), or talk through
your AirPods. It's one conversation either way. The screen shows your words and Claude's, your messages from
every account at the left, what's been done for you at the right, and whatever Claude puts there for you:
websites, YouTube, documents and your files.

**[Try it in your browser](https://chatcomputer.ai)**: Chat Computer's own screen (see
[The demo](#the-demo)).

* **On a Windows PC** it sits next to Windows. When you press the power button, Chat Computer starts by
  itself. Windows stays installed, and you can go back to it whenever you like.
* **On a Mac with Apple silicon (M1, M2 or M3)** it sits next to macOS the same way: when the Mac
  starts, Chat Computer starts. macOS, your apps and your files stay, one restart away. See
  [Chat Computer on a Mac](#chat-computer-on-a-mac).

## What you need on a Windows PC

* This Windows PC. It must start in UEFI mode, which every PC sold in the last ten years does.
* About 70 GB of free disk space, or a spare internal disk.
* The internet. A network cable is best; Wi-Fi works if it uses a normal password.
* A Claude subscription (Pro or Max).
* Your AirPods, paired with Windows. Chat Computer takes over the pairing.
* The Claude app on your phone (iPhone or Android), signed in with the same account, for typing.
* A keyboard and mouse for the setup. After that you won't need them.

## Installing on a Windows PC

1. **Unzip the Chat Computer folder.** Right-click the download, choose **Extract All**, and open the
   extracted folder.
2. **Double-click `Chat Computer Setup`.** The first time, Windows warns that "the publisher could not be
   verified" (or says "Windows protected your PC": click **More info**). Click **Run**. Then click
   **Yes** when Windows asks for permission.
3. **Sign in to Claude.** A small window called "Sign in to Claude" opens, then your browser. Sign in
   with the account that has your subscription and approve. That's all: setup picks up the sign-in by
   itself, and it also lets you type to Chat Computer from the Claude app on your phone.
4. **Check the list and type `y`.** Setup downloads the official Ubuntu installer (about 4 GB; Ubuntu
   is what Chat Computer uses to talk to your Wi-Fi, Bluetooth, sound and screen) and checks it's genuine.
   Then it lists everything it's about to change. Nothing on your disks changes until you answer **y**
   to **Go ahead?**. Then setup:
   * **makes room next to Windows.** It shrinks drive C:, like Windows' own "Shrink volume"; all
     your files stay. Chat Computer gets 64 GB when there's room (at least 40). If files Windows keeps at
     the end of C: are in the way, setup moves them out of the way first: it resets the drive's
     change journal, and only if that isn't enough, deletes your restore points (Windows makes new
     ones) and turns hibernation off. If something it can't move is in the way, such as the page
     file, setup stops, puts everything back, and tells you exactly what to do.
   * puts the installer in the last 6 GB of Chat Computer's space, a small temporary partition, and adds
     **Chat Computer setup** to the PC's power-on menu. No USB stick is needed.
   * turns off Windows "Fast Startup", which would keep Chat Computer from saving to your Windows files.

   If your PC has a second internal disk, setup first asks where Chat Computer should go. On a PC without a
   network cable whose Wi-Fi password Windows doesn't know, it asks for a Wi-Fi network.
5. **Restart.** Setup asks to restart. The PC then starts the installer by itself, once. If Windows
   starts instead, restart and press the boot-menu key while the PC starts (often F12, F11, F8, F9 or
   Esc), then choose **Chat Computer setup**.
6. **Let it install.** Chat Computer installs itself without any questions, using only the free space.
   This usually takes well under an hour, depending on your internet. Don't turn the PC off. If the
   internet drops out near the end, the install still finishes; Chat Computer completes the rest the next
   time the PC starts (see "Finishing the installation" below).
7. **Start Chat Computer.** The PC restarts into Chat Computer by itself. The first time it starts, Chat Computer
   deletes the installer's temporary partition, takes over its space, and takes **Chat Computer setup** out
   of the power-on menu. The installer never installs twice: if it starts again before that, its menu
   says **Start Chat Computer** and starts it after 5 seconds.
8. **Wait for the voice download.** The first time Chat Computer starts, it downloads its voice, about
   600 MB. Then it says hello.

Setup saves a **recovery password** in your Windows Documents folder, in
`Chat Computer recovery password.txt`. You won't need it day to day; it's only for repairs.

## Using Chat Computer

* **Touch, type or talk.** Touch the screen (see [The touchscreen](#the-touchscreen)), type on a keyboard
  or in the Claude app on your phone (see below), or just speak. What you type is answered in writing only;
  what you say gets a spoken answer. To wake Chat Computer after it has gone to sleep, say "Claude", press an
  AirPod stem, or just type.
* **AirPods.** When Chat Computer starts, it connects your AirPods by itself: the ones it already knows, or the
  pairing Windows has, copied over without asking. If they're in their case, take them out and they connect
  within two minutes. Only when there are no AirPods anywhere does it ask "Connect your AirPods?"; say or type
  yes and hold the button on the back of the case. Later, "connect my AirPods" does it in one step. Through the
  PC's own speakers Chat Computer doesn't greet you out loud.
* **The screen.** Like the Claude app: your messages on the right in grey bubbles, Claude's on the left, from
  the top of the screen down, on a warm off-white that's easier on the eyes than white, and a message box at the
  bottom. What you send moves to the top of the screen with the answer coming in under it, its words fading in as
  they come, so you never scroll down to read it; scroll up for anything earlier. Under each message, as in the
  Claude app: under Claude's, copy, read aloud, thumbs up or down (kept, to make Chat Computer better) and ask
  again; under yours, ask again, edit and copy; with when it was. Before
  anything is said, its name, **Chat Computer**, is in the middle, and the conversation starts with its first
  words, the classic `hello, world`; the name goes once you say something.
  The empty message box says, dimmed, "Chat computer anything or type “/” to search", with the cursor already
  blinking in it. The round button at its right sends: grey while there's
  nothing to send, black once there is. There are no menus: everything is done by asking. Scroll up (a finger,
  the mouse wheel, Page Up) to read back.
* **What Claude opens** (a website, a video, a document) goes between the two panels, in a rounded frame,
  above your latest message and Claude's latest reply, both in full. F1 makes it fill the screen and F1 again
  puts it back; Escape, or a swipe up from the bottom edge, closes it. Typing "/" to search shows its list in
  front of it.
* **Messages.** On a wide screen, a panel at the left gathers what's sent to you from every account you
  connect, newest first: Gmail, Outlook and Hotmail, iCloud Mail, other email, Telegram, WhatsApp, X,
  Instagram, Facebook, TikTok and LinkedIn. Tap one to read it between the panels (an email shows as the
  conversation with whoever sent it). Tap + to connect an account: see [Connecting accounts](#connecting-accounts).
* **Actions.** A panel at the right shows today's date and the time, and lists everything done for you,
  by day, each with its time: sites opened, web searches, files changed, settings, programs. Tap one to
  scroll back to the message that asked for it, even from an earlier day. Ask "what did I change yesterday?"
  and Claude reads the same list.
* **Pins.** Pin an item in either panel (the pin by it, or swipe it left on a touchscreen) to keep it at the top.
* **The panels' switches.** Each panel has a switch like the Claude app's sidebar button: Messages' at the top
  left, Actions' at the top right and, while something is open, the conversation's at the bottom right.
  Its icon is filled in while the panel shows. Switched off, the panel gives its room to the rest; typing
  brings the conversation back. Asking works too ("hide the sidebars").
* **"/" searches this computer.** Type / first in the message box and what you type after it searches your
  files by name, in a list above the box; the arrow keys choose and Enter opens it.
* **Interrupt.** You can talk over Claude to stop it or change your mind.
* **Ask for anything.** For example:
  * "Play some calm piano music on YouTube"
  * "Find the tax letter I got in March"
  * "Connect to my phone's hotspot"
  * "Make the text bigger"
  * "Use my webcam's microphone" or "switch to Sonnet"
  * "I sit about two metres from the screen"
  * "Go to sleep"
* **Risky things need a yes.** Before anything risky, like deleting, buying, sending, installing
  or restarting, Claude asks and waits for your yes: out loud when you talked, in the chat when you
  typed. Most file changes can be undone: say "undo that".
* **Your files.** On a PC, everything on Windows' drive is there to open, and Chat Computer offers to bring
  the folders you use over, so your files live in Chat Computer from then on (Windows keeps its copies). If
  Windows' drive is encrypted with BitLocker, setup gives Chat Computer a key of its own for it. On a Mac,
  setup copies the folders you choose onto a drive called ChatComp, which macOS opens too; in Chat Computer
  they're in your Mac folder.

## The touchscreen

Chat Computer is made to be touched first, as an iPad is (the gestures are the iPad's, from Apple's iPad User Guide):

* **Tap** to tap, **move a finger** to scroll (a quick swipe keeps it going; a touch stops it), **pinch** to
  zoom a website or to make the conversation's text bigger or smaller (it stays that size), and **double-tap**
  a photo to zoom in.
* **Swipe up from the bottom edge** for Home: what's open closes and the conversation is back.
* **Swipe in from the left edge** (or right along the bottom edge) to go back; from the right edge (or left
  along the bottom) to go forward.
* **Swipe down from the top centre** for Messages, when the screen is too narrow to show it.
* **Three fingers:** pinch closed to copy, twice to cut, pinch open to paste; swipe left to undo, right to
  redo. **Four or five fingers:** pinch closed for Home, swipe to go back or forward.
* **Text:** double-tap a word to select it, triple-tap for its paragraph, and hold a message to select all
  of it. Hold the message box and move your finger to move the cursor.
* **The keyboard:** touch the message box, or a box on a website, and a keyboard laid out like an iPad's
  comes up along the bottom. Its hide key puts it away, and so does typing on a real keyboard: while one is
  in use, it doesn't come up. As on an iPad, tap Shift twice for Caps Lock and Space twice for a full stop;
  the microphone key types what you say next.
* With a second monitor, the touchscreen's touches still land on the screen Chat Computer fills.

## Connecting accounts

Tap + at the top of Messages (or ask "connect my Gmail"). Sign-ins are typed into Chat Computer's own form and
kept on the computer (in a folder Claude's tools can't open); Claude never sees them. Chat Computer only reads:
it never sends or deletes anything, and email stays unread in your account until you read it there. (A
WhatsApp or social media conversation opens on the site's own page, which may mark it read, as the site does.)

* **Gmail, iCloud Mail, Yahoo:** with an app password made in your account's security settings (Gmail's
  needs 2-Step Verification on). Gmail shows the Primary tab only.
* **Outlook and Hotmail:** Microsoft's own sign-in page. It needs a free sign-in app registered once in
  Microsoft Entra (personal accounts, a "Mobile and desktop" redirect of
  `https://login.microsoftonline.com/common/oauth2/nativeclient`, public client flows on, and the
  IMAP.AccessAsUser.All permission); Chat Computer shows the steps and asks for its Application (client) ID.
* **Other email:** any provider with IMAP: its server, your address and password.
* **Telegram:** an api_id and api_hash of your own (from my.telegram.org, "API development tools"), your phone
  number, and the code Telegram sends. Channels aren't shown, only chats and groups.
* **WhatsApp, X, Instagram, Facebook, TikTok, LinkedIn:** you sign in on the site's own page (WhatsApp: scan
  its code with your phone), and Chat Computer reads the list of conversations from it. These sites don't allow
  automated reading, so they could restrict an account that's read this way. (X's own API, which allows it,
  charges per message read.)

## Typing: your phone is the keyboard

Open the Claude app on your phone, tap **Code**, and pick **Chat Computer**. Type what you need. The PC does
the work, and the answer appears both on the PC's screen and in the app. Typed answers aren't spoken,
so it works the same at your desk or away from home. You can also attach photos and files.

* It's the same conversation as talking: switch between the two whenever you like.
* When something needs your OK, Claude asks in the chat. Answer **yes** or **no**.
* The PC has to be on and running Chat Computer. If it was asleep or offline, the app reconnects when
  it's back.
* While the phone is connected, Anthropic keeps a copy of the conversation on its servers so your
  devices stay in step (this is how the Claude app's Remote Control works). To switch it off, say
  "turn off typing from my phone".
* Stopped working? Run Chat Computer Setup (in Windows, or in macOS on a Mac) and choose **Sign Chat Computer in
  to Claude again**. Chat Computer picks up the new sign-in when it next starts. (On a Mac, say yes to typing
  from your phone.)

## AirTouch: your hands in the air

AirTouch is off unless you ask for it: say or type "turn on AirTouch". If your screen has a camera (an Apple
Studio Display's, a laptop's, or any webcam), you can then use Chat Computer with your hands, without touching
anything:

* **A dot follows each index finger.** Point at the screen with one hand or both.
* **Tap:** pinch your thumb and index finger together and let go. **Scroll:** pinch and move your hand
  up or down, as if dragging the page.
* **Stop:** hold up an open hand, palm to the screen, for a moment. Chat Computer stops talking and a video
  pauses. When Chat Computer is asleep, this wakes it.
* **Yes or no:** a thumbs up or thumbs down, held a moment, answers a question Chat Computer asked.
* **Keyboard:** hold a hand out palm up and curl all four fingers towards you once, as if beckoning
  someone: the keyboard comes up along the bottom of the screen. Do the same with your palm facing down to
  put it away (or tap its hide key, or say "hide the keyboard"; "show the keyboard" brings it up too).
* **Typing in the air:** the keyboard is laid out exactly like an iPad's, every key in the same place and
  the same size. Your hands arrive on it the way they rest on a keyboard: index fingers on F and J, the
  other fingers on the keys beside them. Each index finger keeps its dot; move a hand to move its fingers
  across the keys, and tap any finger down to type the key it's on. As on an iPad, tap Shift twice for
  Caps Lock, and Space twice for a full stop. The microphone key types what you say next instead of
  sending it to Claude, and the smiley key has emoji.
* The camera's pictures are looked at on the computer only, never kept or sent anywhere. Say "turn off
  AirTouch" to switch it off again.

An Apple Studio Display's camera works with Chat Computer on a PC connected with a Thunderbolt or USB-C cable.
Center Stage is part of macOS, so the picture doesn't follow you around. On a Mac, Chat Computer can't use a
Studio Display yet (see [Chat Computer on a Mac](#chat-computer-on-a-mac)).

## A keyboard and a mouse

A USB keyboard and mouse work as soon as they're plugged in. For Bluetooth ones, put them in pairing mode
and say "pair my keyboard" (or "my mouse"). A keyboard may want a code typed on it: Chat Computer shows the code
and says it. Type it on the keyboard, then press Enter.

* On the conversation screen, typing goes into the message box at the bottom: Enter sends, Shift+Enter
  starts a new line, Escape clears it, and the cursor, selecting and pasting work as anywhere else. The
  answer is written, not spoken, and questions like "Go ahead?" are answered by typing yes or no. On a
  website, typing goes into the box you're in.
* The mouse's arrow shows when you move the mouse and hides once it has been still for 3 seconds. Each screen
  runs at the fastest refresh rate it offers (as most are set in Windows), so the arrow moves as smoothly as
  there. If a screen flickers or goes black at that rate, create an empty file `~/ChatComputer/keep-refresh-rate`
  (ask Claude to) and restart.

## Going back to Windows

You can go back for one restart, for every day, or for good.

* **For one restart:** say **"switch to Windows"**. The PC restarts straight into Windows. Restarting
  again from Windows brings you back to Chat Computer. You can also choose **Windows** in the menu that shows
  for 3 seconds when the PC starts.
* **Windows every day, Chat Computer still there:** say **"start Windows when the PC turns on"**, or run
  Chat Computer Setup in Windows and choose **Choose what starts when the PC turns on**. To use Chat Computer
  again, run Chat Computer Setup and choose **Restart into Chat Computer now, just this once**, or make it the
  first choice again the same way.
* **For good:** run Chat Computer Setup in Windows and choose **Remove Chat Computer from this PC**, then type
  REMOVE. Setup deletes Chat Computer's partition and its boot files (and its installer, if it's still
  there), takes it out of the power-on menu, gives its space back to C: when it sits right after C:,
  and deletes what setup left on Windows (the download, unless you keep it to install Chat Computer again,
  and the sign-in copies) and the BitLocker key it gave Chat Computer. Windows, your apps and your Windows
  files stay as they are. Anything you saved only inside Chat Computer is deleted with it.

If Windows uses BitLocker, Windows may ask for its recovery key once after Chat Computer is installed.
Setup showed you this key and asked you to save it.

## Updating Chat Computer on a PC

Put the new version of Chat Computer in the folder you ran Chat Computer Setup from (download it again over the old
one), then start Chat Computer. It asks on the screen whether to update: say or type yes, and it starts again
on the new version a few seconds later. If the new version won't start, Chat Computer goes back to the one
before by itself. A version that needs new parts of Linux says so instead: install it with Chat Computer Setup.

## If something goes wrong on a PC

* **Windows starts instead of the installer.** Restart, press the boot-menu key while the PC starts
  (often F12, F11, F8, F9 or Esc) and choose **Chat Computer setup**. If it isn't listed, or the PC refuses
  to start it, look in the PC's firmware settings: some PCs need "Allow Microsoft third-party UEFI CA"
  (under Secure Boot) turned on, and "Fast Boot" turned off. To give up instead, run Chat Computer Setup and
  choose **Remove Chat Computer from this PC**: it deletes the installer and gives C: its space back.
* **Setup stops with an error after "Go ahead".** Setup takes back what it did (the installer, the
  power-on entry, and C:'s space), and says if anything is left. Run it again later. (Restore points it
  deleted to make room stay deleted; Windows makes new ones.)
* **Setup says Windows can't make room.** It names what's in the way. Windows' page file: follow the
  steps setup shows to turn it off for a moment, restart, and run setup again. The file system's own
  records: Chat Computer needs a separate disk, or a partition tool that works while Windows is off.
* **The "Sign in to Claude" window or your browser doesn't open.** Press Enter in setup's window and
  choose **Try signing in again**, or **Paste a sign-in code instead**: setup shows a line to run in an
  ordinary PowerShell window, which prints a code; paste it into setup with a right-click.
* **The installer shows an error.** Windows and your files are untouched; the installer only ever
  uses free space. Take a photo of the screen, restart into Windows and run setup again.
* **The screen says "Finishing the installation".** The installer couldn't finish everything,
  usually because the internet dropped out or the PC was turned off. Leave the PC on: it keeps
  trying and restarts into Chat Computer when it's done. If it keeps saying it didn't work, check that
  your Wi-Fi router is on and connected to the internet.
* **Chat Computer shows a recovery screen.** Press Enter to try again. The screen lists what went wrong.
* **You want to see what Chat Computer did.** It copies the end of its logs to Windows' drive, a minute after
  it starts, every 10 minutes and before it restarts. In Windows they're in
  `C:\Users\<you>\AppData\Local\ChatComputer\from-chatcomputer`. Sign-ins and passwords are blanked out, and the
  conversation isn't in them.
* **The PC starts Windows instead of Chat Computer.** A big Windows update can put Windows first in the
  power-on menu again. Run Chat Computer Setup and choose **Choose what starts when the PC turns on**.
* **The Claude app doesn't show Chat Computer.** Check that the PC is on and in Chat Computer, and that the app
  is signed in with the same account. If Chat Computer showed a note that typing from the app can't reach
  it, sign in again as described under "Typing: your phone is the keyboard".
* **Claude says it isn't signed in.** A sign-in lasts about a year. To renew it:
  1. Say "switch to Windows".
  2. Run Chat Computer Setup and choose **Sign Chat Computer in to Claude again**.
  3. Restart into Chat Computer.

## Removing Chat Computer from a PC

Run Chat Computer Setup in Windows and choose **Remove Chat Computer from this PC** (see "Going back to Windows").
If setup can't do it, you can do it by hand in Windows:

1. Open **Disk Management** (right-click the Start button).
2. Delete the Chat Computer partition. It's the large partition with no drive letter right after C:. If a
   6 GB partition called CHATCOMP is there too (an install that didn't finish), delete it as well.
3. Right-click C: and choose **Extend Volume** to take the space back.
4. Optionally remove its start-up entries. In an administrator Terminal, run
   `bcdedit /enum firmware`, find the entries named "ubuntu" and "Chat Computer setup", and run
   `bcdedit /delete {its id}` for each.

## Chat Computer on a Mac

On a Mac with Apple silicon, Chat Computer sits next to macOS the way it sits next to Windows on a PC. When the
Mac starts, Chat Computer starts; macOS, your apps and your files stay, one restart away. Talking, typing from
your phone and asking for things work as described above.

On a Mac, Chat Computer is built on Fedora Asahi Remix, the Linux for Apple silicon Macs made by the Asahi Linux
project and Fedora, and setup installs it with their official installer. That decides which Macs it works
on:

* **M1 and M2 Macs:** everything Chat Computer needs works.
* **M3, M3 Pro and M3 Max Macs:** Chat Computer works with limits for now: no graphics acceleration (pages and
  videos are slower), no brightness control, and no second screen. On the iMac with M3, the speakers and
  microphones don't work yet; AirPods do.
* **Not yet:** Macs with M4 or M5, the Mac Studio with M3 Ultra, and Intel Macs.
* A separate screen (a Mac mini's, say) must be connected to the Mac's HDMI port: screens connected over
  USB-C or Thunderbolt don't work in Asahi Linux yet.

Chat Computer can't open macOS's own files. On a Mac with Apple silicon they're always encrypted, FileVault or
not, with keys kept by the Mac's Secure Enclave, and Linux can't use those yet. So setup copies the folders
you choose onto a drive called **ChatComp** that both systems open. In Chat Computer they're
in your Mac folder; in macOS, the ChatComp drive shows in Finder. What you save there in one system is
there in the other. To bring more folders over later, run setup in macOS again.

Chat Computer for Mac hasn't been tried on a real Mac yet. Its setup has been tested on a pretend Mac and, in
parts, on GitHub's virtual Macs (which can't start Linux). Its first start and installation have been tested
on an emulated Mac running the real Fedora image, through to Chat Computer's screen. The step in the Mac's startup
options, Wi-Fi, sound and the Mac's own hardware can only be tried on a real Mac. Whether AirPods' microphone
works in Asahi Linux isn't known yet; the built-in microphones of M1, M2 and M3 laptops do work.

### What you need on a Mac

* A Mac with M1, M2 or M3 (see above) and macOS 13.5 or newer.
* About 80 GB of free space in macOS, plus the size of the folders you bring. Chat Computer gets 60 GB when
  there's room (30 GB at least), the Chat Computer drive gets your folders plus 10 GB, and macOS keeps 38 GB
  free for its updates.
* Wi-Fi with a normal password: Chat Computer finishes installing itself from the internet the first time it
  starts.
* A Claude subscription (Pro or Max).
* The Claude app on your phone, signed in with the same account, for typing.
* Your AirPods. In Chat Computer, put them in pairing mode (lid open, button on the back held until the light
  flashes white) and say or type "pair my AirPods". You may have to pair them again in macOS afterwards.

### Installing on a Mac

1. **Unzip the download.** Double-click it; you get a folder called `Chat Computer` (usually in Downloads).
2. **Open Terminal** (in Applications, then Utilities), type `bash ~/Downloads/ChatComputer/mac/setup.sh`
   and press Return. If the folder is somewhere else, type `bash ` and drag `setup.sh` from the folder's
   `mac` folder into Terminal. You can also double-click `Chat Computer Setup.command` in the folder; if macOS
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
   shrinking macOS's partition, like Disk Utility does, and installs Chat Computer's base there. Type your Mac's
   password when asked (it stays hidden). This takes 10 to 20 minutes, and the Mac may seem to freeze while
   macOS's partition shrinks. Then setup makes the Chat Computer drive and copies your folders onto it.
5. **Do the one step Apple requires.** Setup shows these steps and shuts the Mac down. Take a photo of
   them:
   1. When the Mac has turned off, wait 25 seconds.
   2. Press and hold the power button. Let go when "Loading startup options..." appears.
   3. Click **Chat Computer**, then **Continue**.
   4. If it asks you to "Select a volume to recover", choose your usual macOS (Macintosh HD), click
      **Next** and type your Mac's password.
   5. A window "Chat Computer installer (second step)" opens. Press Enter, then type your Mac's user name and
      password when asked (it asks twice). The note about the security level is about Chat Computer only:
      macOS keeps its own.
   6. At "Installation complete! Press enter to reboot.", press Enter.

   If the Mac starts macOS instead, start again from the first of these steps.
6. **Let Chat Computer set itself up.** On its first start, Chat Computer makes your account, joins your Wi-Fi and
   restarts. It then installs the rest from the internet (20 to 40 minutes), restarts once more, downloads
   its voice (about 600 MB) and says hello.

Setup saves a **recovery password** in your Documents folder in macOS, in `Chat Computer recovery password.txt`.
You won't need it day to day; it's only for repairs.

### Going back to macOS

* **For one restart:** say **"switch to macOS"**. Chat Computer asks, then the Mac restarts into macOS.
  Restarting from macOS brings you back to Chat Computer. You can also hold the power button while the Mac
  starts and choose **Macintosh HD**.
* **macOS every day, Chat Computer still there:** say **"start macOS when the Mac turns on"**, or run setup in
  macOS and choose **Choose what starts when the Mac turns on**. To use Chat Computer then, hold the power button
  while the Mac starts and choose **Chat Computer**, or make it the first choice again the same way.
* **For good:** run setup in macOS, choose **Remove Chat Computer from this Mac** and type REMOVE. If the Chat Computer
  drive has files on it, setup first offers to copy them into a folder called `Chat Computer files` in your home
  folder. It then makes macOS the start-up disk, deletes Chat Computer, its partitions and the Chat Computer drive,
  and gives their space back to macOS. macOS, your apps and the files in your macOS folders stay as they
  are.

### If something goes wrong on a Mac

* **Setup says this Mac can't run Chat Computer.** Chat Computer starts on its own only on Macs with M1, M2 or M3
  (not M3 Ultra). Asahi Linux, which it's built on, doesn't support newer Macs yet.
* **Setup says macOS's partition couldn't be made smaller.** Time Machine's local snapshots can hold on to
  space. See https://alx.sh/tmcleanup, then run setup again.
* **The installer stopped part way.** Run setup again and choose **Remove Chat Computer from this Mac** to
  clean up, then install again.
* **The Mac starts macOS instead of Chat Computer.** Do the startup-options steps above again, or run setup and
  choose **Choose what starts when the Mac turns on**.
* **The screen says "Finishing the installation".** Chat Computer needs the internet to finish. Leave the Mac on
  near your Wi-Fi: it keeps trying and restarts into Chat Computer when it's done.
* **The Mac shows Fedora's own setup questions instead of Chat Computer.** Chat Computer's first start didn't work
  three times in a row. Restart into macOS (hold the power button, choose Macintosh HD), run setup, remove
  Chat Computer and install it again.
* **Claude says it isn't signed in.** Say "switch to macOS", run setup and choose **Sign Chat Computer in to
  Claude again**, then restart into Chat Computer.

## The demo

**https://chatcomputer.ai** is Chat Computer's screen in a browser, for trying it before
installing it: the word logo, Chat Computer saying `hello, world`, and the message box with the cursor in it. It is the same page
and code as on a Chat Computer (`docs/index.html`: `scripts/build_demo.py` copies them, and
`tests/test_demo.py` checks the copy is current), with `docs/demo/host.js` standing in for the computer. The
messages and files there are made up. Everyone gets answers, free:

* **Written answers**, in `host.js`: what Chat Computer is, what it needs and costs (it's free, on a Claude Pro
  or Max plan; the download is coming soon), and what the page can show: music (a few soft chords the browser
  makes), maps, the weather and the air, timers, sums, the date and time, the news (Wikipedia's Current events),
  Wikipedia's article for a question of fact or "look up …", the made-up messages and files. Asked for something
  only an AI can do (a recipe, a poem, a translation), it says so, and how to get one.
* **The browser's own AI**, where there is one: Chrome on a computer has Gemini Nano built in, and Edge
  Phi-4-mini, running on the device itself. The **Download Gemini** button (in Edge, Download Phi-4-mini) at the
  right of the message box shows while the browser can download it; Chrome downloads it once, after that click,
  and keeps it for every site. Then it answers what the written answers can't, and can put a page it writes, a map
  or a timer on the screen; its answers, long ones too, are in the conversation. It can't reach the internet
  itself, so the page looks things up for it: when a question
  needs something recent or that it isn't sure of, or they say "look it up", it says what to look up, the page
  searches Wikipedia, the day's news (Wikipedia's Current events) and, for haze or pollution, the air
  (Open-Meteo), and it answers from what was found, saying where it's from. The page's own answers go into its
  conversation too, so a follow-up ("the haze?" after the weather) makes sense to it.

What's typed stays in the browser, except a place's name asked about for a map, the weather or the air, which goes
to Open-Meteo's free geocoding and forecasts (the map itself comes from OpenStreetMap), the words looked up on
Wikipedia, and with the two other buttons at the right of the message box (`docs/demo/demo.css`):

* **Connect Claude**: paste an API key from Claude Console, and Claude (Opus 5.5, as on a Chat Computer, but at
  the API's own effort, medium, rather than max, for quicker answers) answers for real and does things on the screen: plays YouTube videos (YouTube's player between the panels;
  when the video ends, the conversation has the middle again), shows maps and pages it writes, opens the
  made-up messages and files, makes timers, searches the web. Browsers let a page play sound by itself only
  after a click, tap or key on the site, so a video starts without sound if there hasn't been one yet, and the
  first click, tap or key (on the video too) turns it on. Requests go from the browser straight to
  Anthropic's API; the key stays in that browser, and what's used is billed to it. A Claude Pro or Max plan
  can't be used: Anthropic doesn't allow other developers' apps to sign in with one. (Sign in with ChatGPT
  would let visitors use a ChatGPT plan, but OpenAI offers it to hosted sites only after approval.)
* **Connect AirPods**: you talk and the answers are read out. The browser listens through whatever microphone
  the device uses (your AirPods, once they're paired in its Bluetooth settings: a website can't pair them) and
  turns speech into text itself (Chrome does it on Google's servers). Chrome, Edge and Safari can; Firefox
  can't.

Chat Computer is an independent project, not made by Anthropic.

GitHub Pages serves it from the `docs` folder (Settings, Pages, branch `main`, folder `/docs`; a private
repository needs GitHub Pro) at chatcomputer.ai: `docs/CNAME` names the domain, and its DNS points at GitHub
Pages (A records for the domain itself, a CNAME record for `www`).

## For the curious

* `ARCHITECTURE.md` explains how it fits together.
* `windows/` holds the PC setup tool, `mac/` the Mac setup, `linux/` the thin Linux base (Ubuntu on a PC,
  Fedora Asahi Remix on a Mac), and `chatcomputer/` Chat Computer itself.
* `os/` is Chat Computer's own operating system, being written from scratch to replace the Linux base. Its
  README says what runs so far, what no operating system can own on a PC, and the steps ahead.
* `evals/` says whether a change made things better. `evals/os` gives Chat Computer's Claude everyday requests on
  a simulated computer, each in its own sandbox (`python -m evals.os.run OUT_DIR`, Opus 5.5 at max effort), and
  `evals/web` types a first-time visitor's questions into the website (`node evals/web/run.js OUT.json`). Each
  has pass/fail checks (`evaluate.py`), written from reading the answers and kept honest against hand labels
  (`labels/`).
