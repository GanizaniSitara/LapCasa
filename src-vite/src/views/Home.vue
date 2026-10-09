<template>
  
  <div class="w-screen h-screen flex flex-col overflow-hidden select-none bg-base-300 text-base-content/70">
    <transition name="fade">
      <div
        v-if="isSwitchingLibrary"
        class="absolute inset-0 z-60 bg-base-300/60 backdrop-blur-sm flex items-center justify-center"
      >
        <span class="loading loading-spinner loading-lg text-primary"></span>
      </div>
    </transition>

    <!-- Title Bar -->
    <TitleBar v-if="showDesktopTitleBar" titlebar="LapCasa" viewName="Home" :icon="iconLogo"/>

    <!-- Main Content -->
    <div class="flex-1 flex relative overflow-hidden">
      <div class="w-full h-full flex" :class="inlineEditor ? 'invisible pointer-events-none' : ''">

      <!-- left pane -->
      <div
        v-if="!uiStore.isFullScreen"
        ref="leftPanelRootRef"
        tabindex="-1"
        :class="[
          'relative flex my-1 ml-1 z-10 select-none outline-none',
        ]"
        :style="{ width: leftPanelLayoutExpanded ? leftPanelWidth : '0px' }"
        data-tauri-drag-region
        @focus="uiStore.setActivePane('left-sidebar')"
      >
          <div
            class="absolute inset-y-0 left-0 bg-base-200 rounded-box"
            :class="isDraggingSplitter ? '' : 'transition-[width] duration-200 ease-in-out'"
            :style="{ width: leftPanelVisualExpanded ? leftPanelWidth : '0px' }"
          ></div>

          <!-- panel-->
          <div
            v-if="leftPanelMounted || libraryEmpty || databaseCorrupted"
            class="absolute inset-y-0 left-0 px-1 flex flex-col overflow-hidden transition-[transform,opacity] duration-200 ease-in-out"
            :class="leftPanelVisualExpanded ? 'translate-x-0 opacity-100' : '-translate-x-full opacity-0 pointer-events-none'"
            :style="{ width: leftPanelWidth }"
          >
            <nav aria-label="Views" data-view-shortcuts="sidebar"
              class="shrink-0 flex flex-wrap items-center gap-0.5 py-1 border-b border-base-content/10">
              <TButton v-for="item in visibleButtons" :key="item.index"
                buttonSize="small" :icon="item.icon" text=""
                :tooltip="(item as any).tooltip || item.text" tooltipPlacement="bottom"
                :selected="isRailSelected(item.index)" :disabled="item.disabled"
                @click="clickSidebar(item.index)" />
              <TButton :class="showDebugBadge ? 'text-warning' : ''" buttonSize="small"
                :icon="IconSettings" text="" :tooltip="$t('sidebar.settings')" tooltipPlacement="bottom"
                @click="clickSettings" />
            </nav>

            <!-- Component panel (flex-1 to fill remaining space) -->
            <div
              class="min-h-0 flex-1 overflow-hidden"
              :class="libConfig.activePane === 'collection' && !isStackedPane ? 'sidebar-pane-inactive' : ''"
              @mousedown.capture="!isStackedPane && activateMainPanel()"
            >
              <!-- Albums / People / Folders: one scrolling column of those three sections -->
              <LeftPaneStack v-if="databaseCorrupted === false && isStackedPane" ref="panelRef"
                :key="libraryVersion"
                @section-in-view="stackSectionInView = $event"
              />
              <component v-else-if="databaseCorrupted === false" ref="panelRef"
                :key="libraryVersion"
                :is="activeSidebarButton.component"
                :titlebar="activeSidebarButton.text"
                v-bind="activeSidebarButton.props || {}"
              />
            </div>
            <div
              v-if="databaseCorrupted === false && showBottomCollectionTray && config.collectionTray.expanded"
              class="h-1 -mx-1 shrink-0 cursor-row-resize splitter-indicator splitter-horizontal"
              :class="{ 'splitter-dragging': isDraggingCollectionSplitter }"
              @mousedown="startDraggingCollectionSplitter"
            ></div>
            <CollectionTray
              v-if="databaseCorrupted === false && showBottomCollectionTray"
              :class="[
                'overflow-hidden -mx-1',
                isDraggingCollectionSplitter ? '' : 'transition-[height] duration-200 ease-out',
                config.collectionTray.expanded ? '' : 'h-10',
              ]"
              :style="collectionTrayStyle"
              :expanded="config.collectionTray.expanded"
              @toggle-expanded="toggleCollectionTray"
            />
          </div>
        </div>
      
      <!-- splitter -->
      <div v-if="!uiStore.isFullScreen"
        class="w-1 shrink-0"
        :class="{
          'cursor-col-resize splitter-indicator splitter-vertical': leftPanelLayoutExpanded,
          'splitter-dragging': leftPanelLayoutExpanded && isDraggingSplitter,
        }" 
        @mousedown="startDraggingSplitter"
        @mouseup="stopDraggingSplitter"
      ></div>
       
      <!-- content area -->
      <div 
        :class="[
          'flex-1 flex relative',
          showDesktopTitleBar ? 'rounded-tl-box' : '',
        ]"
      >
        <div v-if="databaseCorrupted" class="flex-1 flex flex-col items-center justify-center gap-3 p-8 text-center" role="alert">
          <h2 class="text-lg font-medium">{{ $t('library.database_corrupted') }}</h2>
          <p class="text-sm text-base-content/60">{{ $t('library.database_corrupted_hint') }}</p>
        </div>
        <Content v-else-if="databaseCorrupted === false" ref="contentRef" :key="libraryVersion" :titlebar="activeSidebarButton.text" :libraryEmpty="libraryEmpty" @edit-requested="inlineEditor = $event"/>
      </div>
      </div>
      <ImageEditor v-if="inlineEditor" class="absolute inset-0 z-30" embedded
        :initial-file="inlineEditor.file" :folder-query="inlineEditor.folderQuery" @back="inlineEditor = null" />
    </div>

    <!-- logo -->
    <div class="fixed bottom-2 left-3 text-[12px] text-base-content/30">
      <span>{{ appName }}</span>
    </div>

    <!-- Manage Libraries Dialog -->

  </div>

</template>

<script setup lang="ts">
import { ref, computed, onBeforeUnmount, onMounted, watch, nextTick } from 'vue';
import { useI18n } from 'vue-i18n';
import { emit, listen } from '@tauri-apps/api/event';
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
import { getName } from '@tauri-apps/api/app';
import { invoke } from '@tauri-apps/api/core';
import { config, libConfig } from '@/common/config';
import { useUIStore } from '@/stores/uiStore';
import { isWin, isMac, isLinux, SCALE_VALUES, isEditableTarget } from '@/common/utils';
import { matchesShortcut, ShortcutPlatform } from '@/common/shortcuts';
import { SIDEBAR } from '@/common/constants';
import { settingsWindowGeometry } from '@/common/settingsWindow';
import { getAppConfig, switchLibrary, cancelIndexing, cancelFaceIndex } from '@/common/api';

// vue components
import Library from '@/components/Library.vue';
import AlbumList from '@/components/AlbumList.vue';
import SmartAlbumList from '@/components/SmartAlbumList.vue';
import ImageSearch from '@/components/ImageSearch.vue';
import Tag from '@/components/Tag.vue';
import Calendar from '@/components/Calendar.vue';
import Location from '@/components/Location.vue';
import Person from '@/components/Person.vue';
import Camera from '@/components/Camera.vue';
import LeftPaneStack from '@/components/LeftPaneStack.vue';

import TitleBar from '@/components/TitleBar.vue';
import TButton from '@/components/TButton.vue';
import Content from '@/components/Content.vue';
import ImageEditor from '@/views/ImageEditor.vue';
import CollectionTray from '@/components/CollectionTray.vue';
import iconLogo from '@/assets/images/icon.png';

import {
  IconTag,
  IconLocation,
  IconPerson,
  IconCamera,
  IconSearch,
  IconSettings,
  IconPhotoAll,
  IconCalendarDay,
  IconFolders,
  IconFolderCog
} from '@/common/icons';

const isSwitchingLibrary = ref(false);
const libraryVersion = ref(0);
const libraryEmpty = ref(false);
// Mount library components only after the startup corruption check has completed.
const databaseCorrupted = ref<boolean | null>(null);

const checkLibraryEmpty = async () => {
  // Guard the corruption probe so this function never rejects: it is called fire-and-forget from
  // onMounted / the albums-refreshed listener, and awaited before the switching lock is released
  // and before emit('library-switched'). An unhandled rejection there would stick the overlay or
  // skip the switch notification. On IPC failure, keep the previous (non-corrupt) behaviour.
  try {
    databaseCorrupted.value = await invoke<boolean>('is_database_corrupted');
  } catch {
    databaseCorrupted.value = false;
  }
  if (databaseCorrupted.value) {
    libraryEmpty.value = false;
    // Note: do NOT force showPanel here — it writes the persisted config.leftPanel.show and would
    // leave the panel expanded after switching back to a healthy library. The library menu is kept
    // reachable while corrupt via `v-if="leftPanelMounted || databaseCorrupted"` in the template.
    return;
  }
  try {
    const albums = await invoke<any[]>('get_all_albums', { refreshAccessibility: false });
    libraryEmpty.value = (albums?.length ?? 0) === 0;
    if (libraryEmpty.value) {
      config.main.sidebarIndex = SIDEBAR.ALBUM;
    }
  } catch {
    libraryEmpty.value = false;
  }
};

/// i18n
const { locale, messages } = useI18n();
const localeMsg = computed(() => messages.value[locale.value] as any);

const uiStore = useUIStore();

// Panel component ref
const panelRef = ref<any>(null);
const contentRef = ref<any>(null);
const inlineEditor = ref<any>(null);
const leftPanelRootRef = ref<HTMLElement | null>(null);
const showPanel = computed({
  get: () => config.leftPanel.show,
  set: (value: boolean) => { config.leftPanel.show = value; },
});
const LEFT_PANEL_ANIMATION_MS = 200;
const leftPanelMounted = ref(showPanel.value);
const leftPanelVisualExpanded = ref(showPanel.value);
const leftPanelLayoutExpanded = ref(showPanel.value);
let leftPanelAnimationTimer: ReturnType<typeof setTimeout> | null = null;
let leftPanelAnimationVersion = 0;

function clearLeftPanelAnimationTimer() {
  if (leftPanelAnimationTimer) {
    clearTimeout(leftPanelAnimationTimer);
    leftPanelAnimationTimer = null;
  }
}

async function commitLeftPanelLayout(expanded: boolean) {
  leftPanelLayoutExpanded.value = expanded;
  await nextTick();
  await contentRef.value?.refreshCenteredGridLayout?.();
}

watch(showPanel, async (expanded) => {
  clearLeftPanelAnimationTimer();
  const animationVersion = ++leftPanelAnimationVersion;

  if (expanded) {
    leftPanelMounted.value = true;
    await nextTick();
    if (animationVersion !== leftPanelAnimationVersion) return;
    leftPanelVisualExpanded.value = true;
    leftPanelAnimationTimer = setTimeout(() => {
      if (animationVersion !== leftPanelAnimationVersion) return;
      leftPanelAnimationTimer = null;
      void commitLeftPanelLayout(true);
    }, LEFT_PANEL_ANIMATION_MS);
    return;
  }

  leftPanelVisualExpanded.value = false;
  leftPanelAnimationTimer = setTimeout(() => {
    if (animationVersion !== leftPanelAnimationVersion) return;
    leftPanelAnimationTimer = null;
    void commitLeftPanelLayout(false).then(() => {
      if (animationVersion !== leftPanelAnimationVersion) return;
      leftPanelMounted.value = false;
    });
  }, LEFT_PANEL_ANIMATION_MS);
});

// Library state
interface Library {
  id: string;
  name: string;
  created_at: number;
  hidden: boolean;
}

interface AppConfig {
  current_library_id: string;
  libraries: Library[];
}

const appConfig = ref<AppConfig | null>(null);
const showDesktopTitleBar = isWin || isLinux;

/// Splitter for resizing the left pane
const isDraggingSplitter = ref(false);
const isDraggingCollectionSplitter = ref(false);

const appName = ref('');
const showDebugBadge = import.meta.env.DEV;
let unlistenOpenPreferences: (() => void) | null = null;
let unlistenOpenAbout: (() => void) | null = null;
let unlistenLibrariesConfirmed: (() => void) | null = null;
let unlistenLibrariesUpdated: (() => void) | null = null;
let unlistenAlbumsRefreshed: (() => void) | null = null;
let unlistenAddAlbumRequested: (() => void) | null = null;
let unlistenEditAlbumRequested: (() => void) | null = null;
let unlistenOpenFolderManager: (() => void) | null = null;
let unlistenFolderManagerScan: (() => void) | null = null;
const shortcutPlatform: ShortcutPlatform = isMac ? 'mac' : (isLinux ? 'linux' : 'windows');

// buttons
const buttons = computed(() =>  [
  { index: SIDEBAR.LIBRARY, icon: IconPhotoAll, component: Library, text: localeMsg.value.sidebar.library },
  { index: SIDEBAR.ALBUM, icon: IconFolders, component: AlbumList, text: localeMsg.value.sidebar.album, props: { selectionSource: 'album' } },
  { index: SIDEBAR.SMART_ALBUM, icon: IconFolderCog, component: SmartAlbumList, text: localeMsg.value.album.smart_album_list },
  { index: SIDEBAR.SEARCH, icon: IconSearch, component: ImageSearch, text: localeMsg.value.sidebar.search },
  { index: SIDEBAR.CALENDAR, icon: IconCalendarDay, component: Calendar, text: localeMsg.value.sidebar.calendar },
  { index: SIDEBAR.TAG, icon: IconTag, component: Tag, text: localeMsg.value.sidebar.tag },
  { index: SIDEBAR.PERSON, icon: IconPerson, component: Person, text: localeMsg.value.sidebar.people },
  { index: SIDEBAR.LOCATION, icon: IconLocation, component: Location, text: localeMsg.value.sidebar.location },
  { index: SIDEBAR.CAMERA, icon: IconCamera, component: Camera, text: localeMsg.value.sidebar.camera },
]);

const activeSidebarButton = computed(() =>
  buttons.value.find(item => item.index === config.main.sidebarIndex) || buttons.value[SIDEBAR.LIBRARY]
);

// Stacked left pane: the Albums (smart albums), People and Folders rail icons all show one
// stacked column of those sections. The other icons keep their panels.
const isStackSidebarIndex = (index: number) =>
  index === SIDEBAR.SMART_ALBUM || index === SIDEBAR.PERSON || index === SIDEBAR.ALBUM;
const isStackedPane = computed(() =>
  !libraryEmpty.value && isStackSidebarIndex(config.main.sidebarIndex)
);

const leftPanelWidth = computed(() =>
  `${(Number(config.leftPanel.width || 260) / 16).toFixed(2)}rem`
);

// Rail order follows the stacked pane: Albums, People, Folders, then the other panels.
const RAIL_ORDER: number[] = [
  SIDEBAR.LIBRARY, SIDEBAR.SMART_ALBUM, SIDEBAR.PERSON, SIDEBAR.ALBUM,
  SIDEBAR.SEARCH, SIDEBAR.TAG, SIDEBAR.CALENDAR, SIDEBAR.LOCATION, SIDEBAR.CAMERA,
];
const visibleButtons = computed(() =>
  buttons.value
    .map((item) => ({ ...item, disabled: libraryEmpty.value && item.index !== SIDEBAR.ALBUM }))
    .filter(item => !item.hidden)
    .sort((a, b) => RAIL_ORDER.indexOf(a.index) - RAIL_ORDER.indexOf(b.index))
);

// While the stacked pane is shown, its icons light up for the section scrolled into view.
const stackSectionInView = ref<number | null>(null);
const isRailSelected = (index: number) =>
  isStackedPane.value && stackSectionInView.value !== null && isStackSidebarIndex(index)
    ? stackSectionInView.value === index
    : config.main.sidebarIndex === index;


watch(() => config.libraryChangedVersion, async () => {
  appConfig.value = await getAppConfig();
});

onMounted(async () => {
  window.addEventListener('keydown', handleHomeKeyDown);
  unlistenOpenPreferences = await listen('app-open-preferences', () => {
    void clickSettings();
  });
  unlistenOpenAbout = await listen('app-open-about', () => {
    void clickSettings(7);
  });

  appConfig.value = await getAppConfig();
  unlistenLibrariesUpdated = await listen('libraries-settings-updated', onManageLibrariesUpdated);
  unlistenLibrariesConfirmed = await listen('libraries-settings-confirmed', async (event: any) => {
    if (event.payload?.backendSwitched) await onManageLibrariesOk(true);
    else if (event.payload?.id && event.payload.id !== appConfig.value?.current_library_id) await doSwitchLibrary(event.payload.id);
    else await onManageLibrariesUpdated();
  });

  void checkLibraryEmpty();

  // File > Add Folder: open the Folder Manager (whole-computer tree).
  unlistenAddAlbumRequested = await listen('add-album-requested', async () => {
    if (config.main.sidebarIndex !== SIDEBAR.ALBUM) config.main.sidebarIndex = SIDEBAR.ALBUM;
    void openFolderManagerWindow();
  });

  unlistenEditAlbumRequested = await listen('edit-album-requested', async (event: any) => {
    const albumId = Number(event.payload?.albumId || 0);
    if (albumId <= 0) return;
    if (config.main.sidebarIndex !== SIDEBAR.ALBUM) config.main.sidebarIndex = SIDEBAR.ALBUM;
    showPanel.value = true;
    await nextTick();
    (panelRef.value as any)?.openAlbumEdit?.(albumId);
  });

  // Open floating Folder Manager window
  unlistenOpenFolderManager = await listen('open-folder-manager', (event: any) => {
    const path = String(event.payload?.path || '');
    void openFolderManagerWindow(path);
  });

  // Folder Manager Scan Once / Scan Always: queue an explicit scan of the album(s).
  unlistenFolderManagerScan = await listen('folder-manager-scan-albums', (event: any) => {
    const ids = (event.payload?.albumIds || []).map((id: any) => Number(id)).filter((id: number) => id > 0);
    // Albums removed or merged into a new root leave the queue.
    const gone = new Set((event.payload?.removedAlbumIds || []).map((id: any) => Number(id)));
    if (gone.size > 0) {
      libConfig.index.albumQueue = (libConfig.index.albumQueue as any[]).filter(id => !gone.has(Number(id)));
      libConfig.index.pausedAlbumIds = (libConfig.index.pausedAlbumIds as any[]).filter(id => !gone.has(Number(id)));
      if (ids.length === 0) {
        libConfig.index.status = (libConfig.index.albumQueue as any[]).length > 0 ? 1
          : (libConfig.index.pausedAlbumIds as any[]).length > 0 ? 2 : 0;
      }
    }
    for (const albumId of ids) {
      libConfig.index.pausedAlbumIds = (libConfig.index.pausedAlbumIds as any[]).filter(id => Number(id) !== albumId);
      if (!(libConfig.index.albumQueue as any[]).some(id => Number(id) === albumId)) {
        libConfig.index.albumQueue.push(albumId);
      }
    }
    if (ids.length > 0) libConfig.index.status = 1;
    // Kick the queue even if its status did not change (a stuck or replaced head).
    // A duplicate request for an album that is already scanning is ignored by the backend.
    if ((libConfig.index.albumQueue as any[]).length > 0 && libConfig.index.status === 1) {
      void emit('trigger-next-album');
    }
  });

  unlistenAlbumsRefreshed = await listen('albums-refreshed', () => {
    void checkLibraryEmpty();
  });

  try {
    const name = await getName();
    if (name) appName.value = name;
  } catch (e) {
    console.error('Failed to get app name:', e);
  }
});

onBeforeUnmount(() => {
  clearLeftPanelAnimationTimer();
  window.removeEventListener('keydown', handleHomeKeyDown);
  document.removeEventListener('mousemove', handleCollectionMouseMove);
  document.removeEventListener('mouseup', stopDraggingCollectionSplitter);
  unlistenOpenPreferences?.();
  unlistenOpenPreferences = null;
  unlistenOpenAbout?.();
  unlistenOpenAbout = null;
  unlistenLibrariesConfirmed?.();
  unlistenLibrariesUpdated?.();
  unlistenAlbumsRefreshed?.();
  unlistenAlbumsRefreshed = null;
  unlistenAddAlbumRequested?.();
  unlistenAddAlbumRequested = null;
  unlistenOpenFolderManager?.();
  unlistenOpenFolderManager = null;
  unlistenFolderManagerScan?.();
  unlistenFolderManagerScan = null;
  unlistenEditAlbumRequested?.();
  unlistenEditAlbumRequested = null;
});

function handleHomeKeyDown(event: KeyboardEvent) {
  if (isEditableTarget(event.target)) return;

  if (event.key === 'Tab' && uiStore.inputStack.length === 0) {
    event.preventDefault();
    event.stopPropagation();
    if (uiStore.activePane === 'left-sidebar' || !leftPanelRootRef.value) {
      contentRef.value?.focusContent?.();
    } else {
      uiStore.setActivePane('left-sidebar');
      const albumListRoot = leftPanelRootRef.value.querySelector<HTMLElement>('[data-album-list-root="true"]');
      const folderTreeRoot = albumListRoot?.querySelector<HTMLElement>(
        '[data-selected-album-folder="true"] [data-folder-tree-root="true"]',
      );
      (folderTreeRoot || albumListRoot || leftPanelRootRef.value).focus({ preventScroll: true });
    }
    return;
  }

  if (matchesShortcut('app.search', event, shortcutPlatform)) {
    event.preventDefault();
    event.stopPropagation();
    if (!libraryEmpty.value && !databaseCorrupted.value) {
      if (config.main.sidebarIndex === SIDEBAR.SEARCH && showPanel.value) {
        nextTick(() => (panelRef.value as any)?.focusSearchInput?.());
      } else {
        config.main.sidebarIndex = SIDEBAR.SEARCH;
        showPanel.value = true;
      }
    }
    return;
  }

  if (!matchesShortcut('app.sidebar.toggle', event, shortcutPlatform)) {
    return;
  }

  event.preventDefault();
  event.stopPropagation();
  showPanel.value = !showPanel.value;
}

const doSwitchLibrary = async (libraryId: string) => {
  try {
    isSwitchingLibrary.value = true;
    databaseCorrupted.value = null;

    // Save current library state before switching (preserves the indexing queue)
    await libConfig.save();

    // Prevent auto-save during shutdown of the current library's background work.
    libConfig._initialized = false;

    // Cancel any running indexing before switching
    if (libConfig.index.status > 0 && libConfig.index.albumQueue.length > 0) {
      const queueCopy = [...libConfig.index.albumQueue];
      for (const albumId of queueCopy) {
        await cancelIndexing(albumId);
      }
    }
    
    // Cancel face indexing if running
    await cancelFaceIndex();
    
    await switchLibrary(libraryId);

    // Reload library state in-place (no page reload)
    await libConfig.reload();
    appConfig.value = await getAppConfig();
    libraryVersion.value++;
    // Settle the corruption/empty state before notifying, so `library-switched` listeners see a
    // resolved databaseCorrupted (matches onManageLibrariesOk's ordering).
    await checkLibraryEmpty();
    await emit('library-switched');
  } catch (error) {
    libConfig._initialized = true;
    console.error('Failed to switch library:', error);
    // Still settle state after a failed switch so the UI reflects the current library.
    await checkLibraryEmpty();
  } finally {
    isSwitchingLibrary.value = false;
  }
};

const onManageLibrariesOk = async (forceReload = false) => {
  const oldLibId = appConfig.value?.current_library_id;
  appConfig.value = await getAppConfig();

  if (forceReload || (oldLibId && appConfig.value?.current_library_id !== oldLibId)) {
    isSwitchingLibrary.value = true;
    databaseCorrupted.value = null;
    try {
      // The backend has already switched; reload in-place.
      await libConfig.reload();
      libraryVersion.value++;
      await checkLibraryEmpty();
      await emit('library-switched');
    } finally {
      isSwitchingLibrary.value = false;
    }
  }
};

const onManageLibrariesUpdated = async () => {
  appConfig.value = await getAppConfig();
};

// click sidebar
function clickSidebar(index: number) {
  // While the library is corrupt the panel <component> is unmounted; ignore navigation so we don't
  // silently mutate the persisted sidebarIndex and dead-end in a blank pane.
  if (databaseCorrupted.value) return;
  const wasMainPane = libConfig.activePane !== 'collection';
  activateMainPanel();
  if (libraryEmpty.value && index !== SIDEBAR.ALBUM) return;
  const stacked = !libraryEmpty.value && isStackSidebarIndex(index);
  if (config.main.sidebarIndex === index) {
    // Stacked pane: the icon first brings its section into view (or comes back to it from
    // an album); once it is there, the icon hides / shows the pane as before.
    if (stacked && showPanel.value) {
      if (!wasMainPane) {
        panelRef.value?.revealSection?.(index, true);
        return;
      }
      if (panelRef.value?.revealSection?.(index)) return;
    }
    showPanel.value = !showPanel.value;
  } else {
    showPanel.value = true;
    config.main.sidebarIndex = index;
  }
  if (stacked && showPanel.value) {
    void nextTick(() => panelRef.value?.revealSection?.(index, true));
  }
}

function activateMainPanel() {
  libConfig.activePane = 'main';
}

// Dragging the splitter
function startDraggingSplitter(event: MouseEvent) {
  if(!leftPanelLayoutExpanded.value) return; // no expanded left pane

  isDraggingSplitter.value = true;
  document.addEventListener('mousemove', handleMouseMove);
  document.addEventListener('mouseup', stopDraggingSplitter);
}

// Stop dragging the splitter
function stopDraggingSplitter(event: MouseEvent) {
  isDraggingSplitter.value = false;
  document.removeEventListener('mousemove', handleMouseMove);
  document.removeEventListener('mouseup', stopDraggingSplitter);
}

// Handle mouse move event
function handleMouseMove(event: MouseEvent) {
  if (isDraggingSplitter.value) {
    const pointerX = event.clientX;
    const maxLeftPaneWidth = window.innerWidth / 2;
    const scale = Number(config.settings.scale || 1);
    config.leftPanel.width = Math.round(Math.max(160, Math.min(pointerX, maxLeftPaneWidth)) / scale) - 5;
  }
}

const collectionTrayStyle = computed(() => {
  if (!config.collectionTray.expanded) return {};
  return { height: `${config.collectionTray.height}px` };
});

const showBottomCollectionTray = computed(() =>
  showPanel.value &&
  !libraryEmpty.value &&
  !isStackedPane.value // the stacked pane has no bottom tray
);

function toggleCollectionTray() {
  config.collectionTray.expanded = !config.collectionTray.expanded;
}

function startDraggingCollectionSplitter(event: MouseEvent) {
  if (!config.collectionTray.expanded) return;
  event.preventDefault();
  isDraggingCollectionSplitter.value = true;
  document.addEventListener('mousemove', handleCollectionMouseMove);
  document.addEventListener('mouseup', stopDraggingCollectionSplitter);
}

function stopDraggingCollectionSplitter() {
  isDraggingCollectionSplitter.value = false;
  document.removeEventListener('mousemove', handleCollectionMouseMove);
  document.removeEventListener('mouseup', stopDraggingCollectionSplitter);
}

function handleCollectionMouseMove(event: MouseEvent) {
  if (!isDraggingCollectionSplitter.value || !leftPanelRootRef.value) return;
  const rect = leftPanelRootRef.value.getBoundingClientRect();
  const headerHeight = 52;
  const minMainPanelHeight = 160;
  const minTrayHeight = 120;
  const maxTrayHeight = Math.max(minTrayHeight, rect.height - headerHeight - minMainPanelHeight);
  const nextHeight = rect.bottom - event.clientY - 2;
  config.collectionTray.height = Math.max(minTrayHeight, Math.min(nextHeight, maxTrayHeight));
}

/// click settings icon
async function clickSettings(tabIndex?: number) {
  if (typeof tabIndex === 'number') {
    config.settings.tabIndex = tabIndex;
    await emit('settings-settingsTabIndex-changed', tabIndex);
  }

  // Reuse an existing settings window if it can be shown; otherwise destroy it
  // and create a fresh one. Re-showing a closed transparent window can fail
  // silently on Windows, so never swallow the failure.
  let settingsWindow: WebviewWindow | null = null;
  try {
    settingsWindow = await WebviewWindow.getByLabel('settings');
  } catch (error) {
    console.error('Failed to look up settings window:', error);
  }

  if (settingsWindow) {
    try {
      if (isWin && await settingsWindow.isMinimized()) {
        await settingsWindow.unminimize();
      }
      await settingsWindow.show();
      if (isWin) {
        await settingsWindow.setFocus();
      }
      return;
    } catch (error) {
      console.error('Failed to show existing settings window, recreating:', error);
      try {
        await settingsWindow.destroy();
      } catch (destroyError) {
        console.error('Failed to destroy stale settings window:', destroyError);
      }
    }
  }

  const options: any = {
    url: '/settings',
    title: 'Settings',
    ...await settingsWindowGeometry(getSettingsWindowScale()),
    resizable: true,
    maximizable: false,
    visible: false, // Start hidden, will show after mount
    transparent: true, // Prevent white flash on show (Tauri 2.x workaround)
    decorations: isMac, // true for macOS, false for Windows
    ...(isMac && {
      titleBarStyle: 'Overlay',
      hiddenTitle: true,
      minimizable: false,
    }),
  };

  // create a new settings window
  const newSettingsWindow = new WebviewWindow('settings', options);
  
  newSettingsWindow.once('tauri://created', () => {
    console.log('settings window created');
  });
}

async function openFolderManagerWindow(initialPath?: string) {
  let fmWindow: WebviewWindow | null = null;
  try {
    fmWindow = await WebviewWindow.getByLabel('folder-manager');
  } catch (error) {
    console.error('Failed to look up folder manager window:', error);
  }

  if (fmWindow) {
    try {
      if (isWin && await fmWindow.isMinimized()) {
        await fmWindow.unminimize();
      }
      await fmWindow.show();
      if (isWin) {
        await fmWindow.setFocus();
      }
      return;
    } catch (error) {
      console.error('Failed to show existing folder manager window, recreating:', error);
      try {
        await fmWindow.destroy();
      } catch (destroyError) {
        console.error('Failed to destroy stale folder manager window:', destroyError);
      }
    }
  }

  const query = initialPath ? `?path=${encodeURIComponent(initialPath)}` : '';
  const scale = getSettingsWindowScale();
  const options: any = {
    url: `/folder-manager${query}`,
    title: 'Folder Manager',
    width: Math.round(840 * scale),
    height: Math.round(620 * scale),
    minWidth: Math.round(720 * scale),
    minHeight: Math.round(500 * scale),
    resizable: true,
    maximizable: true,
    visible: false,
    transparent: true,
    decorations: isMac,
    ...(isMac && {
      titleBarStyle: 'Overlay',
      hiddenTitle: true,
      minimizable: false,
    }),
  };

  const newWindow = new WebviewWindow('folder-manager', options);
  newWindow.once('tauri://created', () => {
    console.log('folder manager window created');
  });
}

function getSettingsWindowScale() {
  return SCALE_VALUES.find((item) => item === Number(config.settings.scale)) ?? 1;
}

</script>
