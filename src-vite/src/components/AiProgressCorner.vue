<template>
  <!-- Corner: a tiny preview of the photo being processed, how many
       are left, and an X to stop background processing. -->
  <div
    v-if="visible"
    class="flex items-center gap-1.5 h-4 pl-0.5 pr-1 rounded bg-base-200 text-[11px] text-base-content/70 select-none"
    :title="progress?.path || ''"
  >
    <img
      v-if="previewSrc"
      :src="previewSrc"
      class="h-4 w-4 object-cover rounded-sm shrink-0"
      draggable="false"
    />
    <span class="whitespace-nowrap">{{ $t('statusbar.ai_progress', { count: left.toLocaleString() }) }}</span>
    <button
      type="button"
      class="btn btn-ghost btn-xs h-4 min-h-0 w-4 p-0 text-base-content/60 hover:text-base-content"
      :title="$t('statusbar.ai_progress_cancel')"
      @click.stop="cancel"
    >
      <IconClose class="w-3 h-3" />
    </button>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { config } from '@/common/config';
import { getAiProgress, setBackgroundProcessing } from '@/common/api';
import { getLapUrl } from '@/common/utils';
import { IconClose } from '@/common/icons';

type AiProgress = {
  enabled: boolean;
  file_id: number | null;
  path: string | null;
  objects_pending: number;
  ocr_pending: number;
};

const progress = ref<AiProgress | null>(null);
const left = computed(() => Math.max(progress.value?.objects_pending || 0, progress.value?.ocr_pending || 0));
const visible = computed(() => !!progress.value?.enabled && left.value > 0);
const previewSrc = computed(() => getLapUrl(progress.value?.file_id, false, config.settings.thumbnailSize));

let timer: ReturnType<typeof setInterval> | undefined;
async function poll() {
  progress.value = await getAiProgress();
}

async function cancel() {
  config.settings.processLibraryInBackground = false;
  await setBackgroundProcessing(false);
  await poll();
}

onMounted(() => {
  void poll();
  timer = setInterval(poll, 2000);
});
onBeforeUnmount(() => clearInterval(timer));
</script>
