<template>
  <div :class="containerClass">
    <div
      v-if="hasData"
      class="flex items-center justify-center flex-1 min-w-0 overflow-hidden whitespace-nowrap text-white"
      role="status"
      :title="statusText"
      data-photo-status
    >
      <div v-if="!hasRealSelectedFile" class="flex items-center gap-1.5 min-w-0">
        <span>{{ $t('statusbar.pictures', { count: totalFileCount.toLocaleString() }) }}</span>
        <template v-if="dateRangeText"><span>&middot;</span><span>{{ dateRangeText }}</span></template>
        <span>&middot;</span>
        <span>{{ $t('statusbar.on_disk', { size: formatFileSize(totalFileSize) }) }}</span>
      </div>

      <div v-if="hasRealSelectedFile" class="flex items-center gap-1.5 min-w-0">
        <span class="truncate">{{ currentFile.name }}</span>
        <template v-if="singleDateText"><span>&middot;</span><span>{{ singleDateText }}</span></template>
        <template v-if="currentFile.width && currentFile.height"><span>&middot;</span><span>{{ currentFile.width }} × {{ currentFile.height }}</span></template>
        <span>&middot;</span>
        <span>{{ $t('statusbar.on_disk', { size: formatFileSize(currentFile.size || 0) }) }}</span>
        <template v-if="showFilmStrip || showQuickView || showScale"><span>&middot;</span><span>{{ (imageScale * 100).toFixed(0) }}%</span></template>
      </div>
    </div>
    <div v-else-if="emptyMessage" class="flex items-center flex-1 min-w-0 text-white">
      <IconList class="t-icon-size-xs mr-1 shrink-0" />
      <span>{{ emptyMessage }}</span>
    </div>
    <div class="flex items-center gap-1.5 shrink-0 ml-auto pl-2">
      <AiProgressCorner v-if="!isEmbedded" />
      <div
        v-if="showUpdateIcon"
        class="px-1.5 py-0.5 flex items-center gap-1 rounded-full bg-white/15 text-white text-[11px] font-mono mr-1 cursor-pointer hover:bg-white/25"
        @click.stop="emit('toggle-scan')"
      >
        <component :is="updateIconComponent" class="t-icon-size-xs shrink-0" :class="{ 'animate-spin': isUpdateAnimating }" />
        <span v-if="scanText" class="truncate text-right max-w-[120px]">{{ scanText }}</span>
      </div>

      <button
        v-if="!isEmbedded"
        type="button"
        :class="[
          'btn btn-ghost btn-xs h-4 min-h-0 px-2 flex items-center gap-1.5 text-xs font-medium rounded transition-colors cursor-pointer select-none',
          isInfoOpen 
            ? 'bg-white/25 text-white hover:bg-white/30' 
            : 'text-white/90 hover:text-white hover:bg-white/20'
        ]"
        :title="isInfoOpen ? $t('toolbar.tooltip.hide_info') : $t('toolbar.tooltip.show_info')"
        @click.stop="emit('toggle-info')"
      >
        <IconInformation class="w-3.5 h-3.5" />
        <span class="text-[11px]">{{ $t('file_info.title') }}</span>
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue';
import {
  formatFileSize,
} from '@/common/utils';
import {
  IconList,
  IconUpdate,
  IconUpdateDot,
  IconInformation,
} from '@/common/icons';
import AiProgressCorner from '@/components/AiProgressCorner.vue';

const emit = defineEmits(['toggle-info', 'toggle-scan']);

const props = defineProps({
  fileList: {
    type: Array as () => any[],
    default: () => [],
  },
  selectedItemIndex: {
    type: Number,
    default: -1,
  },
  totalFileCount: {
    type: Number,
    default: 0,
  },
  totalFileSize: {
    type: Number,
    default: 0,
  },
  showFilmStrip: {
    type: Boolean,
    default: false,
  },
  showQuickView: {
    type: Boolean,
    default: false,
  },
  imageScale: {
    type: Number,
    default: 1,
  },
  selectedFile: {
    type: Object as () => any,
    default: null,
  },
  showScale: {
    type: Boolean,
    default: false,
  },
  isEmbedded: {
    type: Boolean,
    default: false,
  },
  isInfoOpen: {
    type: Boolean,
    default: false,
  },
  showUpdateIcon: {
    type: Boolean,
    default: false,
  },
  isUpdateAnimating: {
    type: Boolean,
    default: false,
  },
  updateIcon: {
    type: String as () => 'update' | 'dot',
    default: 'update',
  },
  scanText: {
    type: String,
    default: '',
  },
  emptyMessage: {
    type: String,
    default: '',
  },
  // First / last picture date of the view, in seconds (null when unknown).
  dateMin: {
    type: Number as () => number | null,
    default: null,
  },
  dateMax: {
    type: Number as () => number | null,
    default: null,
  },
});

const dayFormat = new Intl.DateTimeFormat(undefined, { day: '2-digit', month: 'long', year: 'numeric' });
const dateRangeText = computed(() => {
  if (!props.dateMin) return '';
  const first = dayFormat.format(new Date(props.dateMin * 1000));
  const last = dayFormat.format(new Date((props.dateMax || props.dateMin) * 1000));
  return first === last ? first : `${first} to ${last}`;
});

const updateIconComponent = computed(() =>
  props.updateIcon === 'dot' ? IconUpdateDot : IconUpdate
);

const hasData = computed(() => props.fileList.length > 0 || !!props.selectedFile);
const currentFile = computed(() => {
  if (props.selectedFile) return props.selectedFile;
  return props.selectedItemIndex >= 0 ? props.fileList[props.selectedItemIndex] : null;
});
const hasRealSelectedFile = computed(() => {
  const file = currentFile.value;
  return !!file && !file.isPlaceholder;
});
const singleDateText = computed(() => {
  const file = currentFile.value;
  const date = file?.taken_date || file?.modified_at || file?.created_at;
  return date ? dayFormat.format(new Date(date * 1000)) : '';
});
const statusText = computed(() => hasRealSelectedFile.value
  ? [currentFile.value.name, singleDateText.value,
      currentFile.value.width && currentFile.value.height ? `${currentFile.value.width} × ${currentFile.value.height}` : '',
      `${formatFileSize(currentFile.value.size || 0)} on disk`].filter(Boolean).join(' · ')
  : [`${props.totalFileCount.toLocaleString()} pictures`, dateRangeText.value,
      `${formatFileSize(props.totalFileSize)} on disk`].filter(Boolean).join(' · '));
const containerClass = computed(() => {
  const base = 'px-2 h-[18px] flex items-center justify-between text-xs font-medium cursor-default bg-[#568fb3] text-white';
  if (props.isEmbedded) return base;
  return `${base} absolute bottom-0 left-0 right-0 z-30`;
});
</script>
