// Tone sliders in the editor (LAP-108): how a slider gesture becomes undo steps, and the live
// preview that follows the slider while it moves.

export interface StepClock {
  now(): number;
  set(fn: () => void, ms: number): unknown;
  clear(handle: unknown): void;
}

export interface StepDeps {
  clock: StepClock;
  hasPending(): boolean;   // a slider value waits to be written
  flush(): void;           // write it now, as one undo step
}

const RELEASE_WAIT = 400;  // a released drag is written after this, unless a double-click follows
const DOUBLE_CLICK = 500;  // a second press this soon after a release, at the same spot...
const SAME_SPOT = 4;       // ...within this many pixels, may be the second click of a double-click

// One gesture is one undo step. A drag is written shortly after release; a double-click (whose
// first click may jump the knob) and the reset it asks for are one step; a new gesture started
// before the last one was written writes the last one first.
export function sliderSteps(d: StepDeps) {
  let state: 'idle' | 'pressed' | 'again' | 'released' = 'idle';
  let releasedAt = -Infinity, timer: unknown = null, last: { target: unknown; x: number; y: number } | null = null;
  let lastVal: unknown = undefined;
  let resetting = false;
  const cancel = () => { if (timer !== null) d.clock.clear(timer); timer = null; };
  const writeNow = () => { cancel(); if (d.hasPending()) d.flush(); };
  return {
    moving: () => state === 'pressed' || state === 'again',
    isIdle: () => state === 'idle',
    press(target: unknown, x: number, y: number) {
      const quick = state === 'released' && d.clock.now() - releasedAt < DOUBLE_CLICK && last?.target === target
        && Math.abs(last.x - x) <= SAME_SPOT && Math.abs(last.y - y) <= SAME_SPOT;
      if (quick) { cancel(); state = 'again'; }
      else { writeNow(); state = 'pressed'; }
      last = { target, x, y };
    },
    // Called before a new slider value replaces the pending one.
    valueChanged(val?: unknown) {
      if (state === 'again' && !resetting) { writeNow(); state = 'pressed'; }   // the second press became a drag
      else if (state === 'released' && !resetting) {
        if (d.clock.now() - releasedAt >= 100 && (val === undefined || val !== lastVal)) {
          cancel();
          writeNow();
          state = 'idle';
        }
      }
      resetting = false;
      lastVal = val;
    },
    release() {
      if (state !== 'pressed' && state !== 'again') return;
      state = 'released';
      releasedAt = d.clock.now();
      cancel();
      if (d.hasPending()) timer = d.clock.set(() => { timer = null; state = 'idle'; d.flush(); }, RELEASE_WAIT);
    },
    // A double-click reset: joins the pending first click instead of following it.
    reset() { resetting = true; },
    // The slider's change event: a pointer release is written by release(); anything else at once.
    change() {
      if (d.clock.now() - releasedAt < 100) return;
      writeNow();
    },
  };
}

// Live preview: one render at a time; a request or an invalidation while a render is in flight
// renders again with the newest state when it finishes, so the last result shown is always the
// newest. `render` returns null when there is nothing to draw; `show` decides whether a finished
// render still fits (for example, the base it was made over).
export function latestDraft<T>(render: () => Promise<T | null>, show: (result: T) => void, onError: (error: unknown) => void) {
  let generation = 0, running = false, disposed = false;
  async function run() {
    if (running || disposed) return;
    running = true;
    try {
      let seen: number;
      do {
        seen = generation;
        const result = await render();
        if (result !== null && !disposed) show(result);
      } while (seen !== generation && !disposed);
    } catch (error) { onError(error); } finally { running = false; }
  }
  return {
    request() { ++generation; void run(); },
    invalidate() { ++generation; },
    dispose() { disposed = true; ++generation; },
  };
}
