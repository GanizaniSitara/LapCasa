// Exercise production editor watchers, recipe restoration, draft inputs and saves.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';
import { parse } from '@vue/compiler-sfc';
import { ref, computed, watch, nextTick, effectScope } from 'vue';

const source=fs.readFileSync(new URL('../src/views/ImageEditor.vue',import.meta.url),'utf8');
const ast=ts.createSourceFile('ImageEditor.ts',parse(source).descriptor.scriptSetup.content,ts.ScriptTarget.Latest,true,ts.ScriptKind.TS);
const steps={};
new Function('exports',ts.transpile(fs.readFileSync(new URL('../src/common/sliderSteps.ts',import.meta.url),'utf8'),{module:ts.ModuleKind.CommonJS}))(steps);
function declaration(name) {
  const node=ast.statements.find(s=>ts.isFunctionDeclaration(s)?s.name?.text===name:
    ts.isVariableStatement(s)&&s.declarationList.declarations.some(d=>d.name.getText(ast)===name));
  assert.ok(node,`Production editor declaration missing: ${name}`);
  return node.getText(ast);
}
function evaluate(code, bindings) {
  return new Function(...Object.keys(bindings),ts.transpile(code,{target:ts.ScriptTarget.ES2022}))(...Object.values(bindings));
}

test('photo readiness prepares selection before a click and shares the in-flight load',async()=>{
  const scope=effectScope(),calls=[],pending=[];
  const fileInfo=ref({id:0}),imageReady=ref(false);
  const code=source.slice(source.indexOf('const selectStatus ='),source.indexOf('// A click in Click to select mode'));
  try {
    const api=scope.run(()=>evaluate(`${code};return {prepareSelect,selectPreparing};`,{
      ref,computed,watch,fileInfo,imageReady,activeTool:ref(''),maskMode:ref(false),selectPoints:ref([]),
      cropStatus:ref(0),imageStyle:ref({}),MASK_EFFECTS:[],editorUnmounted:false,
      onMounted:()=>{},onUnmounted:()=>{},listen:()=>Promise.resolve(()=>{}),
      invoke:(command,args)=>{calls.push({command,args});return new Promise(resolve=>pending.push(resolve));},
    }));
    await nextTick();assert.equal(calls.length,0,'no model preparation without an editor photo');
    fileInfo.value={id:101};await nextTick();assert.equal(calls.length,0,'wait for the photo to be ready');
    imageReady.value=true;await nextTick();
    assert.deepEqual(calls,[{command:'select_prepare',args:{fileId:101}}]);
    const firstClick=api.prepareSelect();assert.equal(calls.length,1,'a click shares the proactive preparation');
    pending.shift()();await firstClick;assert.equal(api.selectPreparing.value,false);
    await api.prepareSelect();assert.equal(calls.length,1,'prepared photo is reused');
    imageReady.value=false;fileInfo.value={id:202};await nextTick();assert.equal(calls.length,1);
    imageReady.value=true;await nextTick();assert.equal(calls[1].args.fileId,202);
    pending.shift()();await api.prepareSelect();
  } finally {scope.stop();}
});

test('Auto Contrast and Auto Color survive Fill Light draft and recipe flush',async()=>{
  const scope=effectScope(),calls=[],saves=[];
  const automatic={autoContrast:{low:[5,7,9],gain:[1.1,1.2,1.3]},autoColor:[1.02,0.98,1.01]};
  let server={recipe:{picasa:{fillLight:0}}};
  const fileInfo=ref({id:1,file_path:'/photos/input.jpg',width:80,height:60,edits:JSON.stringify(server)});
  const refs=Object.fromEntries(['imageReady','activeTool','initialImageSrc','imageSrc','imageWidth','imageHeight','isPortrait',
    'containerRect','containerBounds','enableTransition','cropStatus','cropApplied','cropAngle','redEye','rotate',
    'isFlippedX','isFlippedY','brightness','contrast','saturation','hue','blur','selectedFilter','historyError','effectPreviewSrc']
    .map(name=>[name,ref(name==='activeTool'?'':0)]));
  refs.imageReady.value=true;
  const bindings={ref,computed,watch,nextTick,...steps,...refs,fileInfo,initialDisplayRotate:ref(0),
    containerRef:ref({getBoundingClientRect:()=>({left:0,top:0,width:800,height:600})}),containerPadding:0,
    crop:ref({left:0,top:0,width:0,height:0}),resizeOutput:ref({hasResize:false,width:80,height:60}),
    newFileName:ref('input'),config:{settings:{},imageEditor:{quality:0}},
    uiStore:{activeAdjustments:{},clearActiveAdjustments:()=>{}},isPortraitForRotation:()=>false,
    getPreviewUrl:()=>'/preview',getSelectedOutputFormat:()=> 'jpg',getFullPath:(a,b)=>a+'/'+b,
    getFolderPath:()=>'/photos',combineFileName:(a,b)=>a+'.'+b,sendToParent:()=>{},
    onImageLoad:async()=>{refs.imageReady.value=true;},onUnmounted:()=>{},
    requestAnimationFrame:fn=>queueMicrotask(fn),setTimeout:()=>1,clearTimeout:()=>{},
    getFileInfo:async()=>({...fileInfo.value,edits:JSON.stringify(server)}),
    invoke:async(command,args)=>{
      calls.push({command,args});
      if(command==='batch_edit_images'){
        const key=args.op==='auto_contrast'?'autoContrast':'autoColor';
        server.recipe.picasa[key]=automatic[key];return {failed:[]};
      }
      if(command==='render_edit_draft')return '/draft';
      throw Error('Unexpected command '+command);
    },
    saveImageRecipe:async(id,params)=>{saves.push({id,params});server={recipe:params};return JSON.stringify(server);},
  };
  class RangeInput {type='range';}
  bindings.HTMLInputElement=RangeInput;bindings.document={activeElement:new RangeInput()};
  const names=['emptyTools','picasaTools','savedEditState','initEditImageLoadingId','initEditImage','setEditParams',
    'restoringHistory','pendingRecipe','recipeTimer','recipeWrites','persistRecipeStep','flushRecipeSteps',
    'restoreEditorControls','selectPicasaTool','recipeBaseSignature','refreshBasePreview','sliderGesture','sliderMoving'];
  const draft=source.slice(source.indexOf('const TONE_KEYS ='),source.indexOf('// Where the Retouch areas'));
  const recipeWatch=source.slice(source.indexOf('watch([rotate, isFlippedX'),source.indexOf('</script>'));
  try {
    const api=scope.run(()=>evaluate(`let autoPresetRequestId=0,editorUnmounted=false,baseSignature='';
      ${names.map(declaration).join('\n')};${draft};${recipeWatch};
      return {selectPicasaTool,picasaTools,paintFillDraft:()=>fillDraft.request(),flushRecipeSteps};`,bindings));
    for(const tool of ['Auto Contrast','Auto Color']){
      await api.selectPicasaTool(tool);await nextTick();
      const expected=tool==='Auto Contrast'?{autoContrast:automatic.autoContrast}:automatic;
      for(const [key,value] of Object.entries(expected))assert.deepEqual(api.picasaTools.value[key],value);
      api.picasaTools.value.fillLight+=20;await nextTick();api.paintFillDraft();
      for(let i=0;i<6;i++)await Promise.resolve();
      const draft=calls.filter(c=>c.command==='render_edit_draft').at(-1).args.picasa;
      for(const [key,value] of Object.entries(expected))assert.deepEqual(draft[key],value,`${key} in draft`);
      await api.flushRecipeSteps();
      const saved=saves.at(-1).params.picasa;
      for(const [key,value] of Object.entries(expected))assert.deepEqual(saved[key],value,`${key} in saved recipe`);
      assert.equal(saved.fillLight,tool==='Auto Contrast'?20:40);
    }
  } finally {scope.stop();}
});
