<template>
  <section v-if="!prompt || visible" :class="prompt ? 'gpu-backdrop' : 'rounded-box p-2 space-y-2 border border-base-content/5'" data-gpu-status>
    <div :class="prompt ? 'gpu-dialog bg-base-300 text-base-content shadow-xl rounded-box p-5' : ''" :role="prompt ? 'dialog' : undefined" :aria-modal="prompt || undefined" aria-label="AI acceleration">
      <h2 class="font-bold mb-2">AI acceleration</h2>
      <template v-if="status">
        <p v-if="status.hardware.name">{{ status.hardware.name }} · {{ status.hardware.vram_mb }} MB · Driver {{ status.hardware.driver }}</p>
        <p data-gpu-summary>AI acceleration: {{ status.hardware.cuda ? 'on' : `off: ${status.hardware.reason}` }}</p>
        <p v-if="Object.keys(failedEngines).length" class="text-warning text-xs">Some AI tools are not accelerated. See AI engines for details.</p>
        <p v-if="prompt" class="my-3">Your graphics card can speed up AI tools. Download the GPU add-on (1,433 MB)? The first preparation can take a minute.</p>
        <div v-if="status.hardware.name && !busy" class="my-3">
          <label class="flex gap-2 items-start text-sm"><input v-model="terms" type="checkbox" class="checkbox checkbox-sm" data-gpu-terms />
            <span>I accept NVIDIA's <a href="https://docs.nvidia.com/cuda/eula/index.html" target="_blank" rel="noopener" class="link">CUDA</a> and <a href="https://developer.download.nvidia.com/compute/cudnn/redist/cudnn/LICENSE.txt" target="_blank" rel="noopener" class="link">cuDNN</a> terms. Components download directly from NVIDIA.</span>
          </label>
        </div>
        <div v-if="busy" class="my-3" aria-live="polite">
          <p>{{ phase }}</p>
          <progress class="progress progress-primary w-full" :value="phase === 'Downloading' ? percent : undefined" max="100"></progress>
        </div>
        <p v-if="error" class="text-error my-2 break-words" role="alert">{{ error }}</p>
        <div class="flex gap-2 my-2">
          <button v-if="status.hardware.name" class="btn btn-sm btn-primary" data-gpu-download :disabled="busy || !terms" @click="install">{{ prompt ? 'Download' : 'Download / repair GPU add-on' }}</button>
          <button v-if="prompt" class="btn btn-sm" data-gpu-later :disabled="busy" @click="dismiss(false)">Not now</button>
          <button v-if="prompt" class="btn btn-sm btn-ghost" data-gpu-never :disabled="busy" @click="dismiss(true)">Don't ask again</button>
        </div>
        <details v-if="!prompt" class="text-xs mt-2">
          <summary>AI engines</summary>
          <p v-for="(engine, name) in status.engines" :key="name">{{ name }}: {{ engine.provider }}<span v-if="engine.reason"> — {{ engine.reason }}</span></p>
        </details>
      </template>
      <p v-else>Checking graphics hardware...</p>
    </div>
  </section>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
const props = defineProps<{ prompt?: boolean }>();
type Status = { hardware: { name: string | null; vram_mb: number | null; driver: string | null; cuda: boolean; reason: string }; prompt: boolean; engines: Record<string, { provider: string; reason: string }> };
const status = ref<Status>();
const failedEngines = computed(() => Object.fromEntries(Object.entries(status.value?.engines || {}).filter(([, engine]) => ['cpu', 'unavailable', 'mixed'].includes(engine.provider))));
const visible = ref(false);
const terms = ref(false);
const busy = ref(false);
const phase = ref('Downloading');
const percent = ref(0);
const error = ref('');
const cleanup: UnlistenFn[] = [];
let timer: ReturnType<typeof setInterval>;
async function refresh() { status.value = await invoke<Status>('gpu_status'); }
async function dismiss(permanent: boolean) {
  if (permanent) await invoke('gpu_dismiss_prompt');
  sessionStorage.setItem('gpu-prompt-seen', '1');
  visible.value = false;
}
async function install() {
  busy.value = true; error.value = '';
  try {
    await invoke('gpu_install', { acceptNvidiaTerms: terms.value });
    await refresh(); visible.value = false;
  } catch (e) { error.value = String(e); await refresh().catch(() => {}); }
  finally { busy.value = false; }
}
onMounted(async () => {
  try {
    cleanup.push(await listen<{phase: string; bytes: number; total: number}>('gpu-install-progress', event => {
      phase.value = event.payload.phase;
      percent.value = Math.round(100 * event.payload.bytes / event.payload.total);
    }));
    cleanup.push(await listen('gpu-ready', () => { void refresh(); }));
    await refresh();
    visible.value = !!props.prompt && getCurrentWebviewWindow().label === 'main' && !!status.value?.prompt && !sessionStorage.getItem('gpu-prompt-seen');
    if (!props.prompt) timer = setInterval(() => { void refresh().catch(() => {}); }, 3000);
  } catch (e) { error.value = String(e); }
});
onUnmounted(() => { cleanup.forEach(fn => fn()); clearInterval(timer); });
</script>

<style scoped>
.gpu-backdrop { position: fixed; inset: 0; z-index: 100000; display: flex; align-items: center; justify-content: center; background: #0008; }
.gpu-dialog { width: min(520px, 90vw); font-size: 14px; }
</style>
