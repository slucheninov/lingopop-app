/** Unicode scalar count, i.e. Rust `chars().count()` / Swift `unicodeScalars.count` (not UTF-16 units, not graphemes). */
export function countScalars(s: string): number {
  return Array.from(s).length;
}

/** The first `max` scalars; never splits a surrogate pair. */
export function truncateScalars(s: string, max: number): string {
  const scalars = Array.from(s);
  return scalars.length <= max ? s : scalars.slice(0, Math.max(0, max)).join('');
}
