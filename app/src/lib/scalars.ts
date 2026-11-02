/** Unicode scalar count, i.e. Rust `chars().count()` / Swift `unicodeScalars.count` (not UTF-16 units, not graphemes). */
export function countScalars(s: string): number {
  return Array.from(s).length;
}

/** The first `max` scalars; never splits a surrogate pair. */
export function truncateScalars(s: string, max: number): string {
  const scalars = Array.from(s);
  return scalars.length <= max ? s : scalars.slice(0, Math.max(0, max)).join('');
}

/** Rust `char::is_whitespace` (Unicode White_Space): not JS `trim`, which also strips U+FEFF but not U+0085. */
const RUST_WHITESPACE = '\\u0009-\\u000D\\u0020\\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000';
const EDGE_WHITESPACE = new RegExp(`^[${RUST_WHITESPACE}]+|[${RUST_WHITESPACE}]+$`, 'gu');
// ASCII controls except \n and \t (U+007F and everything non-ASCII stay).
const DROPPED_CONTROLS = /[\u0000-\u0008\u000B-\u001F]/g;

/** Drops the controls Rust `sanitize_magic_prompt` drops and cuts to `max` scalars, in that order (no trim). Used live while typing. */
export function dropControlsAndTruncate(s: string, max: number): string {
  return truncateScalars(s.replace(DROPPED_CONTROLS, ''), max);
}

/** Exact order of core `sanitize_magic_prompt` (= Swift `SettingsStore.sanitizeMagicPrompt`): trim, drop controls, cut to 2000 scalars. */
export function sanitizeMagicPrompt(s: string): string {
  return dropControlsAndTruncate(s.replace(EDGE_WHITESPACE, ''), 2000);
}
