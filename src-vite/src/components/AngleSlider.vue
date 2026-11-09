<template>
  <div
    ref="rootRef"
    class="angle-slider relative w-full select-none touch-none outline-none"
    :class="[
      disabled ? 'opacity-40 pointer-events-none' : 'cursor-ew-resize',
      dragging ? 'is-dragging' : '',
    ]"
    :style="{ height: HEIGHT + 'px' }"
    role="slider"
    tabindex="0"
    :aria-valuemin="min"
    :aria-valuemax="max"
    :aria-valuenow="Number(formattedValue)"
    :aria-valuetext="`${formattedValue}°`"
    :aria-label="ariaLabel"
    @pointerdown="onPointerDown"
    @dblclick="onDoubleClick"
    @keydown="onKeyDown"
  >
    <!-- baseline the ticks stand on -->
    <div
      class="absolute left-0 right-0 h-px bg-base-content/10"
      :style="{ bottom: BASELINE + 'px' }"
    ></div>

    <!-- tick marks -->
    <div
      v-for="t in ticks"
      :key="t.value"
      class="absolute w-px rounded-sm"
      :class="t.isZero ? 'bg-primary' : t.isMajor ? 'bg-base-content/45' : 'bg-base-content/20'"
      :style="{
        left: t.pct + '%',
        bottom: BASELINE + 'px',
        height: (t.isZero ? 13 : t.isMajor ? 9 : 5) + 'px',
      }"
    ></div>

    <!-- degree labels on major ticks -->
    <div
      v-for="t in majorTicks"
      :key="'lbl-' + t.value"
      class="absolute -translate-x-1/2 text-[9px] leading-none font-medium tabular-nums"
      :class="t.isZero ? 'text-primary' : 'text-base-content/40'"
      :style="{ left: t.pct + '%', bottom: '1px' }"
    >{{ t.value }}</div>

    <!-- thumb needle -->
    <div
      class="absolute z-10 w-[2px] -translate-x-1/2 rounded-full bg-primary"
      :class="dragging ? '' : ''"
      :style="{ left: thumbPct + '%', bottom: BASELINE + 'px', height: NEEDLE + 'px' }"
    ></div>

    <!-- live value badge (replaces the removed number input) -->
    <div
      class="absolute z-20 -translate-x-1/2 px-1.5 h-[15px] flex items-center rounded-md text-[10px] font-semibold tabular-nums leading-none border shadow-sm whitespace-nowrap"
      :class="
        dragging
          ? 'bg-primary text-primary-content border-primary'
          : 'bg-base-100 text-base-content/80 border-base-content/10'
      "
      :style="{ left: badgeLeftPx + 'px', bottom: BASELINE + NEEDLE - 2 + 'px' }"
    >{{ formattedValue }}°</div>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';

const props = withDefaults(
  defineProps<{
    modelValue: number;
    min?: number;
    max?: number;
    step?: number;
    /** interval (deg) between small ticks */
    minorStep?: number;
    /** interval (deg) between labeled ticks */
    majorStep?: number;
    /** |angle| within this many degrees snaps to 0 (center detent) */
    snapZero?: number;
    disabled?: boolean;
    ariaLabel?: string;
  }>(),
  {
    min: -20,
    max: 20,
    step: 0.1,
    minorStep: 1,
    majorStep: 5,
    snapZero: 0.05,
    disabled: false,
    ariaLabel: 'Angle',
  },
);

const emit = defineEmits<{
  (e: 'update:modelValue', value: number): void;
}>();

// Layout constants (px).
const HEIGHT = 40;
const BASELINE = 13; // ticks/needle stand here; labels live below
const NEEDLE = 15;
const BADGE_HALF = 22; // half badge width, for clamping inside the track

const formattedValue = computed(() => {
  return Number(props.modelValue || 0).toFixed(1);
});

const rootRef = ref<HTMLElement | null>(null);
const dragging = ref(false);
const width = ref(0);

let ro: ResizeObserver | null = null;
onMounted(() => {
  if (rootRef.value) {
    width.value = rootRef.value.getBoundingClientRect().width;
    ro = new ResizeObserver((entries) => {
      width.value = entries[0]?.contentRect.width ?? 0;
    });
    ro.observe(rootRef.value);
  }
});
onBeforeUnmount(() => {
  ro?.disconnect();
  ro = null;
  window.removeEventListener('pointermove', onPointerMove);
  window.removeEventListener('pointerup', onPointerUp);
});

interface Tick {
  value: number;
  pct: number;
  isMajor: boolean;
  isZero: boolean;
}

const ticks = computed<Tick[]>(() => {
  const span = props.max - props.min;
  const out: Tick[] = [];
  for (let v = props.min; v <= props.max + 1e-9; v += props.minorStep) {
    const value = Math.round(v);
    out.push({
      value,
      pct: ((v - props.min) / span) * 100,
      isMajor: Math.abs(v % props.majorStep) < 1e-6,
      isZero: Math.abs(v) < 1e-6,
    });
  }
  return out;
});

const majorTicks = computed(() => ticks.value.filter((t) => t.isMajor));

const thumbPct = computed(() => {
  const span = props.max - props.min;
  return ((props.modelValue - props.min) / span) * 100;
});

const badgeLeftPx = computed(() => {
  const px = (thumbPct.value / 100) * width.value;
  if (width.value <= 0) return px;
  return Math.min(width.value - BADGE_HALF, Math.max(BADGE_HALF, px));
});

const quantize = (raw: number): number => {
  const snap = props.snapZero !== undefined ? props.snapZero : (props.step / 2);
  const v = Math.abs(raw) <= snap ? 0 : Math.round(raw / props.step) * props.step;
  const clamped = Math.min(props.max, Math.max(props.min, v));
  const decimals = Math.max(0, Math.min(4, Math.round(-Math.log10(props.step))));
  return Number(clamped.toFixed(decimals));
};

let dragStartX = 0;
let dragStartVal = 0;

const setValue = (v: number) => {
  if (v !== props.modelValue) emit('update:modelValue', v);
};

const onPointerDown = (e: PointerEvent) => {
  if (props.disabled) return;
  e.preventDefault();
  dragging.value = true;
  rootRef.value?.setPointerCapture?.(e.pointerId);

  dragStartX = e.clientX;
  dragStartVal = props.modelValue;

  const el = rootRef.value;
  if (el) {
    const rect = el.getBoundingClientRect();
    const span = props.max - props.min;
    if (rect.width > 0 && span > 0) {
      const currentThumbX = rect.left + ((props.modelValue - props.min) / span) * rect.width;
      // If clicked far from thumb (> 16px), position to clicked track location first
      if (Math.abs(e.clientX - currentThumbX) > 16) {
        const pct = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
        const clickedVal = quantize(props.min + pct * span);
        setValue(clickedVal);
        dragStartVal = clickedVal;
      }
    }
  }

  window.addEventListener('pointermove', onPointerMove);
  window.addEventListener('pointerup', onPointerUp);
};

const onPointerMove = (e: PointerEvent) => {
  if (!dragging.value) return;
  const dx = e.clientX - dragStartX;
  // 1 pixel of mouse drag moves exactly 0.1 degrees (props.step).
  // Holding Shift makes it 1.0 deg per step (coarse).
  // This guarantees mouse dragging lands on every single 0.1 degree increment without skipping.
  const sensitivity = e.shiftKey ? props.step * 10 : props.step;
  setValue(quantize(dragStartVal + dx * sensitivity));
};

const onPointerUp = () => {
  dragging.value = false;
  window.removeEventListener('pointermove', onPointerMove);
  window.removeEventListener('pointerup', onPointerUp);
};

const onDoubleClick = () => {
  if (props.disabled) return;
  setValue(0);
};

const onKeyDown = (e: KeyboardEvent) => {
  if (props.disabled) return;
  const s = e.shiftKey ? props.step * 10 : props.step;
  let handled = true;
  switch (e.key) {
    case 'ArrowLeft':
    case 'ArrowDown':
      setValue(quantize(props.modelValue - s));
      break;
    case 'ArrowRight':
    case 'ArrowUp':
      setValue(quantize(props.modelValue + s));
      break;
    case 'Home':
      setValue(props.min);
      break;
    case 'End':
      setValue(props.max);
      break;
    case '0':
      setValue(0);
      break;
    default:
      handled = false;
  }
  if (handled) {
    e.preventDefault();
    e.stopPropagation();
  }
};
</script>

<style scoped>
.angle-slider:focus-visible {
  border-radius: 0.375rem;
  box-shadow: 0 0 0 2px color-mix(in oklab, var(--color-primary) 55%, transparent);
}
</style>
