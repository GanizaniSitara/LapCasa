<template>
  <section data-settings-section="search-models" class="rounded-sm p-3 space-y-2.5 bg-base-200/50 border border-base-content/15" aria-label="Model downloads">
    <h3 class="font-bold uppercase text-[10.5px] tracking-wider text-primary/80">Model downloads</h3>
    <p class="text-xs text-base-content/60">Download only the models you need. Review each provider's terms before downloading. Browsing and ordinary text search work without them.</p>
    <p v-if="loadError" role="alert" class="text-sm text-error">{{ loadError }}</p>
    <div v-for="pack in packs" :key="pack.id" class="flex items-center justify-between gap-3 text-sm" :data-model-pack="pack.id">
      <div class="min-w-0">
        <div>{{ displayLabel(pack) }}</div>
        <div class="text-xs text-base-content/60">{{ pack.installed ? (pack.bundled ? 'Available with this build' : 'Installed') : 'Not installed' }} · {{ size(pack.total_bytes) }}</div>
      </div>
      <button class="btn btn-sm shrink-0" :disabled="!!busy" @click="review(pack)">Review licence</button>
    </div>
    <p v-if="busy" class="text-sm" role="status">{{ phase }} · {{ size(received) }} / {{ size(total) }}</p>
    <progress v-if="busy" class="progress progress-primary w-full" :value="received" :max="total || 1"></progress>
    <button v-if="busy" class="btn btn-sm" :disabled="cancelling || phase === 'Installed' || phase === 'Finishing'" @click="cancel">{{ cancelling ? 'Cancelling…' : 'Cancel' }}</button>
    <p v-if="error" role="alert" class="text-sm text-error break-words">{{ error }}</p>
    <p v-if="success" role="status" class="text-sm">{{ success }}</p>

    <dialog ref="dialog" class="modal" aria-labelledby="model-licence-title" @keydown.stop @keydown.esc.prevent="decline" @cancel="decline" @close="clearReview">
      <div v-if="selected" class="modal-box max-w-xl space-y-3" role="document">
        <h3 id="model-licence-title" class="font-bold">{{ displayLabel(selected) }} models — {{ selected.licence }}</h3>
        <p class="text-sm">{{ selected.purpose }}</p>
        <p class="text-sm">Provider: {{ selected.provider }}<br>Download: {{ size(selected.total_bytes) }}</p>
        <p v-if="selected.restriction" class="text-sm font-semibold text-warning">{{ selected.restriction }}</p>
        <a :href="selected.licence_url" target="_blank" rel="noopener noreferrer" class="link text-sm">Read provider terms</a>
        <pre class="text-xs whitespace-pre-wrap max-h-52 overflow-y-auto border border-base-content/20 rounded p-2" tabindex="0" aria-label="Licence text">{{ selected.licence_text }}</pre>
        <p v-if="selected.unavailable" role="status" class="text-sm">{{ selected.unavailable }}</p>
        <p v-if="selected.installed" class="text-sm">These models are already available. Nothing needs to be downloaded.</p>
        <label v-else class="flex items-start gap-2 text-sm">
          <input v-model="accepted" type="checkbox" class="checkbox checkbox-sm" :disabled="!!selected.unavailable" data-model-terms>
          <span>I have read the terms and my intended use is permitted. Acceptance does not grant rights outside these terms.</span>
        </label>
        <div class="modal-action">
          <button class="btn btn-sm" @click="decline">{{ selected.installed ? 'Close' : 'Decline' }}</button>
          <button v-if="!selected.installed" class="btn btn-sm btn-primary" :disabled="!accepted || !!selected.unavailable" data-model-accept @click="download">Accept and download</button>
        </div>
      </div>
    </dialog>
  </section>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

type Pack = { id: string; label: string; purpose: string; provider: string; licence: string;
  licence_url: string; licence_text: string; restriction: string; unavailable: string;
  revision: string; total_bytes: number; installed: boolean; bundled: boolean; downloading: boolean };
const packs = ref<Pack[]>([]);
const dialog = ref<HTMLDialogElement>();
const selected = ref<Pack>();
const accepted = ref(false);
const busy = ref('');
let requestId = '';
const cancelling = ref(false);
const phase = ref('Downloading');
const received = ref(0);
const total = ref(0);
const loadError = ref('');
const error = ref('');
const success = ref('');
let disposed = false;
let unlisten: UnlistenFn | undefined;

function displayLabel(pack: Pack) {
  return ({ search: 'Find photos by description', ocr: 'OCR — text in photos', selection: 'Select subjects for editing' } as Record<string, string>)[pack.id] || pack.label;
}

function size(bytes: number) { return `${(bytes / 1_000_000).toFixed(1)} MB`; }
async function refresh() {
  const result = await invoke<Pack[]>('model_packs');
  if (!disposed) packs.value = result;
}
function review(pack: Pack) {
  accepted.value = false;
  selected.value = pack;
  dialog.value?.showModal();
}
function clearReview() { selected.value = undefined; accepted.value = false; }
function decline() { dialog.value?.close(); clearReview(); }
async function cancel() {
  if (!busy.value) return;
  cancelling.value = true;
  try {
    const accepted = await invoke<boolean>('model_cancel', { id: busy.value, requestId });
    if (!accepted) { cancelling.value = false; phase.value = 'Finishing'; }
  }
  catch (e) { error.value = String(e); cancelling.value = false; }
}
async function download() {
  const pack = selected.value;
  if (!pack || !accepted.value || pack.unavailable || busy.value || pack.installed) return;
  const revision = pack.revision;
  decline();
  busy.value = pack.id;
  requestId = crypto.randomUUID();
  cancelling.value = false;
  phase.value = 'Downloading';
  received.value = 0;
  total.value = pack.total_bytes;
  error.value = '';
  success.value = '';
  try {
    await invoke('model_download', { id: pack.id, requestId, accepted: true, revision });
    if (!disposed) success.value = `${displayLabel(pack)} models are ready.`;
  } catch (e) {
    if (!disposed && !String(e).includes('Model download cancelled')) error.value = String(e);
  } finally {
    if (!disposed) {
      busy.value = '';
      cancelling.value = false;
      await refresh().catch(e => { loadError.value = String(e); });
    }
  }
}
onMounted(async () => {
  try {
    const stop = await listen<{ id: string; request_id: string; phase: string; bytes: number; total: number }>('model-download-progress', event => {
      if (event.payload.id !== busy.value || event.payload.request_id !== requestId || disposed) return;
      phase.value = event.payload.phase;
      received.value = event.payload.bytes;
      total.value = event.payload.total;
    });
    if (disposed) { stop(); return; }
    unlisten = stop;
    await refresh();
  } catch (e) { if (!disposed) loadError.value = String(e); }
});
onUnmounted(() => {
  disposed = true;
  unlisten?.();
  if (busy.value) void invoke('model_cancel', { id: busy.value, requestId }).catch(() => {});
});
</script>
