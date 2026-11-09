import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';

const source = fs.readFileSync(new URL('../src/common/shortcuts.ts', import.meta.url), 'utf8');
const js = ts.transpile(source.replace(/^export /gm, ''), { target: ts.ScriptTarget.ES2022 });
const registry = new Function(js + '; return { SHORTCUTS, getShortcutConflicts, matchesShortcut, getShortcutLabels };')();

const platforms = ['windows', 'linux', 'mac'];
const contexts = [...new Set(registry.SHORTCUTS.flatMap((shortcut) => shortcut.contexts))];

test('no context binds the same chord twice', () => {
  for (const platform of platforms) {
    for (const context of contexts) {
      assert.deepEqual(registry.getShortcutConflicts(context, platform), [], `${context} on ${platform}`);
    }
  }
});

test('the conflict check sees a key binding and a code binding as one chord', () => {
  const extra = { id: 'test.clash', contexts: ['content'], defaultBindings: [{ key: 'r', label: 'r' }] };
  registry.SHORTCUTS.push(extra);
  try {
    assert.deepEqual(registry.getShortcutConflicts('content', 'windows'),
      [{ signature: 'KeyR', actions: ['meta.rotate', 'test.clash'] }]);
  } finally {
    registry.SHORTCUTS.pop();
  }
});

test('LAP-101 Library and editor chords', () => {
  const key = (k, code, mods = {}) => ({ key: k, code, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, ...mods });
  const ctrl = { ctrlKey: true };
  const is = (action, event) => registry.matchesShortcut(action, event, 'windows');
  assert.ok(is('file.editImage', key('Enter', 'Enter')));
  assert.ok(is('file.editImage', key('3', 'Digit3', ctrl)));
  assert.ok(!is('view.quickPreview', key('Enter', 'Enter')));
  assert.ok(is('view.quickPreview', key(' ', 'Space')));
  assert.ok(is('view.thumbnailsSmall', key('1', 'Digit1', ctrl)));
  assert.ok(is('view.thumbnailsNormal', key('2', 'Digit2', ctrl)));
  assert.ok(is('slideshow.toggle', key('4', 'Digit4', ctrl)));
  assert.ok(is('view.timeline', key('5', 'Digit5', ctrl)));
  assert.ok(is('editor.tabBasic', key('1', 'Digit1', ctrl)));
  assert.ok(is('editor.tabTuning', key('2', 'Digit2', ctrl)));
  assert.ok(is('editor.tabEffects', key('3', 'Digit3', ctrl)));
  assert.ok(is('meta.rotate', key('r', 'KeyR')));
  assert.ok(is('meta.rotateCounterclockwise', key('R', 'KeyR', { shiftKey: true })));
  assert.ok(is('meta.info', key('i', 'KeyI', ctrl)));
  assert.ok(!is('file.invertSelection', key('i', 'KeyI', ctrl)));
  assert.ok(is('file.invertSelection', key('I', 'KeyI', { ctrlKey: true, shiftKey: true })));
  assert.ok(!is('meta.info', key('I', 'KeyI', { ctrlKey: true, shiftKey: true })));
  assert.deepEqual(registry.getShortcutLabels('file.invertSelection', 'windows'), ['Ctrl+Shift+I']);
});

test('LAP-107 Automask chords', () => {
  const key = (k, code, mods = {}) => ({ key: k, code, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, ...mods });
  const is = (action, event) => registry.matchesShortcut(action, event, 'windows');
  assert.ok(is('editor.automask', key('m', 'KeyM')));
  ['Whole', 'Subject', 'Sky', 'People', 'Background'].forEach((name, i) =>
    assert.ok(is(`editor.automask${name}`, key(String(i + 1), `Digit${i + 1}`)), name));
  assert.ok(!is('editor.automaskSky', key('3', 'Digit3', { ctrlKey: true })), 'Ctrl+3 stays the Effects tab');
  assert.ok(is('editor.tabEffects', key('3', 'Digit3', { ctrlKey: true })));
  assert.ok(is('editor.automaskSelect', key('s', 'KeyS')));
  assert.ok(is('editor.automaskShowMask', key('x', 'KeyX')));
  assert.ok(is('editor.automaskRemove', key('Delete', 'Delete')));
});
