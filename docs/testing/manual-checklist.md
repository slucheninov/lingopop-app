# Manual checklist (Windows)

Things CI cannot verify: a real desktop, other apps, several monitors, elevation. Run it on a Windows 10/11 machine before a release and after changes to `lingopop-platform`, the popup or the shell. Tick an item only when the expected result is observed. Build with `npm run build --prefix app` then `cargo build -p lingopop-app --release`.

Default hotkeys: Translate `Ctrl+Alt+T`; the other operations use the chords in `settings.json`.

## Startup, tray and files

- [x] Launch `lingopop.exe`. Expected: a tray icon with the tooltip "LingoPop", no window, no taskbar button.
- [x] Launch it a second time. Expected: the second process exits at once; the first keeps running (single instance).
- [x] Tray menu, "Quit LingoPop". Expected: the icon disappears and the process ends. Closing the popup never quits the app.
- [ ] Open `%APPDATA%\LingoPop`. Expected: the data folder is exactly this (not `%APPDATA%\app.lingopop.windows`) and holds `lingopop.log`; `history.json` / `usage.json` appear after the first successful run.
- [ ] Occupy the Translate chord with another app, then start LingoPop. Expected: the app starts, the tray works, other hotkeys work, and `lingopop.log` has one line naming the operation and chord (no user text).
- [ ] Screen Translate hotkey (Phase 4a). Expected: no chord is registered for it and pressing it does nothing (the region picker arrives in Phase 4b).
- [ ] After a session with real text, search `lingopop.log` and `lingopop.log.1` for the selected text, the translation and the API key. Expected: none of them appear.

## Popup focus and dismissal

- [ ] In Notepad select a word and press the Translate hotkey. Expected: the popup appears near the cursor; the Notepad caret keeps blinking and typing continues in Notepad.
- [ ] Click Copy, Rewrite, the language menu and the Retry menu. Expected: the popup never takes focus (the Notepad title bar stays active).
- [ ] With the popup visible press Esc. Expected: the popup closes and the Esc is not delivered to Notepad.
- [ ] With the popup hidden press Esc in an app that reacts to it (a dialog, Chrome find bar). Expected: the app receives Esc normally.
- [ ] With the popup visible press Ctrl+Esc or Shift+Esc. Expected: the chord passes through (Start menu / task switcher behaves as usual) and the popup stays.
- [ ] With the popup visible click anywhere outside it. Expected: the popup closes and the click still reaches the app underneath.
- [ ] Click inside the popup (text, empty area). Expected: the popup stays open.
- [ ] Trigger an operation, then trigger another before the first result arrives. Expected: only the second result is shown; no flicker of the first.

## Capture and Replace in other apps

- [ ] Translate a selection in Notepad, Word, Chrome (text field and page text) and Telegram. Expected: a result appears in each.
- [ ] Replace in Notepad, Word, Chrome (text field) and Telegram. Expected: the selection is replaced by the result in the original window; the popup is hidden before the paste.
- [ ] Switch to another window while the result is shown, then Replace. Expected: the original window (the one that was active at capture) receives the text.
- [ ] Close the target window while the result is shown, then Replace. Expected: the manual-paste message ("Couldn't switch back ... paste it manually with Ctrl+V") and the result is on the clipboard.
- [ ] Copy a file in Explorer, translate a selection, then paste in Explorer. Expected: the file is pasted (clipboard restored after capture).
- [ ] Copy a cell range in Excel, capture a selection elsewhere, then paste in Excel. Expected: cells, formatting and formulas intact; `lingopop.log` lists any skipped format ids (fixed text and ids only).
- [ ] Copy rich text in Word, capture elsewhere, paste in Word. Expected: formatting intact.
- [ ] Copy an image, capture elsewhere, paste the image. Expected: the image is restored.
- [ ] After a capture, open Win+V clipboard history. Expected: the temporary text of LingoPop is not in the history and is not synced.
- [ ] Capture in Notepad, then press Alt alone in Word / Notepad. Expected: no menu bar flashes open after the capture's synthesized key-up.
- [ ] Capture from a console app (cmd, PowerShell) where Ctrl+C does not copy. Expected: "Nothing selected" style message or a clipboard timeout message; no hang, no stray input.
- [ ] Hold the Translate chord's modifiers longer than a second. Expected: the capture waits for the release; no stuck modifier afterwards.

## Elevation (UIPI)

- [ ] Start LingoPop normally and Notepad as administrator ("Run as administrator"). Select text in Notepad and press the hotkey. Expected: the message that LingoPop can't reach an app running as administrator; nothing is pasted or typed; the clipboard is unchanged.
- [ ] Start LingoPop as administrator too and repeat in elevated Notepad. Expected: capture and Replace work.
- [ ] Both elevation checks above passed. Expected: `ForegroundWindow::is_elevated` is confirmed; remove the TODO on it in `crates/lingopop-platform/src/foreground.rs`.

## Copy and Replace failures keep the result

- [ ] Hold the clipboard open from another tool (for example a script that calls `OpenClipboard` and sleeps), then click Copy. Expected: the translation stays visible with a notice (clipboard did not update in time); the popup is not replaced by an error.
- [ ] Release the clipboard and click Copy again. Expected: it succeeds and the notice disappears.
- [ ] Hold the clipboard open and click Replace. Expected: the popup closes, then reappears near the same place with the same result and a notice; nothing was pasted (a confirmed failure, so Replace stays available).
- [ ] After that failure click Replace again with the clipboard free. Expected: the text is pasted into the original window.
- [ ] After the failure press Esc / click outside. Expected: the popup closes (the dismiss watcher works on the re-shown popup), also when you click inside a re-shown popup larger than the first 360x100 size (inside clicks never dismiss).
- [ ] Make the paste hang past the 3 s limit (for example a target app that stops responding). Expected: the result reappears with a notice and WITHOUT a Replace button for the rest of that session; Copy still works.

## Hung clipboard owner

- [ ] Keep another app holding the clipboard, then press the hotkey. Expected: within a few seconds a clipboard timeout message; the tray menu and other hotkeys stay responsive.
- [ ] Copy something from an app that renders clipboard data lazily and then hangs (or suspend that process), press the hotkey. Expected: the same timeout message and no frozen LingoPop.

## DPI, monitors and placement

- [ ] Two monitors with different scaling (100% and 150% or more). Translate with the cursor on each. Expected: the popup appears next to the cursor on the same monitor, fully inside its work area, with the same physical-looking size relative to the text.
- [ ] Move the cursor near each screen edge and corner. Expected: the popup flips above / beside the cursor and never goes off screen.
- [ ] Taskbar on the bottom, top, left and right. Expected: the popup never overlaps the taskbar.
- [ ] Long text (more than 120 characters, and more than 700 px wide). Expected: the popup grows to at most 700 px wide, the action row switches between full, mixed and compact labels so it never overflows, and when it resizes it is repositioned (flipped above the cursor or pushed left/up) so it stays inside the work area, including with the cursor near the bottom or right edge.
- [ ] With a long result near the bottom-right corner, click inside the grown popup far from its top-left corner. Expected: it does not dismiss; clicking just outside its edge does.
- [ ] Change the display scaling while the app runs, then translate. Expected: placement and size are correct.
- [ ] Monitors arranged with a gap or negative coordinates (left of or above the primary). Expected: placement is still correct on those monitors.

## Screen capture and OCR (Phase 4b, with the overlay)

- [ ] Capture a region spanning two monitors with mixed DPI. Expected: the image matches what is on screen, at physical pixels (a gap between monitors is black).
- [ ] A system with no OCR language pack installed. Expected: the "No OCR language is installed" message with the Settings hint, and Retry offers the vision providers.
- [ ] A very large region (taller or wider than the OCR engine limit). Expected: the text is recognized (the image is downscaled) or an OCR error is shown; no crash.
