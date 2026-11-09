// Exercise the editor's actual draft queue, including out-of-order render responses.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';
const source=fs.readFileSync(new URL('../src/views/ImageEditor.vue',import.meta.url),'utf8');
const code=ts.transpile(source.slice(source.indexOf('const TONE_KEYS ='),source.indexOf('// Where the Retouch areas')), {target:ts.ScriptTarget.ES2022});
const stepsSource=fs.readFileSync(new URL('../src/common/sliderSteps.ts',import.meta.url),'utf8');
const exports={};
new Function('exports',ts.transpile(stepsSource,{module:ts.ModuleKind.CommonJS}))(exports);
const settle=async()=>{for(let i=0;i<6;i++)await Promise.resolve();};
function setup(){
  const state={fileInfo:{value:{id:1}},imageReady:{value:true},activeTool:{value:''},picasaTools:{value:{fillLight:0}},savedEditState:{value:{recipe:{picasa:{fillLight:0}}}},effectPreviewSrc:{value:''},historyError:{value:''}};
  const pending=[];let changed;
  const make=new Function(...Object.keys(state),'invoke','onUnmounted','watch','setTimeout','clearTimeout','latestDraft',`let restoringHistory=false,editorUnmounted=false,baseSignature='base';${code};return {paintFillDraft:()=>fillDraft.request()};`);
  const api=make(...Object.values(state),()=>new Promise(resolve=>pending.push(resolve)),()=>{},(_read,fn)=>{changed=fn},()=>1,()=>{},exports.latestDraft);
  return {...state,pending,...api,change(value){state.picasaTools.value.fillLight=value;changed()}};
}
test('returning Fill Light to saved value clears painted and in-flight drafts',async()=>{
  const x=setup();x.change(50);x.paintFillDraft();x.pending.shift()('draft50');await settle();
  assert.equal(x.effectPreviewSrc.value,'draft50');x.change(0);assert.equal(x.effectPreviewSrc.value,'');x.paintFillDraft();await settle();
  assert.equal(x.effectPreviewSrc.value,'');x.change(60);x.paintFillDraft();x.change(0);x.pending.shift()('stale60');await settle();
  assert.equal(x.effectPreviewSrc.value,'');
});
test('continuous Fill Light dragging paints progress and finishes on the newest amount',async()=>{
  const x=setup();x.change(20);x.paintFillDraft();
  for(const amount of [20,40,60]){
    x.change(amount+20);x.paintFillDraft();
    assert.equal(x.pending.length,1,'only one render at a time');
    x.pending.shift()('draft'+amount);await settle();
    assert.equal(x.effectPreviewSrc.value,'draft'+amount,'a moving slider must not starve its preview');
  }
  x.pending.shift()('draft80');await settle();
  assert.equal(x.effectPreviewSrc.value,'draft80');assert.equal(x.pending.length,0);
});
