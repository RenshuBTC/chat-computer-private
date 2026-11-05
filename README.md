# Chat Computer

Chat Computer turns your computer into one you have a conversation with. You touch the screen as you would an
iPad's, type on a keyboard, or talk through your AirPods. It's one conversation either way. The screen shows
your words and Chat Computer's, your messages from every account at the left, what's been done for you at the
right, and whatever Chat Computer puts there for you: websites, YouTube, documents and your files.

It thinks with an open model that runs on the computer itself (one of Qwen's, chosen for the computer's memory,
run by Chat Computer's own engine): there's no account, no sign-in and nothing to pay, and what you say stays on
the computer. If you'd rather it used an AI you already have, the spark beside the message box's + signs in with
it: your ChatGPT plan (in the app, as T3 Code does), OpenRouter (whose free models cost nothing, and which has the
best model of every maker below), or a key of your own from any maker at the top of
[Artificial Analysis's Intelligence Index](https://artificialanalysis.ai/leaderboards/models): Google (Gemini),
Anthropic (Claude), OpenAI, Meta, Moonshot (Kimi), xAI (Grok), Alibaba (Qwen), DeepSeek, Z.ai (GLM), Xiaomi
(MiMo), MiniMax or Mistral, each answering with its best model; then what you say goes to that AI, on your account.

**[Try it in your browser](https://chatcomputer.ai)**: Chat Computer's own screen (see
[The demo](#the-demo)).

**[Get it](https://chatcomputer.ai/get)**: on Windows, one Setup, which asks how you want Chat Computer: as a Chrome
extension, as your web browser, as an app, or as its own system, next to Windows. On a Mac, the same from the Mac app's
next version; until then /get has the Mac app (see [The apps](#the-apps)).

* **On a Windows PC** it sits next to Windows. When you press the power button, Chat Computer starts by
  itself. Windows stays installed, and you can go back to it whenever you like.
* **On a Mac with Apple silicon (M1, M2 or M3)** it sits next to macOS the same way: when the Mac
  starts, Chat Computer starts. macOS, your apps and your files stay, one restart away. See
  [Chat Computer on a Mac](#chat-computer-on-a-mac).

## What you need on a Windows PC

* This Windows PC. It must start in UEFI mode, which every PC sold in the last ten years does.
* About 70 GB of free disk space, or a spare internal disk.
* The internet. A network cable is best; Wi-Fi works if it uses a normal password.
* At least 8 GB of memory (16 GB or more for a smarter model; see [The model](#the-model)).
* Your AirPods, paired with Windows. Chat Computer takes over the pairing.
* A keyboard and mouse for the setup. After that you won't need them.

## Installing on a Windows PC

1. **Get Chat Computer Setup.** Download it from [chatcomputer.ai/get](https://chatcomputer.ai/get) (**Windows**) and
   open it. The first time, Windows says "Windows protected your PC": click **More info**, then **Run anyway**. It
   installs the Chat Computer app for your account, then asks how you want Chat Computer: choose **Its own system**.
   (Or, with the Chat Computer folder: right-click the download, choose **Extract All**, and double-click
   `Chat Computer Setup` in its `windows` folder; Windows may warn that "the publisher could not be verified":
   click **Run**.)
2. **Let Chat Computer Setup start.** It opens in a window of its own; click **Yes** when Windows asks for
   permission.
3. **Check the list and type `y`.** Setup downloads the official Ubuntu installer (about 4 GB; Ubuntu
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
4. **Restart.** Setup asks to restart. The PC then starts the installer by itself, once. If Windows
   starts instead, restart and press the boot-menu key while the PC starts (often F12, F11, F8, F9 or
   Esc), then choose **Chat Computer setup**.
5. **Let it install.** Chat Computer installs itself without any questions, using only the free space.
   This usually takes well under an hour, depending on your internet. Don't turn the PC off. If the
   internet drops out near the end, the install still finishes; Chat Computer completes the rest the next
   time the PC starts (see "Finishing the installation" below).
6. **Start Chat Computer.** The PC restarts into Chat Computer by itself. The first time it starts, Chat Computer
   deletes the installer's temporary partition, takes over its space, and takes **Chat Computer setup** out
   of the power-on menu. The installer never installs twice: if it starts again before that, its menu
   says **Start Chat Computer** and starts it after 5 seconds.
7. **Wait for the downloads.** The first time Chat Computer starts, it downloads its voice (about 600 MB) and
   then its model (see [The model](#the-model): from 2 to 56 GB, so from a few minutes to an hour or more). It
   says how far along it is; until the model is ready it can only do simple things, like Wi-Fi, the volume
   and switching to Windows.

Setup saves a **recovery password** in your Windows Documents folder, in
`Chat Computer recovery password.txt`. You won't need it day to day; it's only for repairs.

## The model

Chat Computer thinks with one of Qwen's open models (Apache 2.0 licence), run on the computer's own processor
by Chat Computer's own engine. It picks the biggest one the computer's memory runs well and its disk has room
for, and downloads it from Qwen's own files the first time (each file checked against a fingerprint pinned in
Chat Computer):

| Memory | Model | Download |
|---|---|---|
| 48 GB or more | Qwen3.8 27B | 56 GB |
| 32 GB | Qwen3.6 35B-A3B | 72 GB |
| 16 GB | Qwen3.5 9B | 19 GB |
| 8 GB | Qwen3.5 4B | 9 GB |
| less | Qwen3.5 0.8B | 2 GB |

The download is turned into Chat Computer's own smaller file (4 bits a weight) and then deleted. To use
another one, ask ("use the 4B model"). While a bigger one is being got ready, the one already there answers.
Nothing you say leaves the computer; only websites you open and web searches use the internet.

The model answers whatever you ask: Qwen's models are trained to refuse some requests, and Chat Computer takes
that out of each one as it gets it ready (it finds the direction the model refuses along, and the engine takes it
out as the model runs; the model's own files stay as Qwen published them). On Qwen3.5 0.8B, of 12 such requests
kept aside to measure it, the model refused 9 as published and none after (see
[os/model/README.md](os/model/README.md#answering-what-its-asked)).

## Using Chat Computer

* **Touch, type or talk.** Touch the screen (see [The touchscreen](#the-touchscreen)), type on a keyboard, or
  just speak. What you type is answered in writing only; what you say gets a spoken answer. To wake Chat
  Computer after it has gone to sleep, say "Computer", press an AirPod stem, or just type.
* **AirPods.** When Chat Computer starts, it connects your AirPods by itself: the ones it already knows, or the
  pairing Windows has, copied over without asking. If they're in their case, take them out and they connect
  within two minutes. Only when there are no AirPods anywhere does it ask "Connect your AirPods?"; say or type
  yes and hold the button on the back of the case. Later, "connect my AirPods" does it in one step. Through the
  PC's own speakers Chat Computer doesn't greet you out loud.
* **The screen.** Like the Claude app: your messages on the right in bubbles, Chat Computer's on the left, from
  the top of the screen down, in the tones of Claude's dark mode on a PC (its middle #191919, its sidebars a shade
  lighter, its words #F0F0F0), and a message box at the bottom. What you send moves to the top of the screen with the answer coming in under it, its words fading in as
  they come, so you never scroll down to read it; scroll up for anything earlier. Under each message, as in the
  Claude app: under Chat Computer's, copy, read aloud, thumbs up or down (kept, to make Chat Computer better) and ask
  again; under yours, ask again, edit and copy; with when it was. A
  conversation starts with its name, **Chat Computer**, alone in the middle; after a second it fades away, and
  Chat Computer's first words come in as if just typed, as its answers do: the classic `hello, world`, alone.
  The empty message box, lifted off the dark screen as the Claude app's is, says, dimmed, "Type anything", with
  the cursor already blinking in it (a "/" first in it searches your files; Escape puts Messages and Actions at
  the sides away, and brings them back). Under the words, as in the Claude app, each in a circle of its own:
  **+** at the left adds files to your message (Chat Computer reads them), and the **microphone** at the right,
  beside the send button, starts or stops listening; neither has a menu. A site's address typed alone (`youtube.com`, `open bbc.co.uk`, a whole link) opens it at once,
  in the middle, as a browser's address bar would. The round button at its right sends: grey while there's
  nothing to send, white once there is. There are no menus: everything is done by asking. Scroll up (a finger,
  the mouse wheel, Page Up) to read back.
* **What Chat Computer opens** (a website, a video, a document, a message) takes the conversation's place in the
  middle, from the top of the screen down to the message box, in a rounded frame with its name and a close
  button (×) at the top; the conversation comes back when it's closed (with the ×, a swipe up from the bottom
  edge, or by asking), and when something you type is answered in words alone (what's open is put away, music
  still playing). It's a page like any other: scroll it, and type in it (a website has the keys until you
  tap the message box; Page Up and Down in the box scroll it too). There are only ever three parts, side by
  side: Messages, the middle and Actions. To start with, the middle fills the screen, message box and all;
  **Escape** brings the panels in at its sides, and Escape again puts them away. Typing "/" to search shows its list in front of it.
* **Messages.** On a wide screen, a panel at the left gathers what's sent to you from every account you
  connect, newest first: Gmail, Outlook and Hotmail, iCloud Mail, other email, Telegram, WhatsApp, WeChat, X,
  Instagram, Facebook, TikTok and LinkedIn. Tap one to read it in the middle: the whole conversation with
  whoever sent it. Tap + to connect an account: see [Connecting accounts](#connecting-accounts).
* **Actions.** A panel at the right shows today's date and the time, and lists everything done for you,
  by day, each with its time: sites opened, web searches, files changed, settings, programs. Tap one to
  scroll back to the message that asked for it, even from an earlier day. Ask "what did I change yesterday?"
  and Chat Computer reads the same list.
* **Pins.** Pin an item in either panel (the pin by it, or swipe it left on a touchscreen) to keep it at the top.
* **The panels' switches.** Each panel has a switch like the Claude app's sidebar button: Messages' at the top
  left, Actions' at the top right and, while something is open, the conversation's at the bottom right.
  Its icon is filled in while the panel shows. Switched off, the panel gives its room to the rest; typing
  brings the conversation back. Asking works too ("hide the sidebars").
* **"/" searches this computer.** Type / first in the message box and what you type after it searches your
  files by name, in a list above the box; the arrow keys choose and Enter opens it.
* **Interrupt.** You can talk over Chat Computer to stop it or change your mind.
* **Ask for anything.** For example:
  * "Play some calm piano music on YouTube"
  * "Find the tax letter I got in March"
  * "Connect to my phone's hotspot"
  * "Make the text bigger"
  * "Use my webcam's microphone" or "use the 4B model"
  * "I sit about two metres from the screen"
  * "Go to sleep"
* **Risky things need a yes.** Before anything risky, like deleting, buying, sending, installing
  or restarting, Chat Computer asks and waits for your yes: out loud when you talked, in the chat when you
  typed. Most file changes can be undone: say "undo that". Or, in Actions at the right, point at what was
  done and press Revert, beside its pin: files go back as they were (edited, saved, moved, copied, trashed,
  new folders), and so do settings (volume, mute, brightness, night light, Wi-Fi, Bluetooth, time zone,
  timers, VPN, what's remembered, programs installed or removed). The revert is listed too, so it can be
  reverted in turn.
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
* **Swipe down from the top centre** for Messages, when it isn't showing (a screen too narrow for it, or with its
  switch off).
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
kept on the computer (in a folder the model's tools can't open); the model never sees them. Chat Computer only reads:
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
* **WhatsApp, WeChat, X, Instagram, Facebook, TikTok, LinkedIn:** you sign in on the site's own page (WhatsApp
  and WeChat: scan its code with your phone), and Chat Computer reads the list of conversations from it. WeChat's
  is WeChat for Web: WeChat doesn't let every account use it (it says so on the page when yours can't), and it
  signs out now and then (scan again). These sites don't allow
  automated reading, so they could restrict an account that's read this way. (X's own API, which allows it,
  charges per message read.)

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
  sending it to Chat Computer, and the smiley key has emoji.
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
  starts a new line, and the cursor, selecting and pasting work as anywhere else; Escape puts Messages and
  Actions at the sides away (and brings them back). The answer is written, not spoken, and questions like "Go ahead?" are answered by
  typing yes or no. On a website, typing goes into the website, into the box you're in.
* The mouse's arrow shows when you move the mouse and hides once it has been still for 3 seconds. Each screen
  runs at the fastest refresh rate it offers (as most are set in Windows), so the arrow moves as smoothly as
  there. If a screen flickers or goes black at that rate, create an empty file `~/ChatComputer/keep-refresh-rate`
  (ask Chat Computer to) and restart.

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
* **Chat Computer says it's still getting its model ready.** The model's download is big: leave Chat Computer on
  and connected to the internet; a download that stops carries on where it stopped. If it says there isn't room,
  it picks a smaller model by itself; free some space (or ask it what takes the space) for a bigger one.

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
  starts, and downloads its model (see [The model](#the-model)).
* Your AirPods. In Chat Computer, put them in pairing mode (lid open, button on the back held until the light
  flashes white) and say or type "pair my AirPods". You may have to pair them again in macOS afterwards.

### Installing on a Mac

1. **Get Chat Computer Setup.** Open Terminal (in Applications, then Utilities), paste
   `curl -fsSL https://chatcomputer.ai/get/mac.sh | bash` and press Return. It puts the Chat Computer app in
   Applications and opens it; it asks how you want Chat Computer: choose **Its own system**. (On a Mac that can't
   have it, it says why. The Mac app on chatcomputer.ai/get is still the one from before it asked, until GitHub's
   Macs build the new one: until then, use the Chat Computer folder, below.)
2. **Setup opens in Terminal.** (Or, with the Chat Computer folder: unzip the download, then in Terminal type
   `bash ~/Downloads/ChatComputer/mac/setup.sh` and press Return; if the folder is somewhere else, type `bash ` and
   drag `setup.sh` from the folder's `mac` folder into Terminal. You can also double-click
   `Chat Computer Setup.command` in the folder; if macOS won't open it because it came from the internet, use
   Terminal.)
3. **Answer setup's questions.**
   * **Your files.** Which of your folders to bring over: Desktop, Documents, Downloads, Pictures, Movies
     and Music. If macOS asks whether Terminal may open them, choose **OK**.
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
   its voice (about 600 MB) and its model, and says hello.

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

## The demo

**https://chatcomputer.ai** is Chat Computer's screen in a browser, for trying it before
installing it: the word logo for a second, then Chat Computer saying `hello, world`, and the message box with the cursor in it. It is the same page
and code as on a Chat Computer (`docs/index.html`: `scripts/build_demo.py` copies them, and
`tests/test_demo.py` checks the copy is current), with `docs/demo/host.js` standing in for the computer. The
messages and files there are made up. Everyone gets answers, free:

* **Written answers**, in `host.js`: what Chat Computer is, what it needs and costs (it's free: its own model runs
  on your computer, with no account; the download is coming soon), and what the page can show: music (a few soft chords the browser
  makes), maps, the weather and the air, timers, sums, the date and time, the news (Wikipedia's Current events),
  Wikipedia's article for a question of fact or "look up …", the made-up messages and files. Google (its search,
  for what Wikipedia hasn't got too), Google Maps' directions and Wikipedia open for real in the middle, to scroll
  and type in: they're the sites that let another page show them (`framed()` in host.js; Google's own addresses
  for that, `igu=1` and `output=embed`). A site's address typed (`youtube.com`, `open bbc.co.uk`, a whole link)
  opens it, as a browser's address bar would, whichever of these answers: Google and Wikipedia in the middle, a
  YouTube video's address in YouTube's player there, and any other site at once in a new tab beside the page (most
  sites don't let another page show them; on a Chat Computer every site opens in the middle). Asked for something
  only its model can do (a recipe, a poem, a translation), it says so, and how to get it.
* **This computer's model**, where the Chat Computer app runs on the same computer: typing **/** lists **Use this
  computer's model**. Chosen, the page talks to the app's engine at 127.0.0.1 (the browser may ask once whether
  chatcomputer.ai can reach other programs on the device: allowed, it can), and Chat Computer's own model answers
  for real, its words streaming in, and does things on the screen: plays YouTube videos (YouTube's player between
  the panels; when the video ends, the conversation has the middle again) or shows YouTube's search, plays calm
  piano, shows maps and pages it writes, says the weather, opens the made-up messages and files, makes timers, and
  looks things up (Wikipedia, the day's news, the air: only the words looked up leave the computer). Browsers let a
  page play sound by itself only after a click, tap or key on the site, so a video starts without sound if there
  hasn't been one yet, and the first click, tap or key (on the video too) turns it on. What's typed goes only to
  the app, on that computer. Chosen once, it's used on later visits while the app runs; when the app stops, the
  page says so and the written answers are back; **Stop using this computer's model** puts it away.
* **Heart, a warmer voice** for Read aloud (and for answers read out while talking): Kokoro's `af_heart`, the voice
  Chat Computer itself speaks with, made on the device by its graphics chip (WebGPU) with kokoro-js (a fixed
  version, from jsDelivr). The first time Read aloud is pressed where it can run, a sheet offers it, with its size
  (about 330 MB, downloaded once from Hugging Face in pieces of 16 MB, each kept as it comes, so a page left part
  way carries on where it stopped, and kept by the browser); it then reads
  what was asked. Closed, the browser's own voice reads, and Heart stays in the **/** list (**Download Heart**,
  later **Remove Heart**). Otherwise the browser's best voice reads: natural, neural or premium voices first
  (Edge's Ava), then Google's, then the usual one, never a novelty voice. Safari lets pages use only an iPhone's
  built-in voices, which sound robotic, so on an iPhone Heart is the warm one. Where there's no WebGPU it isn't
  offered: a phone's processor alone makes it more slowly than it speaks.

What's typed stays on the computer (in the browser, or with the app's model), except a place's name asked about
for a map, the weather or the air, which goes
to Open-Meteo's free geocoding and forecasts (as does, for the greeting's weather, your time zone's city or where
you said you live, at most once every half hour; the map itself comes from OpenStreetMap), the words looked up on
Wikipedia, Google's pages when asked for (as on google.com), and with what else the **/** list connects (there are
no buttons beside the message box):

* **Connect AirPods**: you talk and the answers are read out. The browser listens through whatever microphone
  the device uses (your AirPods, once they're paired in its Bluetooth settings: a website can't pair them) and
  turns speech into text itself (Chrome does it on Google's servers). Chrome, Edge and Safari can; Firefox
  can't. On a phone too: on an iPhone or iPad in Safari (with Siri or Dictation switched on), and on Android in
  Chrome. There it listens for one thing at a time, again after each
  answer is read out, since the phones' listening goes wrong when it's left on; the screen stays on while
  talking.

On a phone the page is laid out as the Claude app is there, in any browser: the text an eighteenth of the screen's
width (never under 16px, so the browser doesn't zoom in on the box and push the page past the screen's edges), the
message box nearly as wide as the screen, with the words above and + and the microphone and send arrow under them, never
under the keyboard (Safari on an iPhone doesn't make the page smaller for its keyboard, so the box is lifted to sit
just above it), and the keyboard going down after sending, as in ChatGPT's app. Nothing zooms, two fingers included. Holding a message
dims the screen behind it and brings up what can be done with it: Copy, Send to (the phone's share sheet),
Select text (the message alone, its words to select as anywhere on the phone), and Edit for yours or Read aloud
for Chat Computer's.

Chat Computer is an independent project. It thinks with Qwen, an open model from Alibaba, run on your computer by
Chat Computer's own engine. It has no icon: the tab shows the browser's own.

GitHub Pages serves it from the `docs` folder (Settings, Pages, branch `main`, folder `/docs`; a private
repository needs GitHub Pro) at chatcomputer.ai: `docs/CNAME` names the domain, and its DNS points at GitHub
Pages (A records for the domain itself, a CNAME record for `www`).

## The apps

**https://chatcomputer.ai/get** offers Chat Computer as apps, laid out as Annotated's page is (the name, a button
for each, the line that says what it is; no video). On Windows and on a Mac it's one Setup, which asks how you want
Chat Computer:

* **Chat Computer Setup.** For Windows, `Chat-Computer-Windows.exe`: the Chat Computer app's installer, for your account
  (no administrator needed). For a Mac, one line of Terminal, `curl -fsSL https://chatcomputer.ai/get/mac.sh | bash`
  (the Mac button shows it): `docs/get/mac.sh` downloads the app's disk image (`Chat-Computer-Mac.dmg`), checks its
  SHA-256 against the one written in it (`scripts/build_apps.py` writes it from the disk image beside it, so run that
  again whenever the disk image is replaced), copies Chat Computer into Applications (or your own `~/Applications`)
  with no password, and opens it. What curl downloads isn't marked as coming from the internet, so macOS opens the
  app without asking for Open Anyway (the disk image is still there to download, and then macOS does ask). The app's
  first page (`apps/desktop/start`) asks **How do you want Chat Computer?** (on a Mac, from the app's next build:
  the disk image on /get is still the earlier app, and the page says so), and the app does it
  (`apps/desktop/src-tauri/src/ways.rs`), keeps the answer, and starts there next time: on the conversation (the app,
  the browser), or on the way's steps (the extension, the own system). To change it: type **/** in Chat Computer and
  choose **How you have Chat Computer**, or open the app again.
  * **Chrome extension**: the extension (`apps/chrome`, which comes with the app) is put in a folder of the app's
    own that stays (`Chrome extension` in its data folder), the folder's path is copied, Chrome is opened (at
    `docs/get/chrome.html`, the same steps, with the folder's path after `#folder=`, which a browser doesn't send to
    the site), and the page shows the three steps: go to chrome://extensions, turn on Developer mode, choose Load
    unpacked and that folder (paste its path). Chrome adds an extension by itself only from its Web Store, and doesn't let another app
    open chrome://extensions (it drops such an address from its command line: tried), so those steps are yours. In
    the Chrome Web Store it would be one click, which needs the owner's developer account.
  * **Browser**: the app as your web browser. Sites stay open in tabs, in a row above the middle (each a view of the
    web of its own in the app, `open_tab`, shown in turn): back and forward, the conversation's own tab, the sites'
    tabs with their x, and + for a new tab (the conversation, its box ready for an address). An address typed opens
    in a new tab, and so does a link, a link for a new window and a link clicked in another app; what Chat Computer
    opens goes in the tab it opened the last in. Ctrl+T, Ctrl+W and Ctrl+Tab work too. The app is listed with the
    system as a browser: on Windows for your account (StartMenuInternet, Capabilities, RegisteredApplications and the
    ChatComputerURL kind of link, under HKEY_CURRENT_USER; one Chat Computer at a time, so a link clicked while it
    runs reaches the one running; taken out again by another way, and by the uninstaller), on a Mac by its
    Info.plist (http and https; macOS sends the link to the app). The page offers to make it the default browser:
    Windows doesn't let an app do that itself, so Settings opens at Default apps for Chat Computer, where you choose
    it; a Mac asks you to confirm (LSSetDefaultHandlerForURLScheme).
  * **App**: the app as it was (below).
  * **Its own system**: Chat Computer Setup, which comes with the app (the repository's files it uses, put beside
    the app by `scripts/build_apps.py desktop --for windows|mac`: the repository is private, so its archive on
    GitHub isn't there to download), is copied to a folder of the app's own (`Chat Computer Setup` in its data
    folder; on a PC, the installed Chat Computer looks there for its updates too) and started from there: on a PC,
    `Chat Computer Setup.cmd`, which asks Windows for permission and shows its plan before changing anything (see
    [Installing on a Windows PC](#installing-on-a-windows-pc)); on a Mac with M1, M2 or M3, `mac/setup.sh`, in
    Terminal (see [Installing on a Mac](#installing-on-a-mac)). On another Mac the page says plainly why it can't.

* **Chrome**: an extension (`apps/chrome`) that shows Chat Computer in Chrome's side panel. The panel is the
  website itself (chatcomputer.ai, in its frame with `?app=chrome`, so it's always the newest), at a side panel's
  text size, and the sites asked for open in real Chrome tabs, any site, without a click: the website asks the
  extension, which opens them (only chatcomputer.ai can ask, and only for a web address). It asks for one
  permission, the side panel. Add to Chrome downloads it as a zip (`scripts/build_apps.py` makes it): unzip it, open
  chrome://extensions, turn on Developer mode and choose Load unpacked, until it's in the Chrome Web Store.
* **Windows and Mac**: the Chat Computer app (`apps/desktop`, made with Tauri): the website's page, bundled into
  a window of its own (so nothing that runs with the app's powers comes from elsewhere), with what a website can't
  do. Any site opens right in the middle, in the app's own browser view laid over the frame the page draws (sites
  that refuse to show inside another page, most of them, open too, as on a Chat Computer), and the same view is
  used each time, so sign-ins stay; a link that would open a new window opens there; downloads go to Downloads.
  "/" finds files on the computer by name (Desktop, Documents, Downloads, Pictures, Music, Videos, OneDrive), and
  the one chosen opens with its own app; programs, scripts and anything "/" didn't find aren't opened. Its model
  can do the same (find_files, open_file), and read and open your messages (read_messages, open_message). Messages
  brings together your own Gmail, Outlook (Outlook.com,
  Hotmail, Live and Microsoft 365), Telegram, WhatsApp, WeChat and Discord: + in Messages (or Connect email, Connect
  chat apps) opens the one chosen in the middle, on its own page, where you sign in yourself (a code or a QR code
  from your phone, as on its website); it then stays signed in, out of sight, and what its list of conversations
  shows comes into Messages, newest first (who, what, when, unread). Tapping one opens it there, at that
  conversation. Nothing is sent anywhere: the app reads the service's page as you'd see it, and only that page can
  hand it over. (WeChat's web version isn't open to every account; Google sometimes refuses sign-ins in an app's
  browser view.) The greeting has the name the computer knows you by. The model comes with the app: Chat
  Computer's own engine (`os/model`, beside the app's program) picks the one this computer runs well, by its memory
  and the room on its disk (from Qwen3.5 0.8B up to Qwen3.8 27B), downloads it once from Qwen's own files over
  Chat Computer's own HTTPS (each file checked against its pinned SHA-256; a download that stops carries on where
  it stopped), makes it ready to run, takes its refusals out, and serves it on 127.0.0.1 to the app's page (and to
  chatcomputer.ai, where it's chosen there): free, with no account, and nothing sent anywhere. Meanwhile the empty
  box says how far it's got, and a smaller one that's here already answers. It gets ready at a lower priority,
  ends with the app, and is started again should it stop by itself. Heart's code comes with the app too. Only the
  app's own page can ask the app for these
  (its capabilities, and each command checks); a site in the middle gets nothing. On a Mac the page comes from
  the app's own server at 127.0.0.1 (a Mac's web view doesn't count Tauri's own scheme as secure, and then there's
  no WebGPU for Heart, no microphone and no browser cache), which answers only to that name. GitHub builds them
  (`.github/workflows/apps.yml`): an installer for Windows (for your account, no administrator needed), and a disk
  image for Macs with Apple silicon or Intel, with `mac.sh` for it. They aren't signed with a paid certificate yet,
  so the first time, Windows says it protected your PC (choose More info, then Run anyway), and macOS, for the disk
  image downloaded in a browser, says it can't check the app (System Settings, Privacy & Security, Open Anyway).
  The Mac's app is signed ad hoc, so each new version is a new app to macOS, which asks again for the microphone;
  signed with one self-signed identity, kept as the repository's secrets, it stays the same app: run
  `bash apps/desktop/make-signing-identity.sh` once on a Mac (it makes the identity and gives GitHub the secrets
  MAC_SIGNING_P12 and MAC_SIGNING_PASSWORD, which the workflow then signs with).
* **Android**: the Chat Computer app (`apps/android`), chatcomputer.ai full screen in Chrome, with no address bar (a
  Trusted Web Activity: the site says the app is its own, in `docs/.well-known/assetlinks.json`, by its signing
  key's fingerprint). Downloaded from /get as an APK until it's on Google Play: Android asks once to allow
  installing from the browser. Its signing key is kept outside the repository.
* **iPhone**: the Chat Computer app (`apps/ios`), chatcomputer.ai full screen, other sites in Safari's own view
  over it; built for the simulator by `.github/workflows/ios.yml` (with a picture of it running). Coming to the
  App Store, which needs Apple's developer account.

## For the curious

* `ARCHITECTURE.md` explains how it fits together.
* `windows/` holds the PC setup tool, `mac/` the Mac setup, `linux/` the thin Linux base (Ubuntu on a PC,
  Fedora Asahi Remix on a Mac), and `chatcomputer/` Chat Computer itself.
* `os/` is Chat Computer's own operating system, being written from scratch to replace the Linux base. Its
  README says what runs so far, what no operating system can own on a PC, and the steps ahead.
* `evals/` says whether a change made things better, the way Hamel Husain's evals FAQ describes: read real traces
  first, note each one's first failure, group those into the failure modes that keep coming up, write a pass/fail
  check for each, and keep the checks honest against hand labels (`labels/`; `evaluate.py --labels` gives how
  many failures they catch and passes they leave alone). `evals/os` gives Chat Computer everyday requests on a
  simulated computer, each in its own sandbox, answered by its own model (`python -m evals.os.run OUT_DIR --models
  DIR`, the models made ready in DIR/llm). `evals/web` types a first-time visitor's questions into the website
  (`node evals/web/run.js OUT.json`): with the page's written answers, or with Chat Computer's model in the browser
  (`--model web`, and `--engine native:PORT` to have the engine's program answer the page's requests, a few times
  faster). `evals/tools` prints traces to read (`show_web.py`, `show_os.py`) and `review.html` opens them to label
  each one pass or fail, with why.
