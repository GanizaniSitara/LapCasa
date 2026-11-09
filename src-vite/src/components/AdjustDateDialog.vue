<template>
  <ModalDialog title="Adjust Date and Time" :width="400" position-key="adjust-date" @cancel="emit('cancel')">
    <div class="flex flex-col gap-3 text-sm" data-adjust-date-dialog>
      <div>
        <div class="text-base-content/60 text-xs">Current date and time of {{ anchorName }}</div>
        <div class="font-medium" data-adjust-date-current>{{ currentText }}</div>
      </div>
      <label class="flex flex-col gap-1">
        <span class="text-base-content/60 text-xs">New date and time</span>
        <input v-model="value" type="datetime-local" step="1" class="input input-sm w-full" aria-label="New date and time" />
      </label>
      <div class="flex gap-1">
        <button v-for="step in steps" :key="step.label" type="button" class="btn btn-xs flex-1" @click="nudge(step.seconds)">{{ step.label }}</button>
      </div>
      <p v-if="count > 1" class="text-xs text-base-content/70" data-adjust-date-note>
        All {{ count.toLocaleString() }} selected photos will move by the same amount ({{ deltaText }}), so the time between them stays the same.
      </p>
      <p v-else class="text-xs text-base-content/70">{{ deltaText }}</p>
      <p class="text-xs text-base-content/50">LapCasa changes the date in its library only; the photo files are not modified.</p>
      <div class="flex justify-end gap-2">
        <button type="button" class="btn btn-sm btn-ghost" @click="emit('cancel')">Cancel</button>
        <button type="button" class="btn btn-sm btn-primary" :disabled="newSeconds === null || delta === 0" data-adjust-date-apply @click="apply">Apply</button>
      </div>
    </div>
  </ModalDialog>
</template>

<script setup lang="ts">
// Picasa's Tools > Adjust Date and Time (LAP-049): the anchor photo's date is set; the rest of the
// selection moves by the same amount.
import { computed, ref } from 'vue';
import ModalDialog from '@/components/ModalDialog.vue';

const props = defineProps({
  anchorName: { type: String, default: '' },
  anchorDate: { type: Number, required: true }, // Unix seconds, shown in local time as elsewhere in the app
  count: { type: Number, default: 1 },
});
const emit = defineEmits(['cancel', 'apply']);

const pad = (n: number) => String(n).padStart(2, '0');
const toInput = (seconds: number) => {
  const d = new Date(seconds * 1000);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
};
const value = ref(toInput(props.anchorDate));
const newSeconds = computed(() => {
  const ms = new Date(value.value).getTime();
  return Number.isFinite(ms) ? Math.round(ms / 1000) : null;
});
const delta = computed(() => (newSeconds.value === null ? 0 : newSeconds.value - props.anchorDate));
const currentText = computed(() => new Date(props.anchorDate * 1000).toLocaleString());
const deltaText = computed(() => {
  const d = delta.value;
  if (!d) return 'No change yet.';
  let rest = Math.abs(d);
  const parts: string[] = [];
  for (const [unit, size] of [['day', 86400], ['hour', 3600], ['minute', 60], ['second', 1]] as const) {
    const n = Math.floor(rest / size);
    rest -= n * size;
    if (n) parts.push(`${n} ${unit}${n === 1 ? '' : 's'}`);
  }
  return `${d > 0 ? 'Later' : 'Earlier'} by ${parts.join(' ')}`;
});
const steps = [
  { label: '-1d', seconds: -86400 }, { label: '-1h', seconds: -3600 },
  { label: '+1h', seconds: 3600 }, { label: '+1d', seconds: 86400 },
];
function nudge(seconds: number) {
  value.value = toInput((newSeconds.value ?? props.anchorDate) + seconds);
}
function apply() {
  if (newSeconds.value !== null && delta.value !== 0) emit('apply', newSeconds.value);
}
</script>
