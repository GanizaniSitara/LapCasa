import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';

const source = fs.readFileSync(new URL('../src/common/sliderSteps.ts', import.meta.url), 'utf8');
const js = ts.transpile(source.replace(/^export /gm, ''), { target: ts.ScriptTarget.ES2022 });
const ss = new Function(js + '; return { sliderSteps, latestDraft };')();

// A fake clock: timers run only when the test advances time.
function clock() {
  let t = 0, seq = 0; const timers = new Map();
  return {
    now: () => t,
    set(fn, ms) { const id = ++seq; timers.set(id, { at: t + ms, fn }); return id; },
    clear(id) { timers.delete(id); },
    advance(ms) {
      const end = t + ms;
      for (;;) {
        const due = [...timers.entries()].filter(([, v]) => v.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
        if (!due) break;
        timers.delete(due[0]); t = due[1].at; due[1].fn();
      }
      t = end;
    },
  };
}

// The editor around the slider: a pending value, written as one undo step per flush.
function editor() {
  const c = clock(), saved = [];
  let pending = null, value = 0;
  const steps = ss.sliderSteps({ clock: c, hasPending: () => pending !== null, flush: () => { if (pending !== null) { saved.push(pending); pending = null; } } });
  const slider = {};
  return {
    c, saved, steps,
    set(v) { steps.valueChanged(v); value = v; pending = v; },   // the editor's watcher after an input event
    get value() { return value; },
    drag(x, to) { steps.press(slider, x, 10); this.set(to); c.advance(50); steps.release(); },
    click(x, to) { steps.press(slider, x, 10); if (to !== undefined) this.set(to); c.advance(5); steps.release(); },
    dblclickReset() { steps.reset(); this.set(0); },
  };
}

// Simulates ImageEditor.vue's startRangeDrag / endRangeDrag and pending recipeTimer flush logic.
function editorWithRecipeTimer() {
  const c = clock(), saved = [];
  let pending = null, value = 0, recipeTimer = null;
  const flushRecipeSteps = () => {
    if (recipeTimer !== null) { c.clear(recipeTimer); recipeTimer = null; }
    if (pending !== null) { saved.push(pending); pending = null; }
  };
  const steps = ss.sliderSteps({
    clock: c,
    hasPending: () => pending !== null,
    flush: flushRecipeSteps,
  });
  const slider = {};
  const startRangeDrag = (x, y = 10) => {
    if (recipeTimer !== null) {
      if (steps.isIdle()) {
        flushRecipeSteps();
      } else {
        c.clear(recipeTimer);
        recipeTimer = null;
      }
    }
    steps.press(slider, x, y);
  };
  const onInput = (v) => {
    steps.valueChanged(v);
    value = v;
    pending = v;
    if (recipeTimer !== null) c.clear(recipeTimer);
    recipeTimer = steps.moving() ? null : c.set(flushRecipeSteps, 200);
  };
  const endRangeDrag = () => {
    steps.release();
  };
  return {
    c, saved, steps,
    startRangeDrag, onInput, endRangeDrag,
    dblclickReset() {
      steps.reset();
      onInput(0);
    },
  };
}

test('two separate drags are two undo steps, even when the second starts right after the first', () => {
  const e = editor();
  e.drag(100, 30);
  e.c.advance(150);              // the second drag starts 150 ms after the first was released
  e.drag(160, 80);
  e.c.advance(2000);
  assert.deepEqual(e.saved, [30, 80]);
});

test('a double-click reset is one undo step: the first click\'s jump is not kept', () => {
  const e = editor();
  e.click(120, 45);              // first click jumps the knob to 45
  e.c.advance(60);
  e.click(120);                  // second click at the same spot, no new value
  e.dblclickReset();
  e.c.advance(2000);
  assert.deepEqual(e.saved, [0]);
});

test('a quick second press that turns into a drag keeps the first gesture as its own step', () => {
  const e = editor();
  e.click(120, 45);
  e.c.advance(60);
  e.drag(120, 70);               // pressed again at the same spot, then dragged
  e.c.advance(2000);
  assert.deepEqual(e.saved, [45, 70]);
});

test('a keyboard change is written at once; the change after a pointer release waits for the release', () => {
  const e = editor();
  e.set(3);
  e.steps.change();
  assert.deepEqual(e.saved, [3]);
  e.drag(100, 20);
  e.steps.change();              // fires with the release
  assert.deepEqual(e.saved, [3]);
  e.c.advance(2000);
  assert.deepEqual(e.saved, [3, 20]);
});

test('a drag followed by a keyboard nudge within 400 ms produces two undo steps', () => {
  const e = editor();
  e.drag(100, 20);
  e.c.advance(150);              // 150 ms after release, user taps arrow key to 21
  e.set(21);
  e.steps.change();
  e.c.advance(2000);
  assert.deepEqual(e.saved, [20, 21]);
});

test('multiple edits in sequence stay distinct: drag, then arrow nudge, then another drag', () => {
  const e = editor();
  e.drag(100, 20);
  e.c.advance(150);
  e.set(21);
  e.steps.change();
  e.c.advance(150);
  e.drag(160, 50);
  e.c.advance(2000);
  assert.deepEqual(e.saved, [20, 21, 50]);
});

test('full double-click sequence (press, release, press, dblclick reset) yields exactly one saved step at 0', () => {
  const e = editorWithRecipeTimer();
  // First click jumps knob to 45
  e.startRangeDrag(120);
  e.onInput(45);
  e.c.advance(50);
  e.endRangeDrag();

  // Second click within double-click window at the same spot
  e.c.advance(60);
  e.startRangeDrag(120);
  // Browser fires dblclick event calling reset
  e.dblclickReset();
  e.c.advance(20);
  e.endRangeDrag();

  // Run through release timer and recipe flush
  e.c.advance(2000);
  assert.deepEqual(e.saved, [0]);
});

test('a late input event with the same value right after release does not split into two steps', () => {
  const e = editor();
  e.drag(100, 30);
  e.c.advance(20);               // 20 ms after release, late input with same value 30 arrives
  e.set(30);
  e.c.advance(2000);
  assert.deepEqual(e.saved, [30]);
});

test('a late input event with a 1-step-off value right after release updates pending without splitting', () => {
  const e = editor();
  e.drag(100, 30);
  e.c.advance(20);               // 20 ms after release (< 100 ms threshold), late input 31 arrives
  e.set(31);
  e.c.advance(2000);
  assert.deepEqual(e.saved, [31]);
});

// The live preview: renders resolve only when the test says so.
function preview() {
  const shown = [], requests = [], waiting = [];
  const state = { value: 0, base: 0 };
  const draft = ss.latestDraft(
    () => { const asked = { value: state.value, base: state.base }; requests.push(asked.value); return new Promise(r => waiting.push(() => r(asked))); },
    (r) => { if (r.base === state.base) shown.push(r.value); },   // the editor shows a draft only over the base it was made for
    (error) => { throw error; });
  const settle = async () => { while (waiting.length) { waiting.shift()(); await new Promise(r => setTimeout(r, 0)); } };
  return { shown, requests, state, draft, settle };
}

test('RING-008 repro: the preview ends on the newest slider value when the base changes mid-render', async () => {
  const p = preview();
  p.state.value = 10; p.draft.request();
  p.state.value = 70; p.draft.request();     // slider moved on while 10 renders
  p.state.base = 1; p.draft.invalidate();    // a saved step replaced the base preview meanwhile
  await p.settle();
  assert.deepEqual(p.requests, [10, 70]);
  assert.equal(p.shown.at(-1), 70);
});

test('the preview never shows an older value after a newer one', async () => {
  const p = preview();
  for (const v of [5, 15, 25, 35]) { p.state.value = v; p.draft.request(); }
  await p.settle();
  assert.equal(p.shown.at(-1), 35);
  assert.deepEqual([...p.shown].sort((a, b) => a - b), p.shown);
  assert.ok(p.requests.length <= 2, `one render in flight plus the newest: ${p.requests}`);
});
