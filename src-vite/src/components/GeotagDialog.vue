<template>
  <ModalDialog title="Place / Geotag" :width="400" position-key="geotag" @cancel="emit('cancel')">
    <div class="flex flex-col gap-3 text-sm" data-geotag-dialog>
      <label class="flex flex-col gap-1">
        <span class="text-base-content/60 text-xs">Coordinates (paste "latitude, longitude", e.g. from a map)</span>
        <input v-model="coords" class="input input-sm w-full" placeholder="49.1951, 16.6068" aria-label="Coordinates" />
      </label>
      <label class="flex flex-col gap-1">
        <span class="text-base-content/60 text-xs">Place name (optional)</span>
        <input v-model="name" class="input input-sm w-full" placeholder="Brno" aria-label="Place name" />
      </label>
      <p v-if="coords && !parsed" class="text-xs text-error">Latitude -90 to 90, longitude -180 to 180, separated by a comma.</p>
      <p class="text-xs text-base-content/70">
        {{ count > 1 ? `All ${count.toLocaleString()} selected photos will be placed here.` : 'The photo will be placed here.' }}
        LapCasa keeps the place in its library only; the photo files are not modified.
      </p>
      <div class="flex justify-end gap-2">
        <button type="button" class="btn btn-sm btn-ghost" @click="emit('cancel')">Cancel</button>
        <button type="button" class="btn btn-sm btn-primary" :disabled="!parsed" data-geotag-apply @click="apply">Apply</button>
      </div>
    </div>
  </ModalDialog>
</template>

<script setup lang="ts">
// Picasa's Geotag (LAP-049): coordinates and an optional place name for the selected photos.
import { computed, ref } from 'vue';
import ModalDialog from '@/components/ModalDialog.vue';

const props = defineProps({
  count: { type: Number, default: 1 },
  latitude: { type: Number, default: null },
  longitude: { type: Number, default: null },
  placeName: { type: String, default: '' },
});
const emit = defineEmits(['cancel', 'apply']);

const coords = ref(props.latitude != null && props.longitude != null ? `${props.latitude}, ${props.longitude}` : '');
const name = ref(props.placeName || '');
const parsed = computed(() => {
  const parts = coords.value.split(/[,;\s]+/).filter(Boolean).map(Number);
  if (parts.length !== 2 || parts.some((v) => !Number.isFinite(v))) return null;
  const [lat, lng] = parts;
  return Math.abs(lat) <= 90 && Math.abs(lng) <= 180 ? { lat, lng } : null;
});
function apply() {
  if (parsed.value) emit('apply', { latitude: parsed.value.lat, longitude: parsed.value.lng, name: name.value.trim() || null });
}
</script>
