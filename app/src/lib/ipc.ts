import { invoke } from '@tauri-apps/api/core';
import type { PopupAction } from './types';
// Never print IPC failures: native errors may contain user data.
export function action(action: PopupAction): Promise<void> {
  return invoke<void>('popup_action', { action });
}
export function sendAction(value: PopupAction): void { void action(value).catch(() => {}); }
export function resized(width: number, height: number): Promise<void> {
  return invoke<void>('popup_resized', { width, height });
}
export function ready(): Promise<void> { return invoke<void>('popup_ready'); }
