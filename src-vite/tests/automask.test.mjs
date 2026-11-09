import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';

const source = fs.readFileSync(new URL('../src/common/automask.ts', import.meta.url), 'utf8');
const js = ts.transpile(source.replace(/^export /gm, ''), { target: ts.ScriptTarget.ES2022 });
const am = new Function(js + '; return { bend };')();

test('the drawn bend matches t_edit_local slider_curve: identity at 0, ends fixed, sliders where they say', () => {
  for (let i = 0; i <= 20; i++) assert.equal(am.bend(i / 20, 0, 0, 0, 0), i / 20);
  assert.ok(Math.abs(am.bend(1, 1, -1, 0, 0) - 1) < 1e-9 && Math.abs(am.bend(0, 1, -1, 0, 0)) < 1e-9);
  assert.ok(am.bend(1 / 3, 1, 0, 0, 0) > 1 / 3 + 0.25, 'Shadows peaks a third of the way up');
  assert.ok(am.bend(2 / 3, 0, -1, 0, 0) < 2 / 3 - 0.25, 'Highlights peaks two thirds up');
});
