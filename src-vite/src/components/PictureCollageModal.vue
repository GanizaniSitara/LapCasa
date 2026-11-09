<template>
  <ModalDialog title="Create Picture Collage" :width="420" position-key="picture-collage" @cancel="emit('cancel')">
    <div class="flex flex-col gap-3 text-sm" data-collage-dialog>
      <p class="text-xs text-base-content/70">{{ count.toLocaleString() }} photo{{ count === 1 ? '' : 's' }}{{ count > max ? ` (the first ${max} are used)` : '' }}</p>
      <label class="flex items-center justify-between gap-2">
        <span>Layout</span>
        <select v-model="layout" class="select select-sm w-48" aria-label="Collage layout">
          <option value="grid">Grid / Contact Sheet</option>
          <option value="mosaic">Mosaic</option>
        </select>
      </label>
      <label class="flex items-center justify-between gap-2">
        <span>Page size</span>
        <select v-model="size" class="select select-sm w-48" aria-label="Page size">
          <option v-for="s in sizes" :key="s" :value="s">{{ s.replace('x', ' x ') }}</option>
        </select>
      </label>
      <label class="flex items-center justify-between gap-2">
        <span>Spacing</span>
        <span class="flex items-center gap-2"><input v-model.number="spacing" type="range" min="0" max="32" class="range range-xs w-36" aria-label="Spacing" /><span class="w-10 text-right">{{ spacing }} px</span></span>
      </label>
      <div class="flex items-center justify-between gap-2">
        <span>Background</span>
        <span class="flex gap-1">
          <button v-for="c in colors" :key="c" type="button" class="w-7 h-7 rounded border"
            :class="bg === c ? 'border-primary border-2' : 'border-base-content/30'" :style="{ background: c }" :aria-label="`Background ${c}`" @click="bg = c" />
        </span>
      </div>
      <label class="flex items-center justify-between gap-2" :class="layout === 'mosaic' ? 'opacity-50' : ''">
        <span>Show captions / filenames</span>
        <input v-model="captions" type="checkbox" class="toggle toggle-primary toggle-sm" :disabled="layout === 'mosaic'" aria-label="Show captions" />
      </label>
      <p class="text-xs text-base-content/50">The collage is saved as a new JPEG in the first photo's folder and added to the library. Photos appear as LapCasa shows them, edits included.</p>
      <div class="flex justify-end gap-2">
        <button type="button" class="btn btn-sm btn-ghost" @click="emit('cancel')">Cancel</button>
        <button type="button" class="btn btn-sm btn-primary" data-collage-create @click="create">Create</button>
      </div>
    </div>
  </ModalDialog>
</template>

<script setup lang="ts">
// Picasa's Create Picture Collage (LAP-049): layout, page size, spacing, background and captions.
import { ref } from 'vue';
import ModalDialog from '@/components/ModalDialog.vue';

defineProps({
  count: { type: Number, default: 0 },
  max: { type: Number, default: 200 },
});
const emit = defineEmits(['cancel', 'create']);
const sizes = ['1920x1080', '2560x1440', '2400x1800', '2000x2000'];
const colors = ['#ffffff', '#18181b', '#000000'];
const layout = ref<'grid' | 'mosaic'>('grid');
const size = ref(sizes[0]);
const spacing = ref(8);
const bg = ref(colors[0]);
const captions = ref(true);
function create() {
  const [width, height] = size.value.split('x').map(Number);
  emit('create', { layout: layout.value, width, height, spacing: spacing.value, bgColor: bg.value, includeCaptions: layout.value === 'grid' && captions.value });
}
</script>
