<template>
  <details class="rounded-box p-2 bg-base-300/30 border border-base-content/5" @toggle="onToggle">
    <summary class="cursor-pointer text-sm font-medium py-1">Import people from Picasa</summary>
    <div class="space-y-3 pt-3 text-sm">
      <p class="text-base-content/60">Bring Picasa's saved names and face labels into this library. Photos and existing labels are preserved.</p>
      <p v-if="unavailable" role="alert" class="text-warning">{{ unavailable }}</p>
      <fieldset :disabled="busy || !!unavailable" class="space-y-3">
        <div>
          <label for="picasa-contacts" class="block mb-1">Picasa contacts file</label>
          <div class="flex gap-2">
            <input id="picasa-contacts" v-model="contacts" class="input input-bordered input-sm flex-1 min-w-0" placeholder="contacts.xml" @input="invalidate" />
            <button type="button" class="btn btn-sm" @click="chooseContacts">Choose…</button>
          </div>
        </div>
        <div v-for="(mapping, index) in mappings" :key="mapping.id" class="space-y-2 border-t border-base-content/10 pt-2">
          <label :for="`picasa-source-${index}`" class="block">Picasa photo folder</label>
          <div class="flex gap-2">
            <input :id="`picasa-source-${index}`" v-model="mapping.source" class="input input-bordered input-sm flex-1 min-w-0" @input="invalidate" />
            <button type="button" class="btn btn-sm" @click="chooseSource(index)">Choose…</button>
            <button v-if="mappings.length > 1" type="button" class="btn btn-sm btn-ghost" aria-label="Remove folder mapping" @click="removeMapping(index)">×</button>
          </div>
          <label :for="`picasa-target-${index}`" class="block text-base-content/60">Matching folder in this library</label>
          <select :id="`picasa-target-${index}`" v-model="mapping.target" class="select select-bordered select-sm w-full" @change="invalidate">
            <option v-for="root in targetRoots" :key="root" :value="root">{{ root }}</option>
          </select>
        </div>
        <button type="button" class="btn btn-sm btn-ghost" @click="addMapping">Add another Picasa folder</button>
        <p class="text-xs text-base-content/60">Choose the original folder, or its resized copy. Matching relative paths and image content are checked; overlapping folders are deduplicated.</p>
        <button type="button" class="btn btn-sm btn-primary" :disabled="!canPreview" @click="previewImport">Preview import</button>
      </fieldset>

      <p v-if="busy" role="status" class="flex gap-2 items-center">
        <span class="loading loading-spinner loading-xs" />
        {{ phase === 'preview' ? 'Checking photos and face labels…' : phase === 'face-thumbnails' ? 'Preparing face thumbnails…' : 'Backing up and importing people…' }}
        <span v-if="phase === 'face-thumbnails'">{{ preparedCrops.toLocaleString() }} ready</span>
        <span v-if="verifiedPairs">{{ verifiedPairs.toLocaleString() }} photos checked</span>
      </p>
      <p v-if="error" role="alert" class="text-error break-words">{{ error }}</p>

      <div v-if="preview && !result" class="space-y-2 border-t border-base-content/10 pt-3">
        <p><strong>{{ count('planned_people') }}</strong> new people · <strong>{{ count('planned_faces') }}</strong> face labels ready to import</p>
        <p>Ignored faces: {{ count('ignored_matched') }} matched · {{ count('ignored_unmatched') }} unmatched · {{ count('ignored_already_imported') }} already imported</p>
        <p v-if="count('already_imported')" class="text-base-content/60">{{ count('already_imported') }} labels are already imported.</p>
        <p v-if="count('duplicate_source_region')" class="text-base-content/60">{{ count('duplicate_source_region') }} duplicate source regions combined.</p>
        <details v-if="skipRows.length">
          <summary class="cursor-pointer">{{ skippedCount }} entries skipped for review</summary>
          <ul class="list-disc ml-5 pt-2 text-base-content/60">
            <li v-for="row in skipRows" :key="row.reason">{{ row.count }} · {{ skipLabel(row.reason) }}</li>
          </ul>
        </details>
        <p class="text-xs text-base-content/60">A database backup is saved before changes. Existing names, edits and recognition data are preserved.</p>
        <p v-if="backgroundEnabled" class="text-warning">Turn off “Process library in background” in General before importing.</p>
        <div class="flex gap-2">
          <button type="button" class="btn btn-sm btn-primary" :disabled="busy || backgroundEnabled || !count('planned_faces')" @click="confirmImport">Back up and import {{ count('planned_faces') }} labels</button>
          <button type="button" class="btn btn-sm btn-ghost" :disabled="busy" @click="openReport">Open report folder</button>
        </div>
      </div>
      <div v-if="result" role="status" class="space-y-2">
        <p>{{ result.applied_people }} people and {{ result.applied_faces }} face labels imported.</p>
        <p>Ignored faces: {{ result.applied_ignored || 0 }} imported · {{ result.ignored_unmatched || 0 }} unmatched</p>
        <p v-if="result.faceThumbnailError || result.faceThumbnails?.failed" class="text-warning">Some face thumbnails could not be prepared. Their imported labels are preserved.</p>
        <p class="text-xs text-base-content/60">Open People to see the imported names. This transfers saved labels; it does not recognize people in additional photos.</p>
        <button type="button" class="btn btn-sm btn-ghost" @click="openReport">Open backup and report folder</button>
      </div>
    </div>
  </details>
</template>

<script setup lang="ts">
import { computed, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ask, open } from '@tauri-apps/plugin-dialog';
import { config } from '@/common/config';

type Mapping = { id: string; source: string; target: string };
const contacts = ref('');
const mappings = ref<Mapping[]>([]);
const targetRoots = ref<string[]>([]);
const unavailable = ref('');
const error = ref('');
const busy = ref(false);
const phase = ref('');
const token = ref('');
const verifiedPairs = ref(0);
const preparedCrops = ref(0);
const preview = ref<any>(null);
const result = ref<any>(null);
let initialized = false;
let unlisten: (() => void) | undefined;
const backgroundEnabled = computed(() => config.settings.processLibraryInBackground);
const canPreview = computed(() => contacts.value.trim() && mappings.value.length && mappings.value.every(m => m.source.trim() && m.target));
const skipRows = computed(() => Object.entries(preview.value?.skipped || {}).map(([reason, count]) => ({ reason, count: Number(count) })));
const skippedCount = computed(() => skipRows.value.reduce((sum, row) => sum + row.count, 0));
const count = (name: string) => Number(preview.value?.counts?.[name] || 0);
const skipLabel = (reason: string) => ({
  unknown_contact: 'No matching Picasa contact name', unmapped_photo: 'Photo not found in this library',
  unverified_photo_or_region: 'Photo or face region could not be verified',
  conflicting_source_labels: 'Conflicting Picasa labels', existing_label_conflict: 'Existing label preserved',
  person_manually_changed: 'Person edited since a previous import', imported_face_changed: 'Face edited since a previous import',
} as Record<string,string>)[reason] || reason.replaceAll('_', ' ');
function invalidate() { preview.value = null; result.value = null; token.value = ''; error.value = ''; }
function addMapping() { invalidate(); mappings.value.push({ id: crypto.randomUUID(), source: '', target: targetRoots.value[0] || '' }); }
function removeMapping(index: number) { invalidate(); mappings.value.splice(index, 1); }
async function onToggle(event: Event) {
  if (!(event.target as HTMLDetailsElement).open || initialized) return;
  initialized = true;
  busy.value = true;
  try {
    const defaults: any = await invoke('picasa_import_defaults');
    contacts.value = defaults.contacts;
    targetRoots.value = defaults.targetRoots;
    unavailable.value = defaults.unavailable || '';
    addMapping();
    if (targetRoots.value.length === 1) mappings.value[0].source = targetRoots.value[0];
    if (!targetRoots.value.length) unavailable.value = 'Add a photo folder to this library first.';
    unlisten = await listen<any>('picasa-import-progress', event => {
      if (event.payload.token !== token.value) return;
      if (event.payload.progress.event === 'face-thumbnails') {
        phase.value = 'face-thumbnails';
        preparedCrops.value = event.payload.progress.stats.cached_after;
      } else verifiedPairs.value = event.payload.progress.verified_pairs || 0;
    });
  } catch (e) { error.value = String(e); initialized = false; }
  finally { busy.value = false; }
}
async function chooseContacts() {
  const selected = await open({ multiple: false, directory: false, filters: [{ name: 'Picasa contacts', extensions: ['xml'] }] });
  if (typeof selected === 'string') { invalidate(); contacts.value = selected; }
}
async function chooseSource(index: number) {
  const selected = await open({ multiple: false, directory: true });
  if (typeof selected === 'string') { invalidate(); mappings.value[index].source = selected; }
}
async function previewImport() {
  invalidate(); token.value = crypto.randomUUID(); verifiedPairs.value = 0; phase.value = 'preview'; busy.value = true;
  try {
    preview.value = await invoke('picasa_import_preview', { contacts: contacts.value.trim(), mappings: mappings.value.map(({ source, target }) => ({ source: source.trim(), target })), token: token.value });
  } catch (e) { error.value = String(e); }
  finally { busy.value = false; }
}
async function confirmImport() {
  const confirmed = await ask(`Import ${count('planned_faces')} face labels into the current library? A backup will be saved first.`, { title: 'Import Picasa people', kind: 'info', okLabel: 'Back up and import', cancelLabel: 'Cancel' });
  if (!confirmed) return;
  busy.value = true; phase.value = 'apply'; verifiedPairs.value = 0; error.value = '';
  try { result.value = await invoke('picasa_import_apply', { token: token.value,
    thumbnailSize: config.settings.thumbnailSize, tileAspect: config.settings.grid.style === 0 ? 5 / 6 : 1 }); }
  catch (e) { error.value = `${String(e)}. Preview again if the library or source files changed.`; }
  finally { busy.value = false; }
}
async function openReport() {
  try { await invoke('picasa_import_open_report', { token: token.value }); }
  catch (e) { error.value = String(e); }
}
onUnmounted(() => unlisten?.());
</script>
