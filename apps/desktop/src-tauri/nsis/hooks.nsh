; Chat Computer Setup for Windows: Tauri's installer (tauri.conf.json, bundle > windows > nsis), with these additions.
; It installs the app for this account (no administrator needed); the app, the first time it starts, asks how you
; want Chat Computer (as an app, or as its own system next to Windows) and does it (src/ways.rs). This file is included before the installer's pages, so its first page and its last say so.

!define MUI_WELCOMEPAGE_TITLE "Chat Computer Setup"
!define MUI_WELCOMEPAGE_TEXT "This puts Chat Computer on this PC, for your account. When it starts, it asks how you want Chat Computer:$\r$\n$\r$\n    as an app,$\r$\n    or as its own system, next to Windows.$\r$\n$\r$\nYou can change your mind later. Click Next to go on."
!define MUI_FINISHPAGE_RUN_TEXT "Start Chat Computer, and choose how you want it"

