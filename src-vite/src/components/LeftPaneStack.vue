<template>
  <!-- Stacked left pane: Albums, People and Folders as collapsible sections of one
       continuous scrolling column. The long lists do not scroll themselves: Folders is
       virtualised against this column and People pages in as the column nears its end. -->
  <div ref="scrollerRef" class="relative h-full overflow-x-hidden overflow-y-auto" data-left-pane-stack="true" @scroll="updateSectionInView">
    <div>
      <section
        ref="albumsRef"
        @click.capture="onItemEvent($event, SIDEBAR.SMART_ALBUM)"
        @contextmenu.capture="onItemEvent($event, SIDEBAR.SMART_ALBUM)"
      >
        <SmartAlbumList
          :stacked="true"
          :collapsed="config.leftStack.albumsCollapsed"
          @toggle-collapse="config.leftStack.albumsCollapsed = !config.leftStack.albumsCollapsed"
        />
      </section>

      <section
        ref="peopleRef"
        :class="libConfig.activePane === 'collection' ? 'sidebar-pane-inactive' : ''"
        @click.capture="onItemEvent($event, SIDEBAR.PERSON)"
        @contextmenu.capture="onItemEvent($event, SIDEBAR.PERSON)"
      >
        <Person
          :titlebar="localeMsg.sidebar.people"
          :stacked="true"
          :collapsed="config.leftStack.peopleCollapsed"
          :scroller="scrollerRef"
          @toggle-collapse="config.leftStack.peopleCollapsed = !config.leftStack.peopleCollapsed"
        />
      </section>

      <section
        ref="foldersRef"
        :class="libConfig.activePane === 'collection' ? 'sidebar-pane-inactive' : ''"
        @click.capture="onItemEvent($event, SIDEBAR.ALBUM)"
        @contextmenu.capture="onItemEvent($event, SIDEBAR.ALBUM)"
        @keydown.capture="onFolderKeyDown"
      >
        <AlbumList
          ref="albumListRef"
          selectionSource="album"
          :titlebar="localeMsg.sidebar.album"
          :stacked="true"
          :collapsed="config.leftStack.foldersCollapsed"
          :scroller="scrollerRef"
          @toggle-collapse="config.leftStack.foldersCollapsed = !config.leftStack.foldersCollapsed"
        />
      </section>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { config, libConfig } from '@/common/config';
import { SIDEBAR } from '@/common/constants';
import { isEditableTarget } from '@/common/utils';
import Person from '@/components/Person.vue';
import AlbumList from '@/components/AlbumList.vue';
import SmartAlbumList from '@/components/SmartAlbumList.vue';

const { locale, messages } = useI18n();
const localeMsg = computed(() => messages.value[locale.value] as any);

const scrollerRef = ref<HTMLElement | null>(null);
const albumsRef = ref<HTMLElement | null>(null);
const peopleRef = ref<HTMLElement | null>(null);
const foldersRef = ref<HTMLElement | null>(null);
const albumListRef = ref<any>(null);

// Picking a person or a folder (click, right-click) switches the grid to that view, as the
// separate People / Folders panels did. Runs before the row's own handler, so the view and
// the selection change together (one content load).
function onItemEvent(event: Event, index: number) {
  const target = event.target instanceof Element ? event.target : null;
  if (!target || target.closest('input')) return;
  const row = index === SIDEBAR.PERSON ? 'li[id^="person-"]'
    : index === SIDEBAR.SMART_ALBUM ? '.sidebar-item' : 'li[id^="folder-"]';
  if (!target.closest(row)) return;
  libConfig.activePane = 'main';
  if (config.main.sidebarIndex !== index) config.main.sidebarIndex = index;
}

// Arrow keys in the focused folder list move the folder selection: Folders view.
function onFolderKeyDown(event: KeyboardEvent) {
  if (isEditableTarget(event.target)) return;
  if (!['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
  if (!(event.target instanceof Element) || !event.target.closest('[data-album-list-root="true"]')) return;
  libConfig.activePane = 'main';
  if (config.main.sidebarIndex !== SIDEBAR.ALBUM) config.main.sidebarIndex = SIDEBAR.ALBUM;
}

const emit = defineEmits<{ (e: 'section-in-view', index: number): void }>();

const sections = () => [
  { index: SIDEBAR.SMART_ALBUM, el: albumsRef.value, collapsedKey: 'albumsCollapsed' },
  { index: SIDEBAR.PERSON, el: peopleRef.value, collapsedKey: 'peopleCollapsed' },
  { index: SIDEBAR.ALBUM, el: foldersRef.value, collapsedKey: 'foldersCollapsed' },
] as const;

// Scrolling lights up the rail icon of the section at the top of the column.
function updateSectionInView() {
  const scroller = scrollerRef.value;
  if (!scroller) return;
  let current: number = SIDEBAR.SMART_ALBUM;
  for (const section of sections()) {
    if (section.el && section.el.offsetTop <= scroller.scrollTop + 8) current = section.index;
  }
  emit('section-in-view', current);
}
let resizeObserver: ResizeObserver | null = null;
onMounted(() => {
  resizeObserver = new ResizeObserver(() => updateSectionInView());
  for (const section of sections()) if (section.el) resizeObserver.observe(section.el);
  updateSectionInView();
});
onBeforeUnmount(() => resizeObserver?.disconnect());

/// Rail icon: open the Albums, People or Folders section and scroll the column to it. Without
/// `force`, returns false (nothing done) when the section is open and its header is in view.
function revealSection(index: number, force = false): boolean {
  const target = sections().find(item => item.index === index);
  if (!target) return false;
  const isFolders = index === SIDEBAR.ALBUM;
  const section = target.el;
  const scroller = scrollerRef.value;
  if (!section || !scroller) return false;
  const collapsedKey = target.collapsedKey;
  if (!force && !config.leftStack[collapsedKey]) {
    const top = section.offsetTop - scroller.scrollTop;
    if (top >= -2 && top < scroller.clientHeight - 24) return false;
  }
  config.leftStack[collapsedKey] = false;
  void nextTick(() => {
    scroller.scrollTop = section.offsetTop;
    if (isFolders) albumListRef.value?.revealSelectedFolder?.();
  });
  return true;
}

defineExpose({
  revealSection,
  openAlbumEdit: (albumId: number) => albumListRef.value?.openAlbumEdit?.(albumId),
});
</script>
