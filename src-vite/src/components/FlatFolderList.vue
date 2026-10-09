<template>
  <!-- Folders list: every folder that holds pictures, flat and always open,
       in disk order (top-level folder, then its subfolders). Rows are virtualised
       (fixed 24px height). In the stacked left pane (`scroller` set) the list does not
       scroll itself: it is virtualised against the shared scrolling column. -->
  <div
    ref="scrollRef"
    tabindex="0"
    data-album-list-root="true"
    :class="[scroller ? '' : 'min-h-0 flex-1 overflow-x-hidden overflow-y-auto', 'select-none outline-none']"
    @scroll="onScroll"
    @keydown="onKeyDown"
    @mousedown.capture="focusList"
    @drop.stop
  >
    <div :style="{ height: `${rows.length * ROW_HEIGHT}px`, position: 'relative' }">
      <div :style="{ transform: `translateY(${firstIndex * ROW_HEIGHT}px)` }">
        <template v-for="row in visibleRows" :key="row.key">
          <AlbumFolder
            :children="row.children"
            :albumId="row.folder.album_id"
            :rootPath="albumPaths[row.folder.album_id] || ''"
            :allowContextMenu="true"
            :treeRoot="false"
            :flat="true"
            :flatDepth="depthOf(row.folder)"
            @folder-favorite-changed="reload"
            @folder-path-changed="reload"
          />
        </template>
      </div>
    </div>
    <div v-if="folderSections.loaded && rows.length === 0" class="sidebar-empty text-sm">
      <span class="text-center">{{ filterText || favoritesOnly ? $t('album.no_folders_found') : $t('tooltip.not_found.albums') }}</span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue';
import { listen } from '@tauri-apps/api/event';
import { config, libConfig } from '@/common/config';
import { useUIStore } from '@/stores/uiStore';
import { listenIndexFinished } from '@/common/api';
import { folderSections, loadFolderSections } from '@/common/folderSections';
import { useAlbumSelection } from '@/composables/useAlbumSelection';
import { SIDEBAR } from '@/common/constants';
import { isEditableTarget } from '@/common/utils';
import AlbumFolder from '@/components/AlbumFolder.vue';

const props = defineProps<{
  albumPaths: Record<number, string>;
  filterText?: string;
  favoritesOnly?: boolean;
  scroller?: HTMLElement | null;  // stacked left pane: the shared scrolling column
}>();

const ROW_HEIGHT = 24;
const OVERSCAN = 12;

const uiStore = useUIStore();
const selection = useAlbumSelection();
const scrollRef = ref<HTMLElement | null>(null);
const scrollTop = ref(0);
const viewportHeight = ref(600);

type FlatFolder = {
  id: number; album_id: number; name: string; path: string; count: number;
  is_favorite: boolean; has_subfolders: boolean;
};
type Row = { type: 'folder'; key: string; folder: FlatFolder; children: FlatFolder[] };

// Stable folder objects per load: AlbumFolder edits them in place (rename, favorite).
const folders = computed<FlatFolder[]>(() => folderSections.list.map(section => reactive({
  id: section.folderId,
  album_id: section.albumId,
  name: section.name,
  path: section.path,
  count: section.count,
  is_favorite: section.isFavorite,
  has_subfolders: false,
})));
// Indent = how far below its top-level folder (album root) a folder sits on disk.
const segments = (path: string) => path.replace(/[\\/]+$/, '').split(/[\\/]+/).filter(Boolean).length;
const depthOf = (folder: FlatFolder) => {
  const root = props.albumPaths[folder.album_id] || '';
  return root ? Math.max(0, segments(folder.path) - segments(root)) : 0;
};

const rows = computed<Row[]>(() => {
  const query = (props.filterText || '').trim().toLocaleLowerCase();
  const result: Row[] = [];
  folderSections.list.forEach((_section, index) => {
    const folder = folders.value[index];
    if (props.favoritesOnly && !folder.is_favorite) return;
    if (query && !folder.name.toLocaleLowerCase().includes(query)
      && !(/[\\/]/.test(query) && folder.path.toLocaleLowerCase().includes(query))) return;
    result.push({ type: 'folder', key: `folder-${folder.path}`, folder, children: [folder] });
  });
  return result;
});

const firstIndex = computed(() => Math.max(0, Math.floor(scrollTop.value / ROW_HEIGHT) - OVERSCAN));
const visibleRows = computed(() => {
  const count = Math.ceil(viewportHeight.value / ROW_HEIGHT) + OVERSCAN * 2;
  return rows.value.slice(firstIndex.value, firstIndex.value + count);
});

function onScroll() {
  if (props.scroller) return;
  scrollTop.value = scrollRef.value?.scrollTop || 0;
}

// Stacked pane: the visible window is where the shared column's viewport meets this list.
function onSharedScroll() {
  const el = scrollRef.value;
  const scroller = props.scroller;
  if (!el || !scroller) return;
  scrollTop.value = Math.max(0, scroller.getBoundingClientRect().top - el.getBoundingClientRect().top);
  viewportHeight.value = scroller.clientHeight || viewportHeight.value;
}

const isSelectedPath = (path: string) => !selection.selected.value && selection.folderPath.value === path;

// Keep the highlighted folder in view (sidebar clicks and grid scrolling both move it).
function revealSelected() {
  const el = scrollRef.value;
  if (!el || selection.selected.value) return;
  const index = rows.value.findIndex(row => row.type === 'folder' && row.folder.path === selection.folderPath.value);
  if (index < 0) return;
  const scroller = props.scroller;
  if (scroller) {
    // Only while the Folders view is shown and this section is open.
    if (config.main.sidebarIndex !== SIDEBAR.ALBUM || libConfig.activePane === 'collection' || !el.offsetParent) return;
    const top = el.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop + index * ROW_HEIGHT;
    if (top < scroller.scrollTop + ROW_HEIGHT) {
      scroller.scrollTop = Math.max(0, top - ROW_HEIGHT);
    } else if (top + ROW_HEIGHT > scroller.scrollTop + scroller.clientHeight) {
      scroller.scrollTop = top + ROW_HEIGHT - scroller.clientHeight;
    }
    onSharedScroll();
    return;
  }
  const top = index * ROW_HEIGHT;
  if (top < el.scrollTop + ROW_HEIGHT) {
    el.scrollTop = Math.max(0, top - ROW_HEIGHT); // keep a row of context above it
  } else if (top + ROW_HEIGHT > el.scrollTop + el.clientHeight) {
    el.scrollTop = top + ROW_HEIGHT - el.clientHeight;
  }
  scrollTop.value = el.scrollTop;
}
watch(() => [selection.folderPath.value, selection.selected.value, rows.value.length], () => {
  void nextTick(revealSelected);
});

function focusList(event: MouseEvent) {
  if (event.target instanceof HTMLInputElement) return;
  uiStore.setActivePane('left-sidebar');
  scrollRef.value?.focus({ preventScroll: true });
}

function onKeyDown(event: KeyboardEvent) {
  if (isEditableTarget(event.target)) return;
  if (uiStore.inputStack.length > 0 || uiStore.activePane !== 'left-sidebar') return;
  if (document.activeElement !== scrollRef.value) return;
  const folderRows = rows.value.filter(row => row.type === 'folder') as Array<Extract<Row, { type: 'folder' }>>;
  if (folderRows.length === 0) return;
  const current = folderRows.findIndex(row => isSelectedPath(row.folder.path));
  let next = current;
  switch (event.key) {
    case 'ArrowUp': next = Math.max(0, current - 1); break;
    case 'ArrowDown': next = current < 0 ? 0 : Math.min(folderRows.length - 1, current + 1); break;
    case 'Home': next = 0; break;
    case 'End': next = folderRows.length - 1; break;
    default: return;
  }
  event.preventDefault();
  if (next === current) return;
  const folder = folderRows[next].folder;
  void selection.selectFolder(folder.album_id, folder as any);
}

function reload() {
  void loadFolderSections(true);
}

let resizeObserver: ResizeObserver | null = null;
let unlistenIndexFinished: (() => void) | undefined;
const unlisteners: Array<() => void> = [];

// Stacked pane: follow the shared column's scroll, its size, and sections above opening,
// closing or loading more rows (they move this list).
let observedScroller: HTMLElement | null = null;
function attachScroller(scroller: HTMLElement | null | undefined) {
  if (observedScroller) observedScroller.removeEventListener('scroll', onSharedScroll);
  observedScroller = scroller || null;
  if (!observedScroller) return;
  observedScroller.addEventListener('scroll', onSharedScroll, { passive: true });
  resizeObserver?.disconnect();
  resizeObserver = new ResizeObserver(() => onSharedScroll());
  resizeObserver.observe(observedScroller);
  if (observedScroller.firstElementChild) resizeObserver.observe(observedScroller.firstElementChild);
  onSharedScroll();
}
watch(() => props.scroller, scroller => attachScroller(scroller));

onMounted(async () => {
  if (props.scroller) {
    attachScroller(props.scroller);
  } else if (scrollRef.value) {
    viewportHeight.value = scrollRef.value.clientHeight || viewportHeight.value;
    resizeObserver = new ResizeObserver(() => {
      viewportHeight.value = scrollRef.value?.clientHeight || viewportHeight.value;
    });
    resizeObserver.observe(scrollRef.value);
  }
  await loadFolderSections();
  void nextTick(revealSelected);
  unlistenIndexFinished = await listenIndexFinished(() => reload());
  for (const name of ['albums-refreshed', 'album-roots-changed', 'library-total-refreshed']) {
    unlisteners.push(await listen(name, () => reload()));
  }
});

// Another library, or the toolbar's file-type filter: the list (and counts) change.
watch(() => [libConfig._libraryId, config.search.fileType], () => {
  void loadFolderSections();
});

defineExpose({ revealSelected });

onBeforeUnmount(() => {
  observedScroller?.removeEventListener('scroll', onSharedScroll);
  resizeObserver?.disconnect();
  unlistenIndexFinished?.();
  for (const unlisten of unlisteners) unlisten();
});
</script>
