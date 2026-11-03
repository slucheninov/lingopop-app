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

### Phase 4b shell foundation

- [ ] A second launch opens/focuses Settings; rapid requests create one settings window. Closing it destroys the window and a later request recreates it.
- [ ] Settings is centered, fixed at 720x560; History starts at 540x480, resizes down to 480x360; both show localized titles. (Frontend routing arrives in later Phase 4b tasks.)
- [ ] Opening Settings to About delivers the initial tab after bootstrap; reopening an already live settings window switches tabs via `settings://tab`.
- [ ] Allowed release/documentation links launch the default browser; foreign URLs/traversal are refused without logging the URL.
- [ ] With Windows UI language Ukrainian/German, system language resolves accordingly; explicit app language overrides it and changes require relaunch.

### Settings: second launch and Main tab

- [ ] With Settings closed, launch `lingopop.exe` a second time. Expected: the second process exits and the Settings window opens; with Settings already open, a further launch focuses it instead of creating another one.
- [ ] Settings > Main: change the target language, auto-detect, language pairs, limits and Launch at login, quit from the tray and start again. Expected: every value is still as set.
- [ ] Move the popup font size slider (11 to 20), then trigger a translation. Expected: the result text and the header (provider / language line) use the new size, the header two points smaller; with a popup already visible the size changes live.
- [ ] Change the app language (for example to Polski). Expected: the `Relaunch` prompt appears at once; click Relaunch. Expected: LingoPop restarts (no second tray icon) and Settings and the popup now show Polish.

### Launch at login

- [ ] Enable Launch at login, sign out and in: LingoPop starts; disable: it does not; move the exe and start it: the Run value is rewritten to the new path.

### Providers tab

- [ ] Settings > Providers: select Claude, paste a key and click Save. Expected: the key field empties at once, `Saved` appears for about 1.5 s and the sidebar shows `Key set`. Restart LingoPop: still `Key set`; open `%APPDATA%\LingoPop\lingopop.log`: the key (and any part of it) appears nowhere.
- [ ] Click Clear on a stored key. Expected: `No key set`, the sidebar shows `No key`; Google (Free) shows `No key needed` and has no key field.
- [ ] Type `http://x` into Endpoint. Expected: the https message appears and Save is disabled; fix it and Save works.
- [ ] Rename `secrets.dat` to something unreadable (e.g. replace its content with text) and restart. Expected: Providers shows the `secrets.dat is unreadable` message instead of the key field and Save is disabled; the file is not modified.
- [ ] `How to get an API key` opens the matching section of `docs/api-keys.md` in the default browser.

- [ ] Settings → Shortcuts: click a field, record a modified chord, and use the new chord immediately.
- [ ] Record a chord owned by another app: the conflict line appears; the remaining shortcuts still work.
- [ ] Record a chord already assigned to another operation: the duplicate line names that operation; the saved shortcut stays unchanged.
- [ ] Press the Translate chord while recording: it is recorded instead of triggering a translation.
- [ ] Start recording, then blur or close Settings: all saved shortcuts resume; closing after blur leaves them registered.
- [ ] Assign Screen Translate a chord: it opens the native region picker.

### Task 11: History and tray

- [ ] Tray history rows copy the translation to the clipboard.
- [ ] Digit mnemonics select recent rows; if `&1` … `&0` render literally on Windows, drop the mnemonic prefix.
- [ ] `Target:` submenu checks the current language and saves the selected language.
- [ ] About LingoPop opens Settings on the About tab.
- [ ] Tray icon animates while working and turns red for 3 seconds on an error; a new operation cancels the error reset.
- [ ] History window search, Copy original / Copy translation, Delete and Clear work; changes refresh the tray and open History window.

### Task 12: tray-triggered operations

- [ ] Select text in Notepad, open the tray menu and click Translate. Expected: the popup shows the translation of the Notepad selection (Notepad is brought back to the front first, the clipboard is restored afterwards).
- [ ] Sign in fresh (or restart LingoPop with no other window ever opened), open the tray menu and click Translate. Expected: the "nothing selected" message.


### Task 13: Screen Translate overlay

- [ ] Overlay dims all monitors, including a negative-origin mixed-DPI arrangement; the taskbar is covered.
- [ ] Cursor is a crosshair throughout selection; the size label and white border scale with the monitor.
- [ ] Drag in either direction: the label shows physical width × height, with a black shadow.
- [ ] Esc cancels silently and the underlying app receives no Esc; afterward its normal Esc handling works.
- [ ] A selection smaller than 10 px in either dimension cancels silently and returns the tray to Idle.
- [ ] Complete a selection and inspect the captured result: the overlay never appears in the capture.
- [ ] Press the Screen Translate hotkey again during selection: the old overlay disappears before a fresh overlay starts.
