<template>
  <div
    ref="containerRef"
    class="w-full h-full"
    :class="{ 
      'pointer-events-none': uiStore.inputStack.length > 0,
    }"
    @wheel="onWheel"
    @dragstart.capture.prevent
  >
    <VirtualScroll
      v-if="fileList.length > 0"
      ref="scroller"
      class="w-full h-full no-scrollbar"
      :class="{
        'pt-12': !isFilmstripView,
        'pb-8': !isFilmstripView && config.settings.showStatusBar,
        'pb-1': !isFilmstripView && !config.settings.showStatusBar,
      }"
      :items="renderItems"
      :direction="isFilmstripView && config.settings.grid.previewPosition < 2 ? 'horizontal' : 'vertical'"
      :grid-items="isFilmstripView ? 1 : columnCount"
      :item-size="isFilmstripView ? (config.settings.grid.previewPosition < 2 ? filmStripItemSize : itemHeight) : itemHeight"
      :item-secondary-size="!isFilmstripView ? itemWidth : (config.settings.grid.previewPosition >= 2 ? itemWidth : undefined)"
      :key="`${isFilmstripView}`"
      :geometry="virtualScrollGeometry"
      :indexed-grid="indexedGrid"
      :content-height="virtualScrollContentHeight"
      :transition="isLayoutTransitioning"
      key-field="id"
      :emit-update="true"
      :buffer="peopleView || albumView ? 0 : 4"
      v-slot="{ item, index }"
      @update="onUpdate"
      @scroll="onScroll"
    >
      <PersonSectionHeader
        v-if="isGroupRow(item) && item.personHeader"
        :person-id="Number(item.group_id)"
        :label="item.label"
        :count="Number(item.count || 0)"
      />
      <FolderSectionHeader
        v-else-if="isGroupRow(item) && item.folderHeader"
        :path="String(item.group_id)"
        :label="item.label"
        :count="Number(item.count || 0)"
      />
      <div
        v-else-if="isGroupRow(item)"
        class="w-full h-full flex items-center"
      >
        <div class="w-full h-8 px-1 flex items-center text-base-content/70 select-none bg-base-200/30 rounded-box">
          <div class="group flex items-center gap-1">
            <div
              class="flex shrink-0 items-center overflow-hidden transition-all duration-100 ease-out"
              :class="selectMode
                ? 'w-4 mr-1 opacity-100'
                : 'w-0 mr-0 opacity-0 group-hover:w-4 group-hover:mr-1 group-hover:opacity-100'"
            >
              <span v-if="isGroupSelectionLoading(item)" class="loading loading-spinner loading-xs text-primary"></span>
              <input
                v-else
                type="checkbox"
                class="checkbox checkbox-xs border-base-content/30 hover:border-base-content/70"
                :checked="getGroupSelectionState(item).checked"
                :indeterminate.prop="getGroupSelectionState(item).indeterminate"
                @change="(event) => $emit('group-select-toggled', item, (event.target as HTMLInputElement).checked)"
              />
            </div>
            <component :is="getGroupIcon(item)" class="w-4 h-4 shrink-0 text-base-content/30" />
            <Breadcrumb
              v-if="isFolderPathGroup()"
              :items="getFolderGroupBreadcrumbItems(item)"
              disabled
              class="min-w-0 flex-1 overflow-hidden"
            />
            <span v-else class="min-w-0 truncate text-sm text-base-content/30">{{ item.label }}</span>
          </div>
          <span class="ml-auto badge badge-xs shrink-0 text-xs text-base-content/30">
            {{ item.countLabel || Number(item.count || 0).toLocaleString() }}
          </span>
        </div>
      </div>
      <div
        v-else
        class="w-full h-full flex items-center justify-center overflow-hidden"
        :class="{ 'opacity-50': draggedFileIds.has(Number(getFileItem(item)?.id)) }"
        @pointerdown="onItemPointerDown($event, getFileIndex(item, index), getFileItem(item))"
      >
        <Thumbnail
          v-if="getFileItem(item) && !getFileItem(item).isPlaceholder"
          :key="getThumbnailKey(item, index)"
          :id="'item-' + getFileIndex(item, index)"
          :file="getFileItem(item)"
          :is-selected="selectMode ? Boolean(getFileItem(item).isSelected) : getFileIndex(item, index) === selectedItemIndex"
          :is-active="getFileIndex(item, index) === selectedItemIndex"
          :select-mode="selectMode"
          :query-source="querySource"
          :dedup-status="dedupStatuses[Number(getFileItem(item).id)]"
          :grid-size="isVerticalFilmstrip ? Math.max(1, itemWidth - 20) : gridSize"
          :filmstrip="isFilmstripView"
          :fill-card-width="gridColumns > 0 && !isFilmstripView"
          :suppress-placeholder="peopleView || albumView || isFilmstripView"
          @clicked="(modifiers) => $emit('item-clicked', getFileIndex(item, index), modifiers)"
          @dblclicked="(modifiers) => $emit('item-dblclicked', getFileIndex(item, index), modifiers)"
          @select-toggled="(shiftKey) => $emit('item-select-toggled', getFileIndex(item, index), shiftKey)"
          @action="(actionName) => $emit('item-action', { action: actionName, index: getFileIndex(item, index) })"
        />
        <div v-else class="w-full h-full" :class="!peopleView && !albumView ? 'bg-base-200/70 flex items-center justify-center text-base-content/10' : ''">
          <IconPhoto v-if="!peopleView && !albumView" class="w-8 h-8 opacity-20 animate-pulse" />
        </div>
      </div>
    </VirtualScroll>
    <!-- Empty State / Loading -->
    <div v-if="fileList.length === 0" class="absolute inset-0 flex flex-col items-center justify-center">
      <div class="text-base-content/30 flex flex-col items-center gap-2 text-center px-4">
        <Transition name="fade" appear>
          <div v-if="showDelayedLoading" class="flex flex-col items-center gap-2">
            <span class="loading loading-dots loading-lg text-primary"></span>
            <span>{{ $t('tooltip.loading') }}</span>
          </div>
        </Transition>
        <div v-if="contentReady" class="flex flex-col items-center gap-2">
          <IconSearch class="w-8 h-8" />
          <span class="text-sm">{{ emptyMessage }}</span>
          <span class="text-xs">{{ emptyHint }}</span>
        </div>
      </div>
    </div>

  </div>

</template>

<script setup lang="ts">

import { watch, ref, onMounted, onBeforeUnmount, computed, nextTick } from 'vue';
import { useI18n } from 'vue-i18n';
import { useUIStore } from '@/stores/uiStore';
import { config } from '@/common/config';
import { thumbnailProfile } from '@/common/thumbnailProfiles';
import { sectionGrid } from '@/common/sectionGrid';
import Thumbnail from '@/components/Thumbnail.vue';
import VirtualScroll from '@/components/VirtualScroll.vue';
import PersonSectionHeader from '@/components/PersonSectionHeader.vue';
import FolderSectionHeader from '@/components/FolderSectionHeader.vue';
import { GROUP } from '@/common/constants';
import { personFaces } from '@/common/personFaces';
import { calculateJustifiedLayout, calculateMasonryLayout, type Geometry } from '@/common/layout';
import { buildFolderBreadcrumbs, formatFolderBreadcrumb, isWithinRootPath, isEditableTarget } from '@/common/utils';
import Breadcrumb from '@/components/Breadcrumb.vue';
import {
  IconCalendarDay,
  IconCalendarMonth,
  IconCamera,
  IconCameraAperture,
  IconFile,
  IconFiles,
  IconFlag,
  IconFlagFilled,
  IconFlagOff,
  IconFolder,
  IconLocation,
  IconPhoto,
  IconSearch,
  IconStarFilled,
  IconVideo,
} from '@/common/icons';

const props = withDefaults(defineProps<{
  selectedItemIndex: number;
  fileList: any[];
  timelineData?: any[];
  sortType?: number;
  selectMode?: boolean;
  contentReady?: boolean;
  emptyMessage?: string;
  emptyHint?: string;
  layoutVersion?: number;
  sectionHeaders?: Array<{ index: number; personHeader?: boolean; folderHeader?: boolean }>;
  groupBy?: number;
  groupSelectedCounts?: Record<string, number>;
  groupSelectionLoading?: Record<string, boolean>;
  folderGroupRoots?: Array<{ path: string; name?: string }>;
  querySource?: string;
  peopleView?: boolean;
  albumView?: boolean;
  dedupStatuses?: Record<number, 'keep' | 'dup'>;
  draggedFileIds?: Set<number>;
  gridSize: number;
  gridColumns?: number;
}>(), {
  selectedItemIndex: -1,
  timelineData: () => [],
  sortType: 0,
  selectMode: false,
  contentReady: false,
  emptyMessage: '',
  emptyHint: '',
  layoutVersion: 0,
  sectionHeaders: () => [],
  groupBy: 0,
  groupSelectedCounts: () => ({}),
  groupSelectionLoading: () => ({}),
  folderGroupRoots: () => [],
  querySource: '',
  peopleView: false,
  albumView: false,
  dedupStatuses: () => ({}),
  draggedFileIds: () => new Set<number>(),
  gridSize: 120,
  gridColumns: 0,
});

const emit = defineEmits([
  'item-clicked',
  'item-dblclicked',
  'item-select-toggled',
  'item-action',
  'date-group-select',
  'group-select-toggled',
  'request-scroll',
  'visible-range-update',
  'scroll',
  'layout-update',
  'grid-size-change',
  'item-drag-start',
  'item-drag',
  'item-drag-end',
]);

const uiStore = useUIStore();
const { locale, messages } = useI18n();
const localeMsg = computed(() => messages.value[locale.value] as any);
const containerRef = ref<HTMLElement | null>(null);
const scroller = ref<any>(null);
const columnCount = ref(4);
const containerWidth = ref(0);
const GROUP_HEADER_HEIGHT = 40;
const groupHeaderHeight = computed(() => GROUP_HEADER_HEIGHT * Number(config.settings.scale || 1));
// People view: a taller section header with the person's 60px face.
const PERSON_HEADER_HEIGHT = 84;
// Folders view: folder name, date range, picture count and path.
const FOLDER_HEADER_HEIGHT = 56;
function headerHeightFor(item: any) {
  if (item?.personHeader) return PERSON_HEADER_HEIGHT;
  if (item?.folderHeader) return FOLDER_HEADER_HEIGHT;
  return groupHeaderHeight.value;
}
let pendingPointerDrag: {
  pointerId: number;
  index: number;
  startX: number;
  startY: number;
  hotspotXRatio: number;
  hotspotYRatio: number;
  active: boolean;
} | null = null;

function clearPointerDragListeners() {
  document.removeEventListener('pointermove', onDocumentPointerMove, true);
  document.removeEventListener('pointerup', onDocumentPointerUp, true);
  document.removeEventListener('pointercancel', onDocumentPointerUp, true);
}

function onItemPointerDown(event: PointerEvent, index: number, file: any) {
  if (index < 0) return;
  const target = event.target as HTMLElement;
  if (
    event.button !== 0
    || event.pointerType === 'touch'
    || target.closest('button, input, a, [role="button"]')
    || !file
    || file.isPlaceholder
  ) return;
  const itemElement = document.getElementById(`item-${index}`);
  const itemRect = itemElement?.getBoundingClientRect();
  pendingPointerDrag = {
    pointerId: event.pointerId,
    index,
    startX: event.clientX,
    startY: event.clientY,
    hotspotXRatio: itemRect?.width
      ? Math.max(0, Math.min(1, (event.clientX - itemRect.left) / itemRect.width))
      : 0.5,
    hotspotYRatio: itemRect?.height
      ? Math.max(0, Math.min(1, (event.clientY - itemRect.top) / itemRect.height))
      : 0.5,
    active: false,
  };
  document.addEventListener('pointermove', onDocumentPointerMove, true);
  document.addEventListener('pointerup', onDocumentPointerUp, true);
  document.addEventListener('pointercancel', onDocumentPointerUp, true);
}

function onDocumentPointerMove(event: PointerEvent) {
  const drag = pendingPointerDrag;
  if (!drag || event.pointerId !== drag.pointerId) return;
  if (!drag.active) {
    if (Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) < 6) return;
    drag.active = true;
    document.documentElement.style.userSelect = 'none';
    document.documentElement.style.webkitUserSelect = 'none';
    emit('item-drag-start', {
      event,
      index: drag.index,
      hotspotXRatio: drag.hotspotXRatio,
      hotspotYRatio: drag.hotspotYRatio,
    });
  }
  event.preventDefault();
  emit('item-drag', event);
}

function onDocumentPointerUp(event: PointerEvent) {
  const drag = pendingPointerDrag;
  if (!drag || event.pointerId !== drag.pointerId) return;
  if (drag.active) {
    event.preventDefault();
    event.stopPropagation();
    emit('item-drag-end', event);
  }
  pendingPointerDrag = null;
  document.documentElement.style.userSelect = '';
  document.documentElement.style.webkitUserSelect = '';
  clearPointerDragListeners();
}

function isGeometryGridStyle(style: number) {
  return style === 2 || style === 3;
}

const isFilmstripView = computed(() => config.settings.grid.viewMode === 'filmstrip');
const renderItems = computed(() => props.fileList);
const hasGroupRows = computed(() =>
  !isFilmstripView.value &&
  (Number(props.groupBy || 0) > 0 || isGroupRow(renderItems.value[0]))
);
const fileIndexToRowIndex = computed(() => {
  const map = new Map<number, number>();
  if (!hasGroupRows.value) return map;
  renderItems.value.forEach((item, rowIndex) => {
    if (!isGroupRow(item)) {
      const fileIndex = getFileIndex(item, rowIndex);
      if (fileIndex >= 0) map.set(fileIndex, rowIndex);
    }
  });
  return map;
});
const rowIndexToFileIndex = computed(() => {
  const map = new Map<number, number>();
  if (!hasGroupRows.value) return map;
  renderItems.value.forEach((item, rowIndex) => {
    if (!isGroupRow(item)) {
      const fileIndex = getFileIndex(item, rowIndex);
      if (fileIndex >= 0) map.set(rowIndex, fileIndex);
    }
  });
  return map;
});

const groupedLayoutGeometryResult = computed(() => {
  if (indexedGrid.value) return { boxes: [], contentSize: indexedGrid.value.contentSize };
  if (!hasGroupRows.value || renderItems.value.length === 0 || containerWidth.value <= 0) {
    return { boxes: [], contentSize: 0 };
  }

  const { style } = config.settings.grid;
  const size = props.gridSize;
  const boxes: Geometry[] = new Array(renderItems.value.length);

  if (!isGeometryGridStyle(style)) {
    let y = 0;
    let col = 0;
    let rowHeight = itemHeight.value;

    renderItems.value.forEach((item, rowIndex) => {
      if (isGroupRow(item)) {
        if (col > 0) {
          y += rowHeight;
          col = 0;
        }
        boxes[rowIndex] = { x: 0, y, width: containerWidth.value, height: headerHeightFor(item) };
        y += headerHeightFor(item);
        rowHeight = item.folderHeader ? photoItemHeight.value : itemHeight.value;
        return;
      }

      boxes[rowIndex] = {
        x: col * itemWidth.value,
        y,
        width: itemWidth.value,
        height: rowHeight,
      };
      col += 1;
      if (col >= columnCount.value) {
        y += rowHeight;
        col = 0;
      }
    });

    if (col > 0) y += rowHeight;
    return { boxes, contentSize: y };
  }

  let y = 0;
  let groupFiles: any[] = [];
  let groupRowIndices: number[] = [];

  const flushGroup = () => {
    if (groupFiles.length === 0) return;
    const result = style === 3
      ? calculateMasonryLayout(groupFiles, containerWidth.value, size, 0)
      : calculateJustifiedLayout(groupFiles, containerWidth.value, size, 0);
    result.boxes.forEach((box, index) => {
      boxes[groupRowIndices[index]] = {
        ...box,
        y: box.y + y,
      };
    });
    y += result.containerHeight;
    groupFiles = [];
    groupRowIndices = [];
  };

  renderItems.value.forEach((item, rowIndex) => {
    if (isGroupRow(item)) {
      flushGroup();
      boxes[rowIndex] = { x: 0, y, width: containerWidth.value, height: headerHeightFor(item) };
      y += headerHeightFor(item);
      return;
    }
    groupFiles.push(getFileItem(item));
    groupRowIndices.push(rowIndex);
  });
  flushGroup();

  return { boxes, contentSize: y };
});

const layoutGeometryResult = computed(() => {
  if (props.fileList.length === 0) {
    return { boxes: [], contentSize: 0 };
  }

  const { style } = config.settings.grid;
  const size = props.gridSize;

  if (hasGroupRows.value) {
    return groupedLayoutGeometryResult.value;
  }

  if (isFilmstripView.value) {
    return { boxes: [], contentSize: 0 };
  } else {
    if (style === 2 && containerWidth.value > 0) {
      const result = calculateJustifiedLayout(props.fileList, containerWidth.value, size, 0);
      return { boxes: result.boxes, contentSize: result.containerHeight };
    }
    else if (style === 3 && containerWidth.value > 0) {
      const result = calculateMasonryLayout(props.fileList, containerWidth.value, size, 0);
      return { boxes: result.boxes, contentSize: result.containerHeight };
    }
  }
  return { boxes: [], contentSize: 0 };
});

const layoutGeometry = computed(() => layoutGeometryResult.value.boxes);
const layoutContentHeight = computed(() => layoutGeometryResult.value.contentSize);
const indexedGrid = computed(() => {
  if (!hasGroupRows.value || isGeometryGridStyle(config.settings.grid.style) || containerWidth.value <= 0) return undefined;
  return sectionGrid(renderItems.value.length,
    props.sectionHeaders.map(header => ({ index: header.index, height: headerHeightFor(header),
      rowHeight: header.folderHeader ? photoItemHeight.value : itemHeight.value })),
    columnCount.value, containerWidth.value, itemWidth.value, itemHeight.value);
});
function boxAt(index: number) {
  return indexedGrid.value ? indexedGrid.value.boxAt(index) : layoutGeometry.value[index];
}
function displayIndexOf(index: number) {
  return indexedGrid.value ? indexedGrid.value.displayIndex(index) : (fileIndexToRowIndex.value.get(index) ?? index);
}
const usesGeometryLayout = computed(() =>
  !isFilmstripView.value && (hasGroupRows.value ||
  isGeometryGridStyle(config.settings.grid.style))
);
const virtualScrollGeometry = computed(() =>
  usesGeometryLayout.value && !indexedGrid.value ? layoutGeometry.value : undefined
);
const virtualScrollContentHeight = computed(() =>
  usesGeometryLayout.value ? layoutContentHeight.value : undefined
);
const isLayoutTransitioning = ref(false);
const startGridSize = ref(0);
let layoutTransitionTimer: ReturnType<typeof setTimeout> | null = null;
let layoutAnchorVersion = 0;
let isInitialLayout = true;

const gap = 8; // Gap between items
const isVerticalFilmstrip = computed(() => isFilmstripView.value && config.settings.grid.previewPosition >= 2);

// item width and height(including gap)
const itemWidth = computed(() => {
  const { style } = config.settings.grid;
  const size = props.gridSize;
  if (isVerticalFilmstrip.value && containerWidth.value > 0) {
    return containerWidth.value;
  }
  if (isFilmstripView.value) return size + 20;
  if (style === 0 && !isFilmstripView.value && props.gridColumns > 0 && containerWidth.value > 0) {
    return containerWidth.value / props.gridColumns;
  }
  if (style === 0) return size + 20; // size + padding(4*2) + border(2*2) + gap(8)
  return size;
});

const cardLabelHeight = computed(() => config.settings.grid.showLabels === false ? 0 :
  ((config.settings.grid.labelPrimary > 0 ? 20 : 0) + (config.settings.grid.labelSecondary > 0 ? 16 : 0)) * Number(config.settings.scale || 1));
const photoItemHeight = computed(() => props.gridSize * 3 / 4 + 20 + cardLabelHeight.value);

const itemHeight = computed(() => {
  const { style } = config.settings.grid;
  const size = props.gridSize;
  if (isFilmstripView.value) return itemWidth.value;
  
  if (style === 0) {
    let labelHeight = cardLabelHeight.value;
    if (props.peopleView || personFaces.controlsRow || personFaces.ignoredMode) labelHeight += 28 * Number(config.settings.scale || 1); // suggestion tick / x row
    
    const height = props.gridColumns > 0 && config.settings.grid.personFaces !== false
      && (props.peopleView || personFaces.controlsRow || personFaces.ignoredMode) ? size * 6 / 5 : size * 3 / 4;
    return height + 20 + labelHeight; // size + padding/border/gap(20) + labels
  }
  if (style === 1) return itemWidth.value + gap * 0.5;
  
  return size;
});

const filmStripItemSize = computed(() => {
  return itemWidth.value;
});

let resizeObserver: ResizeObserver | null = null;
const showDelayedLoading = ref(false);
let loadingDelayTimer: ReturnType<typeof setTimeout> | null = null;

function updateColumnCount() {
  if (containerRef.value) {
    containerWidth.value = containerRef.value.clientWidth;
    if (itemWidth.value > 0) {
      columnCount.value = config.settings.grid.style === 0 && !isFilmstripView.value && props.gridColumns > 0
        ? props.gridColumns : Math.max(1, Math.floor(containerWidth.value / itemWidth.value));
    }
  }
}

function updateLayout() {
  updateColumnCount();
  emit('layout-update', { height: layoutContentHeight.value });
}

watch(() => [props.gridSize, props.gridColumns, config.settings.grid.style, isFilmstripView.value], async () => {
  if (isInitialLayout) {
    isInitialLayout = false;
    updateColumnCount();
    return;
  }

  const anchorVersion = ++layoutAnchorVersion;
  isLayoutTransitioning.value = !isFilmstripView.value && !indexedGrid.value;
  updateColumnCount();

  await nextTick();
  if (anchorVersion !== layoutAnchorVersion) return;

  if (!indexedGrid.value) centerItem(props.selectedItemIndex);

  if (layoutTransitionTimer) clearTimeout(layoutTransitionTimer);
  layoutTransitionTimer = setTimeout(() => {
    layoutTransitionTimer = null;
    isLayoutTransitioning.value = false;
  }, 500);
});

watch(() => props.fileList, () => {
  updateLayout();
});

watch(() => props.timelineData, () => {
  updateLayout();
});

watch(() => props.layoutVersion, () => {
  updateLayout();
});

watch(
  () => props.contentReady,
  (ready) => {
    if (loadingDelayTimer) {
      clearTimeout(loadingDelayTimer);
      loadingDelayTimer = null;
    }

    if (ready) {
      showDelayedLoading.value = false;
      return;
    }

    showDelayedLoading.value = false;
    loadingDelayTimer = setTimeout(() => {
      loadingDelayTimer = null;
      if (!props.contentReady) {
        showDelayedLoading.value = true;
      }
    }, 500);
  },
  { immediate: true }
);

watch(layoutContentHeight, (newHeight) => {
  emit('layout-update', { height: newHeight });
});

watch(() => props.selectedItemIndex, (newValue) => {
  if (newValue !== -1) {
    scrollToItem(newValue);
  }
});

onMounted(() => {
  if (containerRef.value) {
    resizeObserver = new ResizeObserver(() => {
      // updateColumnCount(); // merged into updateLayout
      updateLayout();
      if (props.selectedItemIndex !== -1) {
        scrollToItem(props.selectedItemIndex);
      }
    });
    resizeObserver.observe(containerRef.value);
    updateLayout();

    // gesture events for macOS touchpad pinch
    containerRef.value.addEventListener('gesturestart', onGestureStart as any);
    containerRef.value.addEventListener('gesturechange', onGestureChange as any);
  }
  window.addEventListener('keydown', onKeyDown);
});

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeyDown);
  clearPointerDragListeners();
  pendingPointerDrag = null;
  document.documentElement.style.userSelect = '';
  document.documentElement.style.webkitUserSelect = '';
  if (loadingDelayTimer) {
    clearTimeout(loadingDelayTimer);
    loadingDelayTimer = null;
  }
  if (layoutTransitionTimer) {
    clearTimeout(layoutTransitionTimer);
    layoutTransitionTimer = null;
  }
  if (containerRef.value) {
    containerRef.value.removeEventListener('gesturestart', onGestureStart as any);
    containerRef.value.removeEventListener('gesturechange', onGestureChange as any);
  }
  if (resizeObserver) {
    resizeObserver.disconnect();
  }
});

function onGestureStart(e: any) {
  e.preventDefault();
  startGridSize.value = props.gridSize;
}

function onGestureChange(e: any) {
  e.preventDefault();
  if (startGridSize.value > 0) {
    let newSize = Math.round(startGridSize.value * e.scale);
    const profile = thumbnailProfile(config.settings.thumbnailSize);
    newSize = Math.max(profile.minGridSize, Math.min(profile.maxGridSize, newSize));
    emit('grid-size-change', newSize);
  }
}

function onUpdate(startIndex: number, endIndex: number) {
  emit('visible-range-update', { startIndex, endIndex });
}

function onScroll(e: Event) {
  emit('scroll', e);
}

function onWheel(e: WheelEvent) {
  if (isFilmstripView.value && scroller.value) {
    const isHorizontal = config.settings.grid.previewPosition < 2;
    if (isHorizontal) {
      // If it's a vertical scroll (deltaY) and no horizontal scroll (deltaX),
      // translate it to horizontal scroll
      if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
        scroller.value.$el.scrollLeft += e.deltaY;
        e.preventDefault(); // Prevent default vertical scrolling behavior if any
      }
    }
  }
}

function onKeyDown(e: KeyboardEvent) {
  if (isEditableTarget(e.target)) return;
  // Prevent default scrolling for arrow keys and spacebar
  if (['ArrowUp', 'ArrowDown', 'Space', ' '].includes(e.key)) {
    e.preventDefault();
  }
}

function scrollToItem(index: number, center = false) {
  if (!scroller.value) return;
  
  const el = scroller.value.$el;
  const displayIndex = displayIndexOf(index);

  const renderedItem = center ? containerRef.value?.querySelector(`#item-${index}`) : null;
  if (renderedItem && !virtualScrollGeometry.value && !indexedGrid.value) {
    renderedItem.scrollIntoView({
      behavior: 'auto',
      block: 'center',
      inline: 'center',
    });
    return;
  }
  
  if (!isFilmstripView.value) {
    let itemTop = 0;
    let itemBottom = 0;

    if ((virtualScrollGeometry.value || indexedGrid.value) && boxAt(displayIndex)) {
      const box = boxAt(displayIndex)!;
      itemTop = box.y;
      itemBottom = box.y + box.height;
    } else {
      // Normal Grid Logic
      const row = Math.floor(displayIndex / columnCount.value);
      itemTop = row * itemHeight.value;
      itemBottom = itemTop + itemHeight.value;
    }

    const scrollTop = el.scrollTop;
    const clientHeight = el.clientHeight;
    
    // Account for top and bottom padding
    const topPadding = 48 * Number(config.settings.scale || 1);
    const bottomPadding = config.settings.showStatusBar ? 32 : 4; // pb-8 = 32px, pb-1 = 4px

    if (center) {
      el.scrollTop = Math.min(
        Math.max(0, topPadding + (itemTop + itemBottom) / 2 - clientHeight / 2),
        Math.max(0, el.scrollHeight - clientHeight),
      );
      return;
    }
    
    const viewportTop = scrollTop;
    const viewportBottom = scrollTop + clientHeight - (topPadding + bottomPadding);

    // Only scroll if the item is not fully visible
    const isFullyVisible = itemTop >= viewportTop && itemBottom <= viewportBottom;
    
    if (!isFullyVisible) {
      if (itemTop < viewportTop) {
        // Item is above viewport, scroll to show it at the top
        el.scrollTop = itemTop;
      } else if (itemBottom > viewportBottom) {
        // Item is below viewport, scroll to show it at the bottom (accounting for bottom padding)
        el.scrollTop = itemBottom - clientHeight + (topPadding + bottomPadding);
      }
    }
  } else {
    // Filmstrip mode: center the item
    const isHorizontal = config.settings.grid.previewPosition < 2;
    let itemPos = 0;
    let itemSizeValue = 0;

    if (layoutGeometry.value[displayIndex]) {
      const box = layoutGeometry.value[displayIndex];
      itemPos = isHorizontal ? box.x : box.y;
      itemSizeValue = isHorizontal ? box.width : box.height;
    } else {
      const itemSizeConst = isHorizontal ? filmStripItemSize.value : itemHeight.value;
      itemPos = displayIndex * itemSizeConst;
      itemSizeValue = itemSizeConst;
    }

    const itemCenter = itemPos + itemSizeValue / 2;
    const clientSize = isHorizontal ? el.clientWidth : el.clientHeight;
    
    // Calculate target scroll to center the item
    let targetScroll = itemCenter - clientSize / 2;
    
    // Clamp to bounds
    targetScroll = Math.max(0, targetScroll);
    const maxScroll = (isHorizontal ? el.scrollWidth : el.scrollHeight) - clientSize;
    targetScroll = Math.min(targetScroll, maxScroll);
    
    el.scrollTo({
      [isHorizontal ? 'left' : 'top']: targetScroll,
      behavior: center ? 'auto' : 'smooth'
    });
  }
}

function scrollToPosition(scrollTop: number) {
  if (scroller.value && !isFilmstripView.value) {
    scroller.value.$el.scrollTop = scrollTop;
  }
}

function scrollToRowIndex(rowIndex: number) {
  const box = boxAt(rowIndex);
  if (box) {
    scrollToPosition(box.y);
    return;
  }
  if (!scroller.value || isFilmstripView.value) return;
  const nextScrollTop = Math.max(0, rowIndex) * itemHeight.value;
  scroller.value.$el.scrollTop = nextScrollTop;
}

// Top of a row in the grouped layout (e.g. a People section header), or -1.
function getRowTop(rowIndex: number) {
  const box = boxAt(rowIndex);
  return box ? box.y : -1;
}

function getColumnCount() {
  return columnCount.value;
}

function getItemRowIndices(fileIndex: number): number[] {
  if (fileIndex < 0) return [];
  const displayIndex = displayIndexOf(fileIndex);
  const box = boxAt(displayIndex);
  if (box && indexedGrid.value) {
    const range = indexedGrid.value.range(box.y + 0.5, box.y + 0.5);
    const indices: number[] = [];
    for (let i = range.start; i < range.end; i++) {
      if (Math.abs(boxAt(i)!.y - box.y) > 1) continue;
      const index = indexedGrid.value.fileIndex(i);
      if (index >= 0) indices.push(index);
    }
    return indices;
  }
  if (box) {
    const indices: number[] = [];
    layoutGeometry.value.forEach((candidate, index) => {
      if (!candidate || Math.abs(candidate.y - box.y) > 1 || isGroupRow(renderItems.value[index])) return;
      const itemIndex = hasGroupRows.value ? rowIndexToFileIndex.value.get(index) : index;
      if (itemIndex !== undefined && itemIndex >= 0) indices.push(itemIndex);
    });
    return indices;
  }
  const first = Math.floor(fileIndex / columnCount.value) * columnCount.value;
  return Array.from({ length: Math.min(columnCount.value, props.fileList.length - first) }, (_, i) => first + i);
}

function getScrollTop() {
  return scroller.value ? scroller.value.$el.scrollTop : 0;
}

function centerItem(index: number) {
  if (index >= 0) scrollToItem(index, true);
}

function getNextItemIndex(currentIndex: number, direction: 'up' | 'down', page = false): number {
  const pageHeight = Math.max(1, (scroller.value?.$el.clientHeight ?? 0)
    - 48 * Number(config.settings.scale || 1) - (config.settings.showStatusBar ? 32 : 4));
  const style = config.settings.grid.style;
  if (indexedGrid.value) {
    const grid = indexedGrid.value;
    const currentDisplayIndex = grid.displayIndex(currentIndex);
    const current = grid.boxAt(currentDisplayIndex);
    if (!current) return currentIndex;
    const sign = direction === 'down' ? 1 : -1;
    const targetY = current.y + sign * (page ? pageHeight : itemHeight.value);
    const range = grid.range(Math.max(0, targetY - itemHeight.value - PERSON_HEADER_HEIGHT), targetY + itemHeight.value + PERSON_HEADER_HEIGHT);
    let best = currentIndex, bestY = Infinity, bestX = Infinity;
    for (let i = range.start; i < range.end; i++) {
      const index = grid.fileIndex(i);
      if (index < 0) continue;
      const box = grid.boxAt(i)!;
      if (sign * (box.y - current.y) <= 1) continue;
      const distanceY = Math.abs(box.y - (page ? targetY : current.y));
      const distanceX = Math.abs(box.x + box.width / 2 - current.x - current.width / 2);
      if (distanceY < bestY || (distanceY === bestY && distanceX < bestX)) {
        best = index; bestY = distanceY; bestX = distanceX;
      }
    }
    return best;
  }
  const supportsGeometryNavigation = hasGroupRows.value
    || style === 2
    || (!isFilmstripView.value && isGeometryGridStyle(style));
  if (!supportsGeometryNavigation || layoutGeometry.value.length === 0) {
    if (page && !hasGroupRows.value) {
      const step = Math.max(1, Math.floor(pageHeight / itemHeight.value)) * columnCount.value;
      return Math.max(0, Math.min(props.fileList.length - 1, currentIndex + (direction === 'down' ? step : -step)));
    }
    return -1;
  }

  const currentDisplayIndex = fileIndexToRowIndex.value.get(currentIndex) ?? currentIndex;

  const currentBox = layoutGeometry.value[currentDisplayIndex];
  if (!currentBox) return currentIndex;

  const centerX = currentBox.x + currentBox.width / 2;
  const currentY = currentBox.y;
  
  // Find all items in the target direction
  let candidates: { index: number; box: Geometry; diffY: number }[] = [];

  layoutGeometry.value.forEach((box, displayIndex) => {
    if (direction === 'down') {
      if (box.y > currentY + 1) { // +1 for tolerance
         const fileIndex = hasGroupRows.value
           ? rowIndexToFileIndex.value.get(displayIndex)
           : displayIndex;
         if (fileIndex !== undefined) candidates.push({ index: fileIndex, box, diffY: box.y - currentY });
      }
    } else {
      if (box.y < currentY - 1) { // -1 for tolerance
         const fileIndex = hasGroupRows.value
           ? rowIndexToFileIndex.value.get(displayIndex)
           : displayIndex;
         if (fileIndex !== undefined) candidates.push({ index: fileIndex, box, diffY: currentY - box.y });
      }
    }
  });

  if (candidates.length === 0) return currentIndex;

  // Arrow keys move one row; page keys target one viewport away.
  const targetDistance = page ? pageHeight : 0;
  const minDiffY = Math.min(...candidates.map(c => Math.abs(c.diffY - targetDistance)));
  
  // Filter candidates to only those in the closest row
  const rowCandidates = candidates.filter(c => Math.abs(Math.abs(c.diffY - targetDistance) - minDiffY) < 5); // 5px tolerance

  // Find item with closest centerX
  let closestIndex = -1;
  let minDistX = Infinity;

  rowCandidates.forEach(c => {
    const boxCenterX = c.box.x + c.box.width / 2;
    const dist = Math.abs(boxCenterX - centerX);
    if (dist < minDistX) {
      minDistX = dist;
      closestIndex = c.index;
    }
  });

  return closestIndex !== -1 ? closestIndex : currentIndex;
}

function getFileItem(item: any) {
  return isGroupRow(item) ? null : (item?.type === 'item' ? item.file : item);
}

function getFileIndex(item: any, displayIndex: number) {
  if (isGroupRow(item)) return -1;
  return Number(item?.file_index ?? displayIndex);
}

function getThumbnailKey(item: any, displayIndex: number) {
  const file = getFileItem(item);
  // A learned face shifts later section indices without changing their photos.
  // Keep those thumbnail components mounted under their stable section/file key.
  if (file?.face_person_id) return `person-${file.face_person_id}:${file.id}`;
  return `${getFileIndex(item, displayIndex)}:${file?.id || ''}`;
}

function isGroupRow(item: any) {
  return item?.type === 'group';
}

function getGroupSelectionState(item: any) {
  const groupId = String(item?.group_id || '');
  const selectedCount = Number(props.groupSelectedCounts[groupId] || 0);
  const count = Number(item?.count || 0);
  return {
    checked: count > 0 && selectedCount >= count,
    indeterminate: selectedCount > 0 && selectedCount < count,
  };
}

function isGroupSelectionLoading(item: any) {
  return Boolean(props.groupSelectionLoading[String(item?.group_id || '')]);
}

function getGroupIcon(item: any) {
  if (item?.icon) return item.icon;
  switch (Number(props.groupBy || 0)) {
    case GROUP.FOLDER:
      return IconFolder;
    case GROUP.DAY:
      return IconCalendarDay;
    case GROUP.MONTH:
    case GROUP.YEAR:
      return IconCalendarMonth;
    case GROUP.RATING:
      return IconStarFilled;
    case GROUP.CULLING:
      switch (Number(item?.group_id ?? 0)) {
        case 1:
          return IconFlagFilled;
        case 2:
          return IconFlagOff;
        default:
          return IconFlag;
      }
    case GROUP.FILE_TYPE:
      switch (item?.group_id) {
        case 'image':
        case 'raw':
          return IconPhoto;
        case 'video':
          return IconVideo;
        default:
          return IconFiles;
      }
    case GROUP.LOCATION:
      return IconLocation;
    case GROUP.CAMERA:
      return IconCamera;
    case GROUP.LENS:
      return IconCameraAperture;
    default:
      return IconFiles;
  }
}

function isFolderPathGroup() {
  return Number(props.groupBy || 0) === GROUP.FOLDER;
}

function getGroupTitleSegments(item: any) {
  const label = getFolderGroupLabel(item);
  if (!label) return [];
  if (label.includes(' > ')) {
    return label.split(' > ').map(part => part.trim()).filter(Boolean);
  }
  const normalized = label.replace(/\\/g, '/');
  return normalized.split('/').map(part => part.trim()).filter(Boolean);
}

function getFolderGroupBreadcrumbItems(item: any) {
  const folderPath = String(item?.label || '').trim();
  if (!folderPath || folderPath === 'Unknown folder') return [];

  const root = [...props.folderGroupRoots]
    .filter(root => root.path && isWithinRootPath(folderPath, root.path))
    .sort((a, b) => b.path.length - a.path.length)[0];

  if (root) {
    return buildFolderBreadcrumbs(folderPath, root.path, root.name || '');
  }

  return getGroupTitleSegments(item).map((label, index, segments) => ({
    label,
    path: segments.slice(0, index + 1).join('/'),
  }));
}

function getFolderGroupLabel(item: any) {
  const folderPath = String(item?.label || '').trim();
  if (!folderPath || folderPath === 'Unknown folder') return folderPath;

  const root = [...props.folderGroupRoots]
    .filter(root => root.path && isWithinRootPath(folderPath, root.path))
    .sort((a, b) => b.path.length - a.path.length)[0];
  if (!root) return folderPath;

  return formatFolderBreadcrumb(folderPath, root.path, root.name || '');
}

defineExpose({
  getColumnCount,
  getItemRowIndices,
  scrollToItem,
  scrollToPosition,
  scrollToRowIndex,
  getRowTop,
  getScrollTop,
  centerItem,
  refreshLayout: updateLayout,
  getNextItemIndex
});

</script>

<style scoped>
</style>
