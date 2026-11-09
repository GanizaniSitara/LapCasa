<template>
  <ModalDialog title="Export Picture to Folder" :width="460" position-key="export-to-folder" @cancel="emit('cancel')">
    <div class="flex flex-col gap-3 text-sm" data-export-dialog>
      <p class="text-xs text-base-content/70">{{ count.toLocaleString() }} photo{{ count === 1 ? '' : 's' }}</p>
      <div class="flex flex-col gap-1">
        <span>Export to</span>
        <span class="flex gap-2">
          <input v-model="folder" type="text" class="input input-sm flex-1 min-w-0" aria-label="Export folder" data-export-folder />
          <button type="button" class="btn btn-sm" @click="browse">Browse...</button>
        </span>
      </div>
      <label class="flex items-center justify-between gap-2">
        <span>Image size</span>
        <select v-model="size" class="select select-sm w-48" aria-label="Image size" data-export-size>
          <option :value="0">Use original size</option>
          <option v-for="s in sizes" :key="s" :value="s">Resize to {{ s }} px</option>
        </select>
      </label>
      <label class="flex items-center justify-between gap-2">
        <span>Image quality</span>
        <select v-model="quality" class="select select-sm w-48" aria-label="Image quality" data-export-quality>
          <option :value="85">Normal (85%)</option>
          <option :value="90">High (90%)</option>
          <option :value="98">Maximum (98%)</option>
        </select>
      </label>
      <p class="text-xs text-base-content/50">Photos are exported as LapCasa shows them, edits included, as JPEGs. Resizing sets the longest edge and never enlarges a smaller photo. Existing files are never overwritten.</p>
      <div class="flex justify-end gap-2">
        <button type="button" class="btn btn-sm btn-ghost" @click="emit('cancel')">Cancel</button>
        <button type="button" class="btn btn-sm btn-primary" data-export-run :disabled="!folder.trim()" @click="run">Export</button>
      </div>
    </div>
  </ModalDialog>
</template>

<script setup lang="ts">
// Picasa's File > Export Picture to Folder (LAP-049): destination folder, image size and quality.
import { ref } from 'vue';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import ModalDialog from '@/components/ModalDialog.vue';

const props = defineProps({
  count: { type: Number, default: 0 },
  defaultFolder: { type: String, default: '' },
});
const emit = defineEmits(['cancel', 'export']);
const sizes = [2560, 1920, 1600, 1280, 1024, 800, 640];
const folder = ref(props.defaultFolder);
const size = ref(0);
const quality = ref(85);
async function browse() {
  const picked = await openDialog({ directory: true, multiple: false, defaultPath: folder.value || undefined });
  if (typeof picked === 'string' && picked) folder.value = picked;
}
function run() {
  if (!folder.value.trim()) return;
  emit('export', { destFolder: folder.value.trim(), maxDimension: size.value || null, quality: quality.value });
}
</script>
