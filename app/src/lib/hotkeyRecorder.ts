import type { Modifier } from './settingsTypes';
export type RecorderOutcome = { kind: 'ignored' } | { kind: 'cancel' } | { kind: 'record'; key: string; modifiers: Modifier[] };
export function classifyKeydown(e: { code: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }): RecorderOutcome {
  if (/^(Control|Alt|Shift|Meta)(Left|Right)$/.test(e.code)) return { kind: 'ignored' };
  const modifiers: Modifier[] = [];
  if (e.ctrlKey) modifiers.push('ctrl');
  if (e.altKey) modifiers.push('alt');
  if (e.shiftKey) modifiers.push('shift');
  if (e.metaKey) modifiers.push('win');
  if (e.code === 'Escape' && !modifiers.length) return { kind: 'cancel' };
  return modifiers.length ? { kind: 'record', key: e.code, modifiers } : { kind: 'ignored' };
}
