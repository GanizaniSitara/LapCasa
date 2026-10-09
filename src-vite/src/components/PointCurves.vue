<template>
  <div data-editor-curves>
    <div class="flex mt-1 mb-1" role="group" aria-label="Curve channel">
      <button v-for="c in curveChannels" :key="c" type="button" :aria-pressed="channel===c"
        :data-curve-channel="c" class="flex-1 py-1 border border-base-content/20"
        :style="channel===c ? {background: colors[c], color: c==='rgb' ? '#15191c' : '#fff'} : {}"
        :disabled="disabled" @click="channel=c">{{ labels[c] }}</button>
    </div>
    <svg viewBox="0 0 256 144" class="w-full bg-[#20262a] touch-none" tabindex="0"
      aria-label="Point curve" :aria-disabled="disabled" data-curve-plot
      @pointerdown="start" @pointermove="move" @pointerup="finish" @pointercancel="cancel"
      @dblclick="remove" @keydown.escape="cancel">
      <path d="M64 0V144 M128 0V144 M192 0V144 M0 36H256 M0 72H256 M0 108H256" stroke="#40484c" fill="none" />
      <path d="M0 144L256 0" stroke="#70787c" stroke-dasharray="3 3" />
      <path :d="path" :stroke="colors[channel]" stroke-width="2" fill="none" />
      <circle v-for="(p,i) in draft[channel]" :key="i" :cx="p[0]*256" :cy="(1-p[1])*144" r="4"
        :data-curve-point="i" :fill="colors[channel]" stroke="#fff" stroke-width="1" />
    </svg>
    <p class="mt-1 text-[10px] text-base-content/60">Click to add · drag to adjust · double-click to remove</p>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { curveChannels, curveLut, identityCurves, type Curves, type CurveChannel } from '@/common/curves';
const props = defineProps<{ modelValue?: Curves; disabled?: boolean }>();
const emit = defineEmits<{ preview: [curves: Curves | null]; change: [curves: Curves, channel: CurveChannel] }>();
const labels = {rgb:'RGB',red:'Red',green:'Green',blue:'Blue'};
const colors = {rgb:'#eee',red:'#e63d3d',green:'#1b9a4b',blue:'#397be8'};
const channel = ref<CurveChannel>('rgb');
const clone = (curves?: Curves): Curves => JSON.parse(JSON.stringify({...identityCurves(), ...curves}));
const draft = ref(clone(props.modelValue));
let active = -1, snapshot = '', pointer: number | null = null;
watch(() => props.modelValue, value => { if (pointer===null) draft.value=clone(value); }, {deep:true});
const path = computed(() => curveLut(draft.value[channel.value]).map((y,x) => `${x?'L':'M'}${x/255*256},${(1-y/255)*144}`).join(''));
function coordinate(e: PointerEvent | MouseEvent) {
  const r=(e.currentTarget as SVGElement).getBoundingClientRect();
  return [Math.max(0,Math.min(1,(e.clientX-r.left)/r.width)),Math.max(0,Math.min(1,1-(e.clientY-r.top)/r.height))];
}
function nearest(x:number,y:number) {
  return draft.value[channel.value].findIndex(p => Math.hypot((p[0]-x)*256,(p[1]-y)*144)<10);
}
function start(e:PointerEvent) {
  if (props.disabled || e.button!==0) return;
  e.preventDefault(); (e.currentTarget as SVGElement).focus();
  const [x,y]=coordinate(e), points=draft.value[channel.value];
  snapshot=JSON.stringify(draft.value); active=nearest(x,y);
  if (active===0 || active===points.length-1) {active=-1;return;}
  if (active<0) {
    if (points.length>=16 || points.some(p=>Math.abs(p[0]-x)<1/255)) return;
    active=points.findIndex(p=>p[0]>x); points.splice(active,0,[x,y]);
  }
  pointer=e.pointerId; (e.currentTarget as SVGElement).setPointerCapture(pointer);
  emit('preview',clone(draft.value));
}
function move(e:PointerEvent) {
  if (pointer!==e.pointerId || active<1) return;
  const [x,y]=coordinate(e), points=draft.value[channel.value];
  points[active]=[Math.max(points[active-1][0]+1/255,Math.min(points[active+1][0]-1/255,x)),y];
  emit('preview',clone(draft.value));
}
function finish(e:PointerEvent) {
  if (pointer!==e.pointerId) return;
  (e.currentTarget as SVGElement).releasePointerCapture(pointer); pointer=null; active=-1;
  if (snapshot!==JSON.stringify(draft.value)) emit('change',clone(draft.value),channel.value);
  emit('preview',null);
}
function cancel() { if (pointer===null) return; draft.value=JSON.parse(snapshot);pointer=null;active=-1;emit('preview',null); }
function remove(e:MouseEvent) {
  if (props.disabled) return;
  const [x,y]=coordinate(e), i=nearest(x,y), points=draft.value[channel.value];
  if (i>0 && i<points.length-1) {points.splice(i,1);emit('change',clone(draft.value),channel.value);emit('preview',null);}
}
</script>
