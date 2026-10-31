import type { PopupModel, RowMode } from './types';
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
let sharedContext: CanvasRenderingContext2D | null | undefined;
export function measureText(text: string): number {
  if (sharedContext === undefined) {
    sharedContext = document.createElement('canvas').getContext('2d');
    if (sharedContext) sharedContext.font = '13px system-ui';
  }
  return sharedContext ? sharedContext.measureText(text).width : Array.from(text).length * 7;
}
/** Text-driven width (mac `PopupLayout.dynamicWidth`): 320 floor, 200 + scalar count, 700 cap. */
export function popupWidth(model: { phase: string; text?: string }): number {
  return model.phase === 'result' || model.phase === 'ocr'
    ? Math.min(700, Math.max(320, 200 + Array.from(model.text ?? '').length)) : 320;
}
/** Popup `main` border on each side. Total width = row width (which already holds the 2 x 14 padding) + 2 x border. */
export const BORDER_WIDTH = 1;
export const MAX_WIDTH = 700;
/** Row inputs for result / OCR phases (mac `PopupView.panelWidth` titles), else null. */
export function rowSpec(model: PopupModel): { titles: string[]; retry: boolean; chevrons: number } | null {
  if (model.phase === 'result') {
    return { titles: ['Copy', ...(model.showReplace ? ['Replace'] : []), 'Rewrite', ...(model.showExplain ? ['Explain'] : [])],
      retry: model.retryProviders.length > 0, chevrons: 0 };
  }
  if (model.phase === 'ocr') {
    return { titles: ['Copy', 'Translate'], retry: model.retryProviders.length > 0, chevrons: model.translateProviders.length ? 1 : 0 };
  }
  return null;
}
/**
 * Port of mac `PopupView.panelWidth`: min(700, max(text width, width of the row in the PREFERRED
 * mode)), so the row is widened for first and only falls back inside the bar. Plus the border.
 */
export function panelWidth(model: PopupModel, measure: (text: string) => number = measureText): number {
  const base = popupWidth(model);
  const spec = rowSpec(model);
  if (!spec || (model.phase !== 'result' && model.phase !== 'ocr')) return base;
  const row = rowWidth(spec.titles, spec.retry, model.actionRow.preferred, measure, spec.chevrons);
  return Math.min(MAX_WIDTH, Math.max(base, row + 2 * BORDER_WIDTH));
}
/** Width available to the action row inside a popup of `width`: everything but the border. */
export const rowAvailable = (width: number): number => width - 2 * BORDER_WIDTH;
/** Identity of a displayed result: changes when content changes, not when only the retry list does. */
export function viewKey(model: PopupModel): string {
  const { retryProviders: _retry, notice: _notice, ...rest } = model;
  return JSON.stringify(rest);
}
