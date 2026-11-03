# Manual checklist (Windows)

The basics CI cannot verify: a real desktop and other apps. Run it on a Windows 10/11 machine (a VM is fine) before a release and after changes to `lingopop-platform`, the popup or the shell. Tick an item only when the expected result is observed. Use the release exe, or build the same way the release workflow does: `npm run build --prefix app` then `cargo build -p lingopop-app --release --locked --features custom-protocol` (without the feature the exe loads the dev server URL).

Default hotkeys: Translate `Ctrl+Alt+T`; the other operations use the chords in `settings.json`.

## Startup, tray and files

- [x] Launch `lingopop.exe`. Expected: a tray icon with the tooltip "LingoPop", no window, no taskbar button.
- [x] Launch it a second time. Expected: the second process exits at once; the first keeps running (single instance).
- [x] Tray menu, "Quit LingoPop". Expected: the icon disappears and the process ends. Closing the popup never quits the app.
- [x] Open `%APPDATA%\LingoPop`. Expected: the data folder is exactly this (not `%APPDATA%\app.lingopop.windows`) and holds `lingopop.log`; `history.json` appears after the first successful run, `usage.json` only after a provider that reports token usage (not Google (Free)).
- [x] Occupy the Translate chord with another app, then start LingoPop. Expected: the app starts, the tray works, other hotkeys work, and `lingopop.log` has one line naming the operation and chord (no user text).
- [ ] After a session with real text, search `lingopop.log` and `lingopop.log.1` for the selected text, the translation and the API key. Expected: none of them appear.
- [ ] Quit the app, rename an existing `settings.json` to `settings.json.keep` (fresh install), start the app and translate; afterwards quit and rename it back. Expected: Google (Free) is the active provider and works without any key.

## Popup

- [x] In Notepad select a word and press the Translate hotkey. Expected: the popup appears near the cursor; the Notepad caret keeps blinking and typing continues in Notepad.
- [ ] Press the Translate hotkey (`Ctrl+Alt+T`) in Notepad (Windows 11) and release the keys. Expected: no menu access-key tips (F, E, V, ...) appear over the Notepad menu bar.
- [x] Show the popup (short and long result) at 100% and 200% display scaling. Expected: no scrollbars and the close button (top right) is fully visible.
- [x] Click Copy, Rewrite, the language menu and the Retry menu. Expected: the popup never takes focus (the Notepad title bar stays active).
- [x] Click the close button (x). Expected: the popup closes.
- [x] With the popup visible press Esc. Expected: the popup closes and the Esc is not delivered to Notepad.
- [x] With the popup hidden press Esc in an app that reacts to it (a dialog, Chrome find bar). Expected: the app receives Esc normally.
- [ ] With the popup visible click anywhere outside it. Expected: the popup closes and the click still reaches the app underneath. Also inside a virtual machine (Parallels / UTM / VMware), where the mouse arrives as injected input.

## Copy, Replace and errors

- [ ] Click Copy. Expected: the translation is on the clipboard (paste it somewhere); the popup stays open.
- [ ] In Notepad select a whole line including its line break (triple-click or Shift+Down from the line start), translate and click Replace. Expected: the line is replaced by the translation and the line break after it is still there (the next line does not join).
- [ ] Select text in a browser text field (Chrome / Edge), translate and click Replace. Expected: the selection is replaced by the result; the popup is hidden before the paste.
- [ ] With Google (Free) active, click Rewrite and Explain. Expected: a clear message that the provider does not support the operation (no crash, no empty popup).
- [x] Start LingoPop normally and Notepad as administrator ("Run as administrator"). Select text in Notepad and press the hotkey. Expected: the message that LingoPop can't reach an app running as administrator (it closes by itself after ~6 s or with x); nothing is pasted or typed; the clipboard is unchanged.
- [x] Start LingoPop as administrator too and repeat in elevated Notepad. Expected: capture and Replace work.

## Settings

- [ ] Open Settings from the tray. Expected: the window is centered, fixed at 720x560, and has the tabs Main, Providers, Shortcuts, Usage, Magic and About with a localized title.
- [ ] Tray menu, About LingoPop, then (with Settings still open) About again from another tab. Expected: Settings opens on the About tab; a live window switches to it instead of opening a second window.
- [ ] Close Settings and open it again several times quickly. Expected: only one Settings window ever exists; closing destroys it and the next request recreates it.
- [ ] Settings > Main: change the target language, auto-detect, language pairs, limits and Launch at login, quit from the tray and start again. Expected: every value is still as set.
- [ ] Move the popup font size slider (11 to 20), then trigger a translation. Expected: the result text and the header (provider / language line) use the new size, the header two points smaller; with a popup already visible the size changes live.
- [ ] Settings > Providers: select Claude, paste a key and click Save. Expected: the key field empties at once, `Saved` appears for about 1.5 s and the sidebar shows `Key set`. Restart LingoPop: still `Key set`; open `%APPDATA%\LingoPop\lingopop.log`: the key (and any part of it) appears nowhere.
- [ ] Click Clear on a stored key. Expected: `No key set`, the sidebar shows `No key`; Google (Free) shows `No key needed` and has no key field.
- [ ] Type `http://x` into Endpoint. Expected: the https message appears and Save is disabled; fix it and Save works.
- [ ] Rename `secrets.dat` to something unreadable (e.g. replace its content with text) and restart. Expected: Providers shows the `secrets.dat is unreadable` message instead of the key field and Save is disabled; the file is not modified.
- [ ] Click `How to get an API key`, Privacy Policy and Download. Expected: the matching `docs/api-keys.md` section, `PRIVACY.md` and the releases page open in the default browser; a foreign or traversal URL is refused and never written to `lingopop.log`.
- [ ] Settings > About > `Check for updates` while no stable release is newer than this build. Expected: a brief spinner, then nothing visible (no banner, no error); with a newer stable release the `Version ... available` banner and `Download` appear.

## Shortcuts

- [ ] Settings > Shortcuts: click a field, record a modified chord, and use the new chord immediately. Expected: the operation runs from the new chord at once.
- [ ] Record a chord owned by another app. Expected: the conflict line appears; the remaining shortcuts still work.
- [ ] Record a chord already assigned to another operation. Expected: the duplicate line names that operation; the saved shortcut stays unchanged.
- [ ] Press the Translate chord while recording. Expected: it is recorded instead of triggering a translation.
- [ ] Start recording, then close Settings (or click another window). Expected: all saved shortcuts work again at once, and closing after blur leaves them registered.
- [ ] Clear a shortcut. Expected: the field shows no chord and the key no longer triggers the operation.

## Screen Translate

- [ ] Assign Screen Translate a chord and press it. Expected: the native region overlay opens (also from the tray menu entry).
- [ ] Look at the overlay on a multi-monitor, mixed-DPI setup (including a monitor left of / above the primary). Expected: all monitors are dimmed and the taskbar is covered.
- [ ] Move the mouse during selection. Expected: the cursor is a crosshair, moves freely with a physical mouse/touchpad, and the size label and white border scale with the monitor.
- [ ] Press and hold the left button without dragging. Expected: no border or `0 × 0` label.
- [ ] Drag a selection in either direction. Expected: the label shows physical width × height with a black shadow.
- [ ] Press Esc during selection. Expected: the overlay closes silently, the underlying app receives no Esc and its normal Esc handling works afterwards.
- [ ] Select a region smaller than 10 px in either dimension. Expected: the overlay closes silently and the tray returns to Idle.
- [ ] Press the Screen Translate hotkey again during selection. Expected: the old overlay disappears before a fresh overlay starts.
- [ ] Select a region that contains text. Expected: the popup shows the recognized text; the overlay itself never appears in the captured image; with auto-copy of OCR text on, the text is on the clipboard.

## Tray and History

- [ ] Open the tray menu. Expected: the header, the recent history rows, the operations with their hotkeys, the `Target:` submenu, History, Settings, About and Quit.
- [ ] Click a history row in the tray menu. Expected: the translation is on the clipboard.
- [ ] Press the digit mnemonics of the recent rows. Expected: the matching row is selected; if `&1` ... `&0` render literally on Windows, drop the mnemonic prefix.
- [ ] `Target:` submenu. Expected: the current language is checked and choosing another saves it.
- [ ] Run an operation, then force an error (for example with no network). Expected: the tray icon animates while working and turns red for 3 seconds on an error; a new operation cancels the error reset.
- [ ] Select text in Notepad, open the tray menu and click Translate. Expected: the popup shows the translation of the Notepad selection (Notepad is brought back to the front first, the clipboard is restored afterwards).
- [ ] Sign in fresh (or restart LingoPop with no other window ever opened), open the tray menu and click Translate. Expected: the "nothing selected" message.
- [ ] Tray menu, History. Expected: the History window opens with a localized title (540x480, resizes down to 480x360); search, Copy original / Copy translation, Delete and Clear work, and changes refresh the tray rows and an open History window.

## Startup and language

- [ ] With Settings closed, launch `lingopop.exe` a second time. Expected: the second process exits and the Settings window opens; with Settings already open a further launch focuses it instead of creating another one.
- [ ] Change the app language (for example to Polski). Expected: the `Relaunch` prompt appears at once; click Relaunch. Expected: LingoPop restarts, only one `lingopop.exe` process and one tray icon exist, and Settings, the tray menu and the popup now show Polish.
- [ ] With Windows UI language Ukrainian or German and the app language on System. Expected: the UI uses that language after a restart; an explicit app language overrides it.
- [ ] Enable Launch at login, sign out and in. Expected: LingoPop starts; with the option disabled it does not. Move the exe and start it: the Run value is rewritten to the new path.
