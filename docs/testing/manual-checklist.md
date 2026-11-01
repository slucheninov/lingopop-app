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
