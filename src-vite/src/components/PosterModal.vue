<template>
  <ModalDialog title="Make a Poster" :width="420" position-key="make-poster" @cancel="emit('cancel')">
    <div class="flex flex-col gap-3 text-sm" data-poster-dialog>
      <p class="text-xs text-base-content/70 truncate">{{ name }}</p>
      <label class="flex items-center justify-between gap-2">
        <span>Poster size</span>
        <select v-model="grid" class="select select-sm w-48" aria-label="Poster size" data-poster-size>
          <option :value="2">2 x 2 sheets (400%)</option>
          <option :value="3">3 x 3 sheets (900%)</option>
          <option :value="4">4 x 4 sheets (1600%)</option>
        </select>
      </label>
      <label class="flex items-center justify-between gap-2">
        <span>Overlap sheets</span>
        <input v-model="overlap" type="checkbox" class="toggle toggle-primary toggle-sm" aria-label="Overlap sheets" data-poster-overlap />
      </label>
      <p class="text-xs text-base-content/50">The photo, edits included, is enlarged and cut into sheets to print and join. Overlapping sheets share a margin for trimming and gluing. The sheets are saved in a new Poster folder next to the photo.</p>
      <div class="flex justify-end gap-2">
        <button type="button" class="btn btn-sm btn-ghost" @click="emit('cancel')">Cancel</button>
        <button type="button" class="btn btn-sm btn-primary" data-poster-create @click="create">Make Poster</button>
      </div>
    </div>
  </ModalDialog>
</template>

<script setup lang="ts">
// Picasa's Tools > Make a Poster (LAP-049): poster size (sheets across and down) and overlap.
import { ref } from 'vue';
import ModalDialog from '@/components/ModalDialog.vue';

defineProps({
  name: { type: String, default: '' },
});
const emit = defineEmits(['cancel', 'create']);
// the overlap each sheet reaches into its neighbour, in saved pixels
const OVERLAP_PX = 80;
const grid = ref(2);
const overlap = ref(true);
function create() {
  emit('create', { cols: grid.value, rows: grid.value, overlapPx: overlap.value ? OVERLAP_PX : 0 });
}
</script>
