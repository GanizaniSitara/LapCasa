<template>
  <!-- Mask mode (LAP-098, LAP-103): sits exactly over the photo (same size and transform). The canvas
       shows the effect only where the mask is (the photo underneath shows through elsewhere), plus
       the red mask overlay when it is on; the svg draws the brush ring, the selection points and
       the gradient handles. The mask is a content selection (click, Subject, Sky, People) with
       brush touch-ups on top, or a painted / gradient mask on its own. -->
  <canvas ref="viewRef" :width="width" :height="height" data-effect-mask :data-mask-tool="mode"
    :style="{ ...layerStyle, cursor: mode === 'paint' ? 'none' : mode === 'select' ? 'crosshair' : 'default', touchAction: 'none' }"
    @pointerdown="onDown" @pointermove="onMove" @pointerup="onUp" @pointercancel="onUp"
    @pointerleave="hover = null; emit('select-leave')" @contextmenu.prevent></canvas>
  <svg :width="width" :height="height" :viewBox="`0 0 ${width} ${height}`"
    :style="{ ...layerStyle, pointerEvents: 'none', overflow: 'visible' }">
    <circle v-if="mode === 'paint' && hover" :cx="hover.x * width" :cy="hover.y * height" :r="radius"
      fill="none" stroke="white" :stroke-width="1.5 / scale" />
    <circle v-if="mode === 'paint' && hover" :cx="hover.x * width" :cy="hover.y * height" :r="radius + 1.5 / scale"
      fill="none" stroke="black" :stroke-width="1 / scale" />
    <template v-if="mode === 'select'">
      <circle v-for="(p, i) in points" :key="'p' + i" :data-select-point="p.label" :cx="p.x * width" :cy="p.y * height"
        :r="6 / scale" :fill="p.label ? '#22c55e' : '#ef4444'" stroke="white" :stroke-width="1.5 / scale" />
    </template>
    <template v-if="mode === 'gradient'">
      <line v-for="(l, i) in gradientLines" :key="i" :x1="l[0]" :y1="l[1]" :x2="l[2]" :y2="l[3]"
        stroke="white" :stroke-width="1.5 / scale" :stroke-dasharray="i ? `${6 / scale} ${4 / scale}` : undefined" />
      <circle v-for="(p, i) in handles" :key="'h' + i" :data-gradient-handle="i" :cx="p.x * width" :cy="p.y * height"
        :r="8 / scale" :fill="i ? 'black' : 'white'" stroke="white" :stroke-width="2 / scale"
        style="pointer-events: all; cursor: move" @pointerdown.stop="startHandle(i, $event)" />
    </template>
  </svg>
</template>

<script setup lang="ts">
import { ref, computed, watch, onMounted, type CSSProperties } from 'vue';

export type SelectPoint = { x: number; y: number; label: 0 | 1 };

const props = defineProps<{
  width: number;             // natural size of the photo on screen (the frame effects use)
  height: number;
  layerStyle: CSSProperties; // the photo's own size and transform
  effectSrc: string;         // the effect applied to the whole photo
  mode: 'paint' | 'gradient' | 'select';
  region: string;            // how the selection was made: '', 'subject', 'sky', 'people', 'background'
  points: SelectPoint[];     // click-to-select points (shown while selecting)
  brush: number;             // brush radius, fraction of the short side
  feather: number;           // softening, fraction of the short side
  inverted: boolean;
  showOverlay: boolean;
  outline?: boolean;         // draw the region's edge (LAP-107)
  hoverSrc?: string;         // the region under the pointer before anything is picked (LAP-107)
  scale: number;
  toImage: (clientX: number, clientY: number) => { x: number; y: number };
}>();
const emit = defineEmits<{
  (e: 'select-click', p: { x: number; y: number; add: boolean; subtract: boolean }): void;
  (e: 'select-hover', p: { x: number; y: number }): void;
  (e: 'select-leave'): void;
}>();

const viewRef = ref<HTMLCanvasElement | null>(null);
const hover = ref<{ x: number; y: number } | null>(null);
const handles = ref([{ x: 0.5, y: 0.3 }, { x: 0.5, y: 0.7 }]);
const radius = computed(() => Math.max(1, props.brush * Math.min(props.width, props.height)));

// The mask itself: white, its alpha is the effect strength. It is the content selection (if any)
// with the brush strokes replayed on top, so a new selection keeps the touch-ups.
const mask = document.createElement('canvas');
const selection = document.createElement('canvas');
const scratch = document.createElement('canvas');
const effect = new Image();
let effectReady = false;
let hasSelection = false;
type Stamp = { x: number; y: number; r: number; erase: boolean };
let strokes: Stamp[] = [];

function sizeCanvases() {
  for (const c of [mask, selection, scratch]) if (c.width !== props.width || c.height !== props.height) { c.width = props.width; c.height = props.height; }
}

function alphaSource(): HTMLCanvasElement {
  if (!props.inverted) return mask;
  const ctx = scratch.getContext('2d')!;
  ctx.globalCompositeOperation = 'source-over';
  ctx.filter = 'none';
  ctx.fillStyle = 'white';
  ctx.fillRect(0, 0, scratch.width, scratch.height);
  ctx.globalCompositeOperation = 'destination-out';
  ctx.drawImage(mask, 0, 0);
  ctx.globalCompositeOperation = 'source-over';
  return scratch;
}

let frame = 0;
function draw() {
  frame = 0;
  const view = viewRef.value;
  if (!view) return;
  const ctx = view.getContext('2d')!;
  const blur = props.feather * Math.min(props.width, props.height);
  const filter = blur >= 0.3 ? `blur(${blur}px)` : 'none';
  ctx.globalCompositeOperation = 'source-over';
  ctx.globalAlpha = 1;
  ctx.filter = 'none';
  ctx.clearRect(0, 0, view.width, view.height);
  const alpha = alphaSource();
  if (effectReady) {
    ctx.drawImage(effect, 0, 0, view.width, view.height);
    ctx.globalCompositeOperation = 'destination-in';
    ctx.filter = filter;
    ctx.drawImage(alpha, 0, 0);
    ctx.filter = 'none';
    ctx.globalCompositeOperation = 'source-over';
  }
  // LAP-107: the region's edge, outlined in blue as soon as it is chosen: the mask grown by a couple of screen pixels, minus the mask itself.
  if (props.outline && !props.showOverlay && (hasSelection || strokes.length || props.mode === 'gradient')) {
    const d = Math.max(1, 2 / props.scale);
    const ring = document.createElement('canvas');
    ring.width = view.width; ring.height = view.height;
    const r = ring.getContext('2d')!;
    for (const [dx, dy] of [[d, 0], [-d, 0], [0, d], [0, -d], [d, d], [-d, -d], [d, -d], [-d, d]]) r.drawImage(alpha, dx, dy);
    r.globalCompositeOperation = 'destination-out';
    r.drawImage(alpha, 0, 0);
    r.globalCompositeOperation = 'source-in';
    r.fillStyle = 'rgb(30,120,255)';
    r.fillRect(0, 0, ring.width, ring.height);
    ctx.drawImage(ring, 0, 0);
  }
  if (hoverReady && !hasSelection && !strokes.length && props.mode === 'select') {
    const t = document.createElement('canvas');
    t.width = view.width; t.height = view.height;
    const c = t.getContext('2d')!;
    c.drawImage(hoverMask, 0, 0, t.width, t.height);
    c.globalCompositeOperation = 'source-in';
    c.fillStyle = 'rgb(30,120,255)';
    c.fillRect(0, 0, t.width, t.height);
    ctx.globalAlpha = 0.3;
    ctx.drawImage(t, 0, 0);
    ctx.globalAlpha = 1;
  }
  if (props.showOverlay) {
    const tint = document.createElement('canvas');
    tint.width = view.width; tint.height = view.height;
    const t = tint.getContext('2d')!;
    t.fillStyle = 'rgb(255,0,0)';
    t.fillRect(0, 0, tint.width, tint.height);
    t.globalCompositeOperation = 'destination-in';
    t.filter = filter;
    t.drawImage(alpha, 0, 0);
    ctx.globalAlpha = 0.45;
    ctx.drawImage(tint, 0, 0);
    ctx.globalAlpha = 1;
  }
}
function redraw() { if (!frame) frame = requestAnimationFrame(draw); }

function stampOn(ctx: CanvasRenderingContext2D, s: Stamp) {
  const g = ctx.createRadialGradient(s.x, s.y, 0, s.x, s.y, s.r);
  g.addColorStop(0, 'rgba(255,255,255,1)');
  g.addColorStop(0.5, 'rgba(255,255,255,1)');
  g.addColorStop(1, 'rgba(255,255,255,0)');
  ctx.globalCompositeOperation = s.erase ? 'destination-out' : 'source-over';
  ctx.fillStyle = g;
  ctx.beginPath();
  ctx.arc(s.x, s.y, s.r, 0, Math.PI * 2);
  ctx.fill();
  ctx.globalCompositeOperation = 'source-over';
}
function stamp(x: number, y: number, erase: boolean) {
  const s = { x, y, r: radius.value, erase };
  strokes.push(s);
  stampOn(mask.getContext('2d')!, s);
}
// Selection first, then every brush touch-up again on top of it.
function rebuild() {
  const ctx = mask.getContext('2d')!;
  ctx.globalCompositeOperation = 'source-over';
  ctx.clearRect(0, 0, mask.width, mask.height);
  if (hasSelection) ctx.drawImage(selection, 0, 0, mask.width, mask.height);
  for (const s of strokes) stampOn(ctx, s);
  redraw();
}

let stroke: { x: number; y: number; erase: boolean; id: number } | null = null;
function pixel(event: PointerEvent) {
  const p = props.toImage(event.clientX, event.clientY);
  return { x: p.x * props.width, y: p.y * props.height, n: p };
}
function onDown(event: PointerEvent) {
  if (props.mode === 'select') {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    const p = props.toImage(event.clientX, event.clientY);
    if (p.x < 0 || p.y < 0 || p.x > 1 || p.y > 1) return;
    emit('select-click', { x: p.x, y: p.y, add: event.shiftKey, subtract: event.altKey });
    return;
  }
  if (props.mode !== 'paint' || (event.button !== 0 && event.button !== 2)) return;
  event.preventDefault();
  event.stopPropagation();
  const p = pixel(event);
  stroke = { x: p.x, y: p.y, erase: event.button === 2 || event.altKey, id: event.pointerId };
  viewRef.value?.setPointerCapture(event.pointerId);
  stamp(p.x, p.y, stroke.erase);
  redraw();
}
let lastHoverEmit = 0;
function onMove(event: PointerEvent) {
  const p = pixel(event);
  hover.value = p.n;
  // LAP-107: before anything is picked, the region under the pointer is shown (the editor asks
  // the selection model, at most every 120 ms).
  if (props.mode === 'select' && !hasSelection && !strokes.length && event.timeStamp - lastHoverEmit > 120
      && p.n.x >= 0 && p.n.y >= 0 && p.n.x <= 1 && p.n.y <= 1) {
    lastHoverEmit = event.timeStamp;
    emit('select-hover', { x: p.n.x, y: p.n.y });
  }
  if (!stroke || event.pointerId !== stroke.id) return;
  const step = Math.max(1, radius.value / 4);
  const d = Math.hypot(p.x - stroke.x, p.y - stroke.y);
  for (let i = 1; i <= Math.floor(d / step); i++) {
    const t = (i * step) / d;
    stamp(stroke.x + (p.x - stroke.x) * t, stroke.y + (p.y - stroke.y) * t, stroke.erase);
  }
  if (d >= step) { stroke.x = p.x; stroke.y = p.y; }
  redraw();
}
function onUp(event: PointerEvent) {
  if (stroke && event.pointerId === stroke.id) viewRef.value?.releasePointerCapture(event.pointerId);
  stroke = null;
}

// Linear gradient: full effect at the first (white) handle, none at the second (black) one. It
// replaces any selection and touch-ups.
function paintGradient() {
  hasSelection = false; strokes = [];
  const ctx = mask.getContext('2d')!;
  const [a, b] = handles.value;
  ctx.globalCompositeOperation = 'source-over';
  ctx.clearRect(0, 0, mask.width, mask.height);
  const g = ctx.createLinearGradient(a.x * props.width, a.y * props.height, b.x * props.width, b.y * props.height);
  g.addColorStop(0, 'rgba(255,255,255,1)');
  g.addColorStop(1, 'rgba(255,255,255,0)');
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, mask.width, mask.height);
  redraw();
}
const gradientLines = computed(() => {
  const [a, b] = handles.value.map(p => ({ x: p.x * props.width, y: p.y * props.height }));
  const dx = b.x - a.x, dy = b.y - a.y, len = Math.hypot(dx, dy) || 1;
  const px = -dy / len, py = dx / len, far = props.width + props.height;
  return [a, b].map(p => [p.x - px * far, p.y - py * far, p.x + px * far, p.y + py * far]);
});
function startHandle(index: number, event: PointerEvent) {
  event.preventDefault();
  const target = event.currentTarget as Element;
  target.setPointerCapture(event.pointerId);
  const move = (e: PointerEvent) => {
    const p = props.toImage(e.clientX, e.clientY);
    handles.value = handles.value.map((h, i) => i === index ? { x: p.x, y: p.y } : h);
    paintGradient();
  };
  const up = () => { target.removeEventListener('pointermove', move as EventListener); target.removeEventListener('pointerup', up); };
  target.addEventListener('pointermove', move as EventListener);
  target.addEventListener('pointerup', up);
}

watch(() => props.effectSrc, src => {
  effectReady = false;
  if (!src) { redraw(); return; }
  effect.onload = () => { effectReady = true; redraw(); };
  effect.src = src;
}, { immediate: true });
// The hovered region: grey PNG -> alpha, as setSelection does.
const hoverMask = document.createElement('canvas');
let hoverReady = false;
watch(() => props.hoverSrc, src => {
  if (!src) { hoverReady = false; redraw(); return; }
  const img = new Image();
  img.onload = () => {
    if (props.hoverSrc !== src) return;
    hoverMask.width = props.width; hoverMask.height = props.height;
    const c = hoverMask.getContext('2d', { willReadFrequently: true })!;
    c.clearRect(0, 0, hoverMask.width, hoverMask.height);
    c.drawImage(img, 0, 0, hoverMask.width, hoverMask.height);
    const data = c.getImageData(0, 0, hoverMask.width, hoverMask.height);
    for (let i = 0; i < data.data.length; i += 4) { data.data[i + 3] = data.data[i]; data.data[i] = data.data[i + 1] = data.data[i + 2] = 255; }
    c.putImageData(data, 0, 0);
    hoverReady = true;
    redraw();
  };
  img.src = src;
});
watch(() => props.mode, mode => { if (mode === 'gradient') paintGradient(); });
watch(() => [props.width, props.height], () => { sizeCanvases(); rebuild(); });
watch(() => [props.feather, props.inverted, props.showOverlay, props.outline, props.scale], redraw);
onMounted(() => { sizeCanvases(); if (props.mode === 'gradient') paintGradient(); redraw(); });

/** A content selection (grey PNG data URL, white = selected, any size) becomes the mask's base;
 *  brush touch-ups made so far stay on top. null clears the selection. */
function setSelection(dataUrl: string | null): Promise<void> {
  return new Promise(resolve => {
    const ctx = selection.getContext('2d')!;
    ctx.globalCompositeOperation = 'source-over';
    ctx.filter = 'none';
    ctx.clearRect(0, 0, selection.width, selection.height);
    if (!dataUrl) { hasSelection = false; rebuild(); resolve(); return; }
    const img = new Image();
    img.onload = () => {
      // Grey level -> alpha: draw the PNG, then keep its luminance as the alpha of white.
      ctx.drawImage(img, 0, 0, selection.width, selection.height);
      const data = ctx.getImageData(0, 0, selection.width, selection.height);
      const d = data.data;
      for (let i = 0; i < d.length; i += 4) { d[i + 3] = d[i]; d[i] = d[i + 1] = d[i + 2] = 255; }
      ctx.putImageData(data, 0, 0);
      hasSelection = true;
      rebuild();
      resolve();
    };
    img.onerror = () => { hasSelection = false; rebuild(); resolve(); };
    img.src = dataUrl;
  });
}
function clear() {
  hasSelection = false; strokes = [];
  selection.getContext('2d')!.clearRect(0, 0, selection.width, selection.height);
  mask.getContext('2d')!.clearRect(0, 0, mask.width, mask.height);
  redraw();
}
function isEmpty() {
  const data = mask.getContext('2d', { willReadFrequently: true })!.getImageData(0, 0, mask.width, mask.height).data;
  for (let i = 3; i < data.length; i += 4) if (data[i]) return false;
  return true;
}
// The mask as an opaque greyscale PNG (white = full effect), unfeathered and not inverted:
// feather and inversion travel as settings beside it.
function exportPng() {
  const out = document.createElement('canvas');
  out.width = mask.width; out.height = mask.height;
  const ctx = out.getContext('2d')!;
  ctx.fillStyle = 'black';
  ctx.fillRect(0, 0, out.width, out.height);
  ctx.drawImage(mask, 0, 0);
  return out.toDataURL('image/png');
}
function info() {
  if (props.mode === 'gradient') {
    return { kind: 'gradient', from: [handles.value[0].x, handles.value[0].y], to: [handles.value[1].x, handles.value[1].y] };
  }
  const kind = hasSelection ? (props.region || 'select') : 'paint';
  const out: Record<string, unknown> = { kind, touch_ups: strokes.length };
  if (hasSelection && !props.region) out.points = props.points.map(p => [p.x, p.y, p.label]);
  return out;
}
defineExpose({ clear, isEmpty, exportPng, info, setSelection });
</script>
