import type { RowMode } from './types';
export const LONG_TEXT_CHARACTER_THRESHOLD = 120;
export const isIconOnly = (mode: RowMode, index: number): boolean => mode === 'compact' || (mode === 'mixed' && index >= 2);
export function candidates(chars: number, buttons: number): RowMode[] {
  return chars >= LONG_TEXT_CHARACTER_THRESHOLD || buttons <= 2 ? ['full', 'mixed', 'compact'] : ['mixed', 'compact'];
}
export function pickMode(modes: readonly RowMode[], measure: (mode: RowMode) => number, available = 700): RowMode {
  return modes.find(mode => measure(mode) <= available) ?? 'compact';
}
// Same chrome/spacing metrics as core/action_row.rs, with an injected text measurer.
export function rowWidth(titles: readonly string[], retry: boolean, mode: RowMode, measure: (text: string) => number, chevrons = 0): number {
  const widths = titles.map((title, i) => isIconOnly(mode, i) ? 30 : Math.ceil(measure(title)) + 38);
  if (chevrons && widths.length) widths[widths.length - 1] += chevrons * 34;
  if (retry) widths.push((isIconOnly(mode, titles.length) ? 30 : Math.ceil(measure('Retry')) + 38) + 14);
  return widths.length ? widths.reduce((a,b) => a+b, 0) + 8 * widths.length + 28 : 0;
}
export function measureText(text: string): number {
  const canvas = document.createElement('canvas');
  const context = canvas.getContext('2d');
  if (!context) return Array.from(text).length * 7;
  context.font = '13px system-ui';
  return context.measureText(text).width;
}
export function popupWidth(model: { phase: string; text?: string }): number {
  return model.phase === 'result' || model.phase === 'ocr'
    ? Math.min(700, Math.max(320, 200 + Array.from(model.text ?? '').length)) : 320;
}
