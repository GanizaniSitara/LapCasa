// Exercise the Settings filter with real Vue patches and browser highlight ranges.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import ts from 'typescript';
import { parse, compileStyle } from '@vue/compiler-sfc';
import postcss from 'postcss';
import tailwind from '@tailwindcss/postcss';

const source = fs.readFileSync(new URL('../src/views/Settings.vue', import.meta.url), 'utf8');
const filter = ts.transpile(source.slice(source.indexOf('function filterSettings()'), source.indexOf('onUpdated(filterSettings)')), { target: ts.ScriptTarget.ES2022 });
const lifecycle = ts.transpile(source.slice(source.indexOf('onUpdated(filterSettings)'),source.indexOf('const showManageLibraries')), {target:ts.ScriptTarget.ES2022});
const styles = parse(source).descriptor.styles.map(style => compileStyle({source:style.content,id:'data-v-test',scoped:style.scoped}).code).join('\n');
function runBrowser(script, extraEnv = {}) {
  assert.ok(process.env.LAPCASA_TEST_PYTHON,'Set LAPCASA_TEST_PYTHON to the provisioned test interpreter (tests/README.md)');
  const vue = createRequire(import.meta.url).resolve('vue/dist/vue.global.prod.js');
  const run = spawnSync(process.env.LAPCASA_TEST_PYTHON, [fileURLToPath(new URL('./settingsSearch.browser.py',import.meta.url)),vue], {env:{...process.env,...extraEnv},input:JSON.stringify({script}),encoding:'utf8',timeout:60000});
  assert.equal(run.status,0,run.stderr);
  return JSON.parse(run.stdout);
}

test('Settings browser driver preserves Unicode independently of the Python console encoding',()=>{
  const label='日本語 – Über';
  assert.equal(runBrowser('()=>'+JSON.stringify(label),{PYTHONIOENCODING:'cp1252'}),label);
});

test('Settings search highlights without detaching reactive text and clear restores every row', () => {
  const script = `async () => {
    const {ref,shallowRef,onUpdated,onUnmounted,onMounted,nextTick,createApp}=Vue;
    const searchQuery=ref(''), title=ref('Alpha'), settingsContentRef=ref(null), selected=ref(0);
    const tabMatchCounts=ref([0,0,0]),searchMatches=shallowRef([]);
    const settingsTabs=['General','Libraries','Child settings'];
    const settingSectionsCatalog=[{id:'appearance',title:'Appearance',keywords:['theme','zoom']}];
    const locale=ref('en'),searchInputRef=ref(null);
    const t=x=>locale.value==='de'&&x==='Current library'?'Aktuelle Bibliothek':x;
    const style=document.createElement('style');style.textContent=${JSON.stringify(styles)};document.head.append(style);
    const component={__scopeId:'data-v-test',template:\`<div ref="settingsContentRef" class="options-window">
      <div v-if="searchQuery || selected===0" data-settings-tab="0"><section data-settings-section="appearance"><h2>Appearance</h2>
        <div data-row="description"><div><div>Background jobs</div><p>Pause processing while editing</p></div></div>
        <div data-row="other"><div><div>Font size</div><p>Scale interface text</p></div></div>
      </section></div>
      <div v-if="searchQuery || selected===1" data-settings-tab="1"><section><h2>Library</h2><div data-row="reactive"><strong>{{ $t('Current library') }} {{title}}<span> status</span></strong></div></section></div>
      <section data-settings-tab="2"><section><h2>Child settings</h2><ChildSetting /></section></section>
      <Teleport v-for="(match,index) in searchMatches" :key="index" :to="match.row"><div data-search-breadcrumb>{{match.breadcrumb}}</div></Teleport>
      </div>\`,components:{ChildSetting:{__scopeId:'data-v-child',template:'<div><p data-child-label>Child-rendered setting</p></div>'}},setup(){${filter};${lifecycle};return {settingsContentRef,searchMatches,title,searchQuery,selected}}};
    const errors=[];const app=createApp(component);app.config.globalProperties.$t=t;app.config.errorHandler=e=>errors.push(String(e));
    const instance=app.mount('#app');
    const check=(condition,message)=>{if(!condition)throw Error(message)};
    const search=async q=>{searchQuery.value=q;await nextTick();await nextTick();await nextTick()};
    await search('pause');
    check(tabMatchCounts.value[0]===1 && tabMatchCounts.value[1]===0,'description-only count '+JSON.stringify({counts:tabMatchCounts.value,errors}));
    check(CSS.highlights.get('settings-match').size>0,'browser ranges highlight description');
    check(getComputedStyle(document.querySelector('[data-row="description"] p'),'::highlight(settings-match)').backgroundColor==='rgb(231, 198, 91)','compiled component CSS paints the highlight');
    check(document.querySelector('[data-row="other"]').classList.contains('settings-search-hidden'),'unmatched row hidden');
    await search('theme');check(tabMatchCounts.value[0]===1,'section keyword counts section once');
    check(!document.querySelector('[data-search-breadcrumb]').classList.contains('settings-search-hidden'),'section breadcrumb stays visible after Vue update');
    await search('Alpha');const reactive=document.querySelector('[data-row="reactive"] strong');const text=reactive.firstChild;
    title.value='Beta';await nextTick();await nextTick();
    check(reactive.firstChild===text && text.data==='Current library Beta','Vue text identity and reactive update survive search');
    check(tabMatchCounts.value[1]===0,'updated label no longer matches old term');
    await search('Beta');check(tabMatchCounts.value[1]===1,'new reactive label searchable');
    await search('Current library');check(tabMatchCounts.value[1]===1,'English locale label searchable');
    locale.value='de';await nextTick();await nextTick();await nextTick();
    check(tabMatchCounts.value[1]===0,'locale change automatically removes old-language matches');
    await search('Aktuelle Bibliothek');check(tabMatchCounts.value[1]===1,'translated label automatically searchable');
    await search('Child-rendered');
    check(getComputedStyle(document.querySelector('[data-child-label]'),'::highlight(settings-match)').backgroundColor==='rgb(231, 198, 91)','child component text receives the compiled highlight colour');
    await search('');check(!CSS.highlights.has('settings-match'),'clear removes ranges');
    check(document.querySelectorAll('.settings-search-hidden').length===0,'clear restores rows');
    check(document.querySelectorAll('[data-search-breadcrumb]').length===0,'Vue removes breadcrumbs');
    check(!document.querySelector('[data-settings-tab="1"]'),'clear unmounts nonselected panel');
    check(errors.length===0,errors.join('; '));app.unmount();return {passed:true};
  }`;
  assert.equal(runBrowser(script).passed,true);
  assert.throws(()=>runBrowser(script.replace('onUpdated(filterSettings);','')),/searchable|count|highlight|breadcrumb/,'missing production update hook must fail this case');
});

test('empty Settings search does not walk row text on unrelated updates',()=>{
  assert.equal(runBrowser(`async()=>{
    const {ref,shallowRef,onUpdated,onMounted,onUnmounted,nextTick,createApp}=Vue;
    const searchQuery=ref(''),settingsContentRef=ref(null),searchInputRef=ref(null),tabMatchCounts=ref([0]),searchMatches=shallowRef([]),checked=ref(false);
    const settingsTabs=['General'],settingSectionsCatalog=[],t=x=>x;
    let walks=0;const walk=document.createTreeWalker.bind(document);document.createTreeWalker=(...args)=>{walks++;return walk(...args)};
    const component={template:'<div ref="settingsContentRef"><section data-settings-tab="0"><section><h2>General</h2><div>Search target <input type="checkbox" v-model="checked"></div></section></section><span>{{searchQuery}}</span></div>',setup(){${filter};${lifecycle};return {settingsContentRef,searchQuery,checked}}};
    const app=createApp(component);app.mount('#app');const settle=async()=>{await nextTick();await nextTick();await nextTick()};await settle();
    if(walks)throw Error('empty initial search walked text');
    checked.value=true;await settle();if(walks)throw Error('empty toggle update walked text');
    searchQuery.value='target';await settle();if(!walks)throw Error('active search was not exercised');
    searchQuery.value='';await settle();walks=0;checked.value=false;await settle();
    if(walks)throw Error('cleared search still walks text');
    if(document.querySelectorAll('.settings-search-row,.settings-search-hidden').length||searchMatches.value.length)throw Error('search state not cleared');
    app.unmount();return {passed:true};
  }`).passed,true);
});

test('matched fixed-height Settings row fits its breadcrumb and two-line label',async()=>{
  const descriptor=parse(source).descriptor;
  const find=(node,predicate)=>predicate(node)?node:(node.children||[]).map(child=>find(child,predicate)).find(Boolean);
  const row=find(descriptor.template.ast,n=>n.loc?.source?.includes('Each slide zooms slowly')&&n.props?.some(p=>p.name==='class'&&p.value?.content.includes('h-8')));
  assert.ok(row,'actual Pan and zoom fixed-height row');
  const utilities=(await postcss([tailwind()]).process('@import "tailwindcss" source(none); @source inline("flex flex-col items-center justify-between h-8 gap-0.5 text-sm leading-5 text-xs");',{from:fileURLToPath(new URL('../src/assets/app.css',import.meta.url))})).css;
  const template=`<div ref="settingsContentRef" class="options-window"><section data-settings-tab="0"><section><h2>Slideshow</h2>${row.loc.source}</section></section><Teleport v-for="(match,index) in searchMatches" :key="index" :to="match.row"><div class="settings-search-breadcrumb" data-search-breadcrumb>{{match.breadcrumb}}</div></Teleport><span>{{searchQuery}}</span></div>`;
  assert.equal(runBrowser(`async()=>{
    const {ref,reactive,shallowRef,onUpdated,onMounted,onUnmounted,nextTick,createApp}=Vue;
    const searchQuery=ref('Pan and zoom'),settingsContentRef=ref(null),searchInputRef=ref(null),tabMatchCounts=ref([0]),searchMatches=shallowRef([]),config=reactive({settings:{slideShowPanZoom:true}});
    const settingsTabs=['Viewer'],settingSectionsCatalog=[],t=x=>x;
    const style=document.createElement('style');style.textContent=${JSON.stringify(utilities+'\n'+styles)};document.head.append(style);
    const component={__scopeId:'data-v-test',template:${JSON.stringify(template)},setup(){${filter};${lifecycle};return {settingsContentRef,searchMatches,searchQuery,config}}};
    const app=createApp(component);app.mount('#app');await nextTick();await nextTick();await nextTick();
    const row=document.querySelector('.h-8'),label=row.firstElementChild,crumb=row.querySelector('[data-search-breadcrumb]');
    const a=row.getBoundingClientRect(),b=label.getBoundingClientRect(),c=crumb.getBoundingClientRect();
    if(b.top<c.bottom-.1||b.bottom>a.bottom+.1)throw Error('matched h-8 label overlaps breadcrumb or next row: '+JSON.stringify({row:a.toJSON(),label:b.toJSON(),breadcrumb:c.toJSON()}));
    searchQuery.value='';await nextTick();await nextTick();await nextTick();
    if(row.getBoundingClientRect().height!==32)throw Error('cleared search changed the original row height');
    searchQuery.value='Pan and zoom';await nextTick();await nextTick();await nextTick();
    return {passed:true};
  }`).passed,true);
});

test('actual Settings sections expose shortcut and slideshow catalog keywords', () => {
  const descriptor = parse(source).descriptor;
  const find = (node, predicate) => predicate(node) ? node : (node.children || []).map(child => find(child, predicate)).find(Boolean);
  const section = find(descriptor.template.ast, node => node.props?.some(p => p.name==='data-settings-section' && p.value?.content==='view-disp'));
  const shortcuts = find(descriptor.template.ast, node => node.props?.some(p => p.name==='bind' && p.arg?.content==='data-settings-tab' && p.exp?.content==='6'));
  const ast = ts.createSourceFile('Settings.ts', descriptor.scriptSetup.content, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const declaration = name => ts.transpile(ast.statements.find(s => ts.isVariableStatement(s) && s.declarationList.declarations.some(d => d.name.getText(ast)===name)).getText(ast));
  const template = `<div ref="settingsContentRef" class="options-window"><div data-settings-tab="3">${section.loc.source}</div>${shortcuts.loc.source}</div>`;
  assert.equal(runBrowser(`async()=>{
    const {ref,shallowRef,onUpdated,onMounted,onUnmounted,nextTick,createApp,reactive}=Vue;
    const searchInputRef=ref(null);
    const searchQuery=ref(''),settingsContentRef=ref(null),tabMatchCounts=ref(Array(9).fill(0)),searchMatches=shallowRef([]);
    const t=x=>x;${declaration('settingsTabs')};${declaration('settingSectionsCatalog')};
    const config=reactive({settings:{tabIndex:6,viewBackground:'dark',slideShowTransition:'fade',slideShowPanZoom:true}});
    const shortcutSections=[{key:'navigation',title:'Navigation',items:[{actionId:'next',label:'Next picture',keys:['Right']},{actionId:'zoom',label:'Zoom in',keys:['+']}]},{key:'editing',title:'Editing',items:[{actionId:'edit',label:'Edit photo',keys:['E']}]}];
    const viewBackgroundOptions=[{value:'dark',label:'Dark'}],slideShowTransitionOptions=[{value:'fade',label:'Fade'}];
    const component={template:${JSON.stringify(template)},setup(){${filter};${lifecycle};return {config,shortcutSections,viewBackgroundOptions,slideShowTransitionOptions,searchQuery,settingsContentRef,$t:t}}};
    const errors=[],app=createApp(component);app.config.globalProperties.$t=t;app.config.errorHandler=e=>errors.push(String(e));const vm=app.mount('#app');
    for(const [query,tab] of [['hotkeys',6],['keybindings',6],['navigation',6],['zoom',6],['edit',6],['slideshow',3],['delay',3],['transition',3]]){
      searchQuery.value=query;await nextTick();await nextTick();await nextTick();
      if(!tabMatchCounts.value[tab])throw Error(query+' has no catalog matches');
      const panel=document.querySelector('[data-settings-tab="'+tab+'"]');
      if(![...panel.children].some(s=>!s.classList.contains('settings-search-hidden')))throw Error(query+' hides all sections');
      if(query==='edit'){
        const navigation=[...panel.children].find(section=>section.textContent.includes('Next picture'));
        if(!navigation.classList.contains('settings-search-hidden'))throw Error('edit search expands unrelated Navigation shortcut group');
      }
      if(query==='transition'){
        const background=panel.querySelector('[data-settings-section="view-disp"] select');
        if(!background.parentElement.classList.contains('settings-search-hidden'))throw Error('transition search includes unrelated background row');
      }
    }
    if(errors.length)throw Error(errors.join('; '));app.unmount();return {passed:true};
  }`).passed,true);
});

test('Settings keeps nonselected panels and hardware/about fetch components lazy', () => {
  assert.equal((source.match(/v-show="!!searchQuery/g)||[]).length,0);
  assert.match(source,/<GpuAcceleration v-if="config.settings.tabIndex === 0"/);
  assert.match(source,/<SettingsAbout v-if="config.settings.tabIndex === 7"/);
});

test('lazy hardware/About search uses translated stand-ins and matches actual child labels on every tab', () => {
  const descriptor=parse(source).descriptor;
  const find=(node,predicate)=>predicate(node)?node:(node.children||[]).map(child=>find(child,predicate)).find(Boolean);
  const gpu=find(descriptor.template.ast,n=>n.props?.some(p=>p.name==='data-settings-section'&&p.value?.content==='gen-gpu'));
  const about=find(descriptor.template.ast,n=>n.props?.some(p=>p.name==='data-settings-section'&&p.value?.content==='about'));
  const ast=ts.createSourceFile('Settings.ts',descriptor.scriptSetup.content,ts.ScriptTarget.Latest,true,ts.ScriptKind.TS);
  const declaration=name=>ts.transpile(ast.statements.find(s=>ts.isVariableStatement(s)&&s.declarationList.declarations.some(d=>d.name.getText(ast)===name)).getText(ast));
  const gpuTemplate=parse(fs.readFileSync(new URL('../src/components/GpuAcceleration.vue',import.meta.url),'utf8')).descriptor.template.content;
  const aboutTemplate=parse(fs.readFileSync(new URL('../src/components/SettingsAbout.vue',import.meta.url),'utf8')).descriptor.template.content;
  const translated=JSON.parse(fs.readFileSync(new URL('../src/locales/de.json',import.meta.url),'utf8')).settings.about.package;
  const template=`<div ref="settingsContentRef"><input v-model="searchQuery"><div data-settings-tab="0">${gpu.loc.source}</div>${about.loc.source}</div>`;
  assert.equal(runBrowser(`async()=>{
    const {ref,shallowRef,reactive,onUpdated,onMounted,onUnmounted,nextTick,createApp}=Vue;
    const searchInputRef=ref(null);
    const labels=${JSON.stringify(translated)};
    const translations={'settings.search_preview.gpu_title':'Beschleunigung',
      'settings.search_preview.gpu_description':'Grafikkarte und KI',
      'settings.search_preview.open_gpu':'Grafikeinstellungen öffnen',
      'settings.search_preview.about_title':'Über LapCasa',
      'settings.search_preview.about_description':'Version und Lizenz',
      'settings.search_preview.open_about':'Über öffnen'};
    const t=key=>translations[key]||labels[key.replace('settings.about.package.','')]||key;
    ${declaration('settingsTabs')};${declaration('settingSectionsCatalog')};
    const config=reactive({settings:{tabIndex:6}}),searchQuery=ref('gpu'),settingsContentRef=ref(null),tabMatchCounts=ref(Array(9).fill(0)),searchMatches=shallowRef([]);
    const GpuAcceleration={template:${JSON.stringify(gpuTemplate)},setup:()=>({prompt:false,visible:true,status:{hardware:{name:'Test GPU',cuda:true,vram_mb:1024,driver:'test'},engines:{}},failedEngines:{},busy:false,terms:false,error:'',phase:'',percent:0,install:()=>{},dismiss:()=>{}})};
    const SettingsAbout={template:${JSON.stringify(aboutTemplate)},setup:()=>({iconLogo:'',packageInfo:{name:'LapCasa',license:'GPL',repository:'#'},displayVersion:'test',buildTime:'test',issuesUrl:'#',privacyUrl:'#'})};
    const component={template:${JSON.stringify(template)},components:{GpuAcceleration,SettingsAbout},setup(){${filter};${lifecycle};return {config,searchQuery,settingsContentRef,selectSettingsCategory:()=>{}}}};
    const errors=[],app=createApp(component);app.config.globalProperties.$t=t;app.config.errorHandler=e=>errors.push(String(e));const vm=app.mount('#app');await nextTick();
    if(!document.body.textContent.includes('Beschleunigung')||!document.body.textContent.includes('Über öffnen'))throw Error('lazy stand-ins are not translated');
    const queries=[['Download / repair GPU add-on',0],['AI engines',0],['cuDNN',0],...['version','build_time','license','link','github','feedback','privacy','app_description'].map(key=>[labels[key],7])];
    for(const [query,tab] of queries){
      let previous;
      for(const selected of [tab,6]){
        config.settings.tabIndex=selected;searchQuery.value=query;await nextTick();await nextTick();await nextTick();
        const count=tabMatchCounts.value[tab];
        if(!count)throw Error(query+' missing when selected tab is '+selected);
        if(previous!==undefined&&count!==previous)throw Error(query+' count depends on selected tab');
        previous=count;
      }
    }
    if(errors.length)throw Error(errors.join('; '));app.unmount();return {passed:true};
  }`).passed,true);
});

test('every supported locale supplies the lazy search preview translations',()=>{
  for(const file of fs.readdirSync(new URL('../src/locales/',import.meta.url)).filter(name=>name.endsWith('.json'))){
    const messages=JSON.parse(fs.readFileSync(new URL('../src/locales/'+file,import.meta.url),'utf8'));
    for(const key of ['gpu_title','gpu_description','open_gpu','about_title','about_description','open_about']){
      assert.equal(typeof messages.settings.search_preview?.[key],'string',file+' '+key);
    }
  }
});
