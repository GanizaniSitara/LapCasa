<template>
  <!-- Folders view: one header per folder, then that folder's pictures -->
  <div class="w-full h-full px-3 flex items-center border-t border-base-content/10 bg-base-200/40 select-none" :title="path">
    <div class="min-w-0 flex flex-col">
      <span class="truncate text-[1.35rem] leading-tight text-base-content/90" style="font-family: Georgia, 'Times New Roman', serif">{{ name }}</span>
      <div class="flex items-center gap-1 min-w-0 text-xs text-base-content/50 tabular-nums">
        <span class="truncate">
        <template v-if="dateLabel">{{ dateLabel }} &middot; </template>{{ count.toLocaleString() }} {{ count === 1 ? 'picture' : 'pictures' }}
        <span class="text-base-content/35"> &middot; {{ path }}</span>
        </span>
        <button type="button" class="shrink-0 p-0.5 hover:text-base-content rounded"
          title="Open in Explorer" aria-label="Open in Explorer"
          data-testid="folder-open-explorer" :data-path="path"
          @click.stop="revealPath(path)"><IconFolderExpanded class="w-3.5 h-3.5" /></button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue';
import { folderSectionOf } from '@/common/folderSections';
import { revealPath } from '@/common/api';
import { IconFolderWindows as IconFolderExpanded } from '@/common/icons';

const props = defineProps<{
  // Section id = folder path.
  path: string;
  label: string;
  count: number;
}>();

const section = computed(() => folderSectionOf(props.path));
const name = computed(() =>
  section.value?.name || props.label || props.path.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || props.path
);

const dateFormat = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'long', year: 'numeric' });
const formatDay = (seconds: number) => dateFormat.format(new Date(seconds * 1000));
const dateLabel = computed(() => {
  const min = section.value?.dateMin;
  const max = section.value?.dateMax;
  if (!min) return '';
  const first = formatDay(min);
  const last = max ? formatDay(max) : first;
  return first === last ? first : `${first} to ${last}`;
});
</script>
