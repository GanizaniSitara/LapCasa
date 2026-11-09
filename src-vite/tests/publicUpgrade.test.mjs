import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import assert from 'node:assert/strict';

const editor = readFileSync(new URL('../src/views/ImageEditor.vue', import.meta.url), 'utf8');
const startup = readFileSync(new URL('../src/main.js', import.meta.url), 'utf8');
const savedState = editor.match(/const savedEditState = computed\(\(\) => \{([\s\S]*?)\n\}\);/)[1];
const gridDefault = startup.match(/if \(isMainWindow && config\.settings\.grid[^\n]*\) \{[\s\S]*?\n\}/)[0];

test('old public editing controls retain their saved effects', () => {
  const legacy = { recipe: { tools: { fillLight: 27, effects: [{ name: 'Sepia', amount: 0.4 }] } }, revision: 9 };
  const source = JSON.stringify(legacy);
  const read = new Function('fileInfo', savedState);
  const result = read({ value: { edits: source } });
  assert.deepEqual(result.recipe.picasa, legacy.recipe.tools);
  assert.equal(result.revision, 9);
  assert.equal(source, JSON.stringify(legacy), 'reading does not rewrite the stored document');
});

test('current editing controls take precedence when both field names exist', () => {
  const result = new Function('fileInfo', savedState)({ value: { edits: JSON.stringify({ recipe: { picasa: { fillLight: 7 }, tools: { fillLight: 99 } } }) } });
  assert.equal(result.recipe.picasa.fillLight, 7);
});

test('previous public grid preference is retained after its default migration', () => {
  const config = { settings: { grid: { scaling: 1, scalingFitDefault: true, scalingPicasaDefault: false } } };
  new Function('config', 'isMainWindow', gridDefault)(config, true);
  assert.equal(config.settings.grid.scaling, 1);
});

test('fresh grid settings still receive the approved default only in the main window', () => {
  for (const main of [false, true]) {
    const config = { settings: { grid: { scaling: 1, scalingPicasaDefault: false } } };
    new Function('config', 'isMainWindow', gridDefault)(config, main);
    assert.equal(config.settings.grid.scaling, main ? 0 : 1);
  }
});
