import { invoke } from '@tauri-apps/api/core';
import type { PermissionKindId, PermissionStatus } from './settingsTypes';
// A rejection is a catalog key (rendered through t()), never user text. Never log it.
/** State only: never prompts. Used on mount. */
export const getPermissions = (): Promise<PermissionStatus[]> => invoke<PermissionStatus[]>('permissions_status');
/** State only: never prompts. Used when the user returns from System Settings. */
export const refreshPermissions = (): Promise<PermissionStatus[]> => invoke<PermissionStatus[]>('permissions_refresh');
/** The only call that can show the system prompt: a button for exactly this kind. */
export const requestPermission = (kind: PermissionKindId): Promise<PermissionStatus[]> => invoke<PermissionStatus[]>('permissions_request', { kind });
/** Only the kind id travels: the shell chooses the System Settings pane, never the webview. */
export const openPermissionSettings = (kind: PermissionKindId): Promise<void> => invoke<void>('permissions_open_settings', { kind });
