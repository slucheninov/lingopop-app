/**
 * Serializes size reports: at most one in flight, identical sizes are skipped, and while one is
 * in flight only the latest pending size is kept and sent after it settles (so order is preserved
 * and a stale size can never win).
 */
export function createResizeReporter(send: (width: number, height: number) => Promise<void>) {
  let last: [number, number] | null = null;
  let pending: [number, number] | null = null;
  let inFlight = false;
  async function pump(): Promise<void> {
    if (inFlight) return;
    inFlight = true;
    try {
      while (pending) {
        const [w, h] = pending;
        pending = null;
        if (last && last[0] === w && last[1] === h) continue;
        last = [w, h];
        try { await send(w, h); } catch { last = null; }
      }
    } finally { inFlight = false; }
  }
  return { report(width: number, height: number): Promise<void> { pending = [width, height]; return pump(); } };
}
