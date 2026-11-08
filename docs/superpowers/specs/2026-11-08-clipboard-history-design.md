# Clipboard history

## Purpose

Keep a local, persistent history of copied text, separate from translation history, so a previous entry can be copied back to the system clipboard.

## Behavior

- Start monitoring automatically when LingoPop starts on Windows and macOS. Observe clipboard changes, not keyboard shortcuts; no new shortcuts are required.
- Ignore content marked sensitive by the system or source application. Only text is in scope.
- Store the newest entries first. Copying text already in history moves the existing entry to the top. Selecting an entry copies it to the clipboard and likewise makes it the newest entry.
- Settings independently control the number of entries retained (default 100) and shown in the tray menu (default 10). Reducing the retention limit removes the oldest excess entries.
- Show recent entries in the tray/menu-bar menu. Clicking an entry copies it. Deletion is available in the history window and removes entries from persistent history and every view.
- A menu action opens a separate window with all retained entries. Clicking an entry copies it; each entry can be deleted, and the window offers an action to clear all history. Changes are reflected in both views.
- Clipboard history stays local and is never sent to an AI provider merely because it was copied.

## Approach

Use OS-specific change detection: Windows clipboard notifications and the macOS pasteboard change counter. Read contents only after a change and check sensitive markers before recording. A common history store and shared window/settings UI serve both platforms. An alternative is polling on both platforms; the OS-specific approach was selected to use the mechanisms available on each system.

## Implementation considerations

The existing tray uses Tauri native menus. Native tray entries need no right-click action. Define platform-specific sensitive marker handling and test it with synthetic data. Exclude marked temporary clipboard writes. Selection copied by the foreground application for translation may appear in history; do not suppress other applications' writes. Keep clipboard contents out of logs. Handle concurrent changes and persistence failures without losing the user's clipboard. Validate native behavior on both operating systems as well as store and UI behavior.
