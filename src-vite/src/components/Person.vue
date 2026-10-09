<template>
  <div class="sidebar-panel relative overflow-hidden" :class="stacked ? 'h-auto!' : ''">
    <!-- Face Indexing Progress Overlay -->
    <div v-if="isIndexing" 
      class="absolute inset-0 z-50 bg-base-200/80 backdrop-blur-md"
    >
      <div class="mt-8 px-2 flex flex-col items-center text-base-content/30">
        <IconUpdate class="w-8 h-8 mb-2 animate-spin" />
        <span class="text-sm text-center">
          {{ indexProgress.phase === 'clustering' 
            ? $t('face_index.clustering') 
            : $t('face_index.indexing', { current: indexProgress.current.toLocaleString(), total: indexProgress.total.toLocaleString() }) 
          }}
        </span>
        <span v-if="indexProgress.phase === 'clustering' && clusterProgressText" class="text-xs text-center mt-1">
          {{ clusterProgressText }}
        </span>
        <span v-else-if="indexProgress.faces_found > 0" class="text-xs text-center mt-1">
          {{ $t('face_index.faces_found', { count: indexProgress.faces_found.toLocaleString() }) }}
        </span>
        <button class="btn btn-primary btn-sm mt-4" @click="clickCancelIndex">
          <IconClose class="w-4 h-4" />
          {{ $t('face_index.cancel') }}
        </button>
      </div>
    </div>

    <!-- Incomplete Indexing Warning Banner -->
    <div v-if="allPersons.length > 0 && incompleteCount > 0 && !isIndexing" class="flex-none px-2 py-2">
        <div class="p-3 rounded-box flex flex-row items-center gap-2">
          <IconUpdate class="w-5 h-5 shrink-0" />
          <span class="text-xs flex-1">
            {{ $t('face_index.incomplete', { count: incompleteCount.toLocaleString() }) }}
          </span>
          <button class="btn btn-xs btn-primary" @click="clickIndexFaces">
            {{ $t('face_index.resume') }}
          </button>
        </div>
    </div>

    <div
      class="sidebar-panel-header"
      :class="stacked ? 'cursor-pointer' : ''"
      @click="onStackedHeaderClick"
    >
      <TButton
        v-if="stacked"
        :icon="IconRight"
        :buttonSize="'small'"
        iconClasses="transition-transform duration-200"
        :iconStyle="{ transform: collapsed ? 'rotate(0deg)' : 'rotate(90deg)' }"
        @click.stop="emit('toggle-collapse')"
      />
      <!-- Stacked pane: "People (52)", counting named people only. -->
      <span class="sidebar-panel-header-title flex-1 min-w-0 overflow-hidden text-ellipsis whitespace-nowrap" :class="stacked ? 'normal-case' : ''">
        <template v-if="stacked">{{ titlebar }}<template v-if="namedPersons.length > 0"> ({{ namedPersons.length.toLocaleString() }})</template></template>
        <template v-else>{{ titlebar }}<template v-if="allPersonCount > 0"> ({{ allPersonCount.toLocaleString() }})</template></template>
      </span>

      <ContextMenu :menuItems="personPanelMenuItems" :iconMenu="IconMore" :smallIcon="true" />
    </div>

    <!-- Section body (hidden while the stacked pane's People section is closed) -->
    <div v-show="!(stacked && collapsed)" class="contents">
    <div class="mx-1 mb-2 px-1 shrink-0">
      <div
        :class="[
          'h-8 flex items-center rounded-box transition-colors bg-base-100/40',
          isPersonSearchFocused ? 'border-2 border-primary' : 'border border-base-content/10 hover:border-base-content/30',
          !isLoadingPersons && allPersonCount === 0 ? 'opacity-50' : '',
        ]"
      >
        <IconSearch class="ml-2 w-4 h-4 shrink-0" :class="isPersonSearchFocused ? 'text-primary/70' : 'text-base-content/30'" />
        <input
          v-model="personSearch"
          type="text"
          :disabled="!isLoadingPersons && allPersonCount === 0"
          :placeholder="$t('menu.person.search')"
          class="w-full min-w-0 bg-transparent border-none focus:ring-0 px-2 text-sm placeholder-base-content/30 focus:outline-none disabled:opacity-50"
          @focus="isPersonSearchFocused = true"
          @blur="isPersonSearchFocused = false"
        />
        <button
          v-if="personSearch"
          type="button"
          :disabled="!isLoadingPersons && allPersonCount === 0"
          class="mr-1 p-1 rounded-box text-base-content/30 hover:text-base-content/70 disabled:opacity-30"
          @click="personSearch = ''"
        >
          <IconClose class="w-4 h-4" />
        </button>
      </div>
    </div>

    <!-- Person List: Split into Named and Unnamed sections -->
    <label class="mx-2 mb-2 flex items-center gap-2 text-xs cursor-pointer">
      <input type="checkbox" data-testid="show-globally-ignored" :checked="libConfig.person?.id === -1" @change="toggleGloballyIgnored" />
      Show ignored
    </label>
    <div
      v-if="allPersons.length > 0"
      ref="personListRef"
      :class="stacked ? '' : 'grow overflow-x-hidden overflow-y-auto'"
      @scroll="handlePersonListScroll"
    >
      <!-- Named People Section -->
      <div class="mb-2">
        <div
          class="px-2 py-1.5 flex items-center justify-between text-xs font-bold uppercase tracking-wider text-base-content/60 select-none cursor-pointer hover:text-base-content"
          @click="isNamedSectionCollapsed = !isNamedSectionCollapsed"
        >
          <div class="flex items-center gap-1.5 min-w-0">
            <IconRight
              class="w-3.5 h-3.5 shrink-0 transition-transform"
              :class="{ 'rotate-90': !isNamedSectionCollapsed }"
            />
            <span class="truncate">{{ $t('menu.person.named_people') || 'Named People' }}</span>
          </div>
          <span class="text-[11px] tabular-nums font-normal opacity-70">
            {{ namedPersonCount.toLocaleString() }}
          </span>
        </div>

        <ul v-show="!isNamedSectionCollapsed">
          <li
            v-for="person in namedPersons"
            :key="person.id"
            :id="'person-' + person.id"
          >
            <div
              :class="[
                'sidebar-item gap-2 group',
                selectedPerson && selectedPerson.id === person.id && !isRenamingPerson ? 'sidebar-item-selected' : 'sidebar-item-hover',
              ]"
              @click="selectPerson(person)"
              @contextmenu.prevent.stop="(e: MouseEvent) => handlePersonContextMenu(person, e)"
            >
              <!-- Face thumbnail -->
              <div class="w-8 h-8 rounded-none overflow-hidden bg-base-300/70 ring-1 ring-base-content/5 shrink-0 flex items-center justify-center">
                <img 
                  v-if="person.thumbnail" 
                  :src="'data:image/jpeg;base64,' + person.thumbnail" 
                  class="w-full h-full object-cover pointer-events-none"
                />
                <IconPerson v-else class="w-5 h-5 text-base-content/30" />
              </div>
              
              <!-- Name input or display -->
              <input v-if="selectedPerson && selectedPerson.id === person.id && isRenamingPerson"
                ref="personInputRef"
                type="text"
                maxlength="255"
                class="input px-1 flex-1 focus:border text-base"
                v-model="personNameDraft"
                @keydown.enter="handleRenamePerson"
                @keydown.esc="cancelRenamePerson"
                @blur="handleRenamePerson"
              />
              <template v-else>
                <!-- "Name (count)", bold while the person has suggestions -->
                <span class="sidebar-item-label text-xs" :style="person.suggestions > 0 ? { fontWeight: 700 } : undefined">
                  {{ getPersonDisplayName(person) }}<span v-if="person.count"> ({{ person.count.toLocaleString() }})</span>
                </span>
                <div class="ml-auto flex flex-row items-center gap-1 text-base-content/30">
                  <!-- Orange "?": this person has unconfirmed face suggestions -->
                  <span
                    v-if="person.suggestions > 0"
                    class="shrink-0 w-4 h-4 inline-flex items-center justify-center rounded-full bg-orange-500 text-white text-[10px] font-bold leading-none"
                    :title="`${person.suggestions.toLocaleString()} suggestions`"
                  >?</span>
                  <div :class="[
                      selectedPerson?.id === person.id ? '' : 'hidden group-hover:flex'
                    ]"
                  >
                    <ContextMenu
                      :ref="(el: any) => { if (el) personContextMenus[person.id] = el }"
                      :iconMenu="IconMore"
                      :menuItems="getMoreMenuItems(person)"
                      :smallIcon="true"
                    />
                  </div>
                </div>
              </template>
            </div>
          </li>
          <li v-if="namedPersons.length === 0" class="px-4 py-2 text-xs text-base-content/40 italic">
            {{ $t('menu.person.no_named_people') || 'No named people' }}
          </li>
        </ul>
      </div>

      <!-- Unnamed People Section -->
      <div class="mb-2">
        <div
          class="px-2 py-1.5 flex items-center justify-between text-xs font-bold uppercase tracking-wider text-base-content/60 select-none cursor-pointer hover:text-base-content"
          @click="isUnnamedSectionCollapsed = !isUnnamedSectionCollapsed"
        >
          <div class="flex items-center gap-1.5 min-w-0">
            <IconRight
              class="w-3.5 h-3.5 shrink-0 transition-transform"
              :class="{ 'rotate-90': !isUnnamedSectionCollapsed }"
            />
            <span class="truncate">{{ $t('menu.person.unnamed_people') || 'Unnamed People' }}</span>
          </div>
          <span class="text-[11px] tabular-nums font-normal opacity-70">
            {{ unnamedPersonCount.toLocaleString() }}
          </span>
        </div>

        <ul v-show="!isUnnamedSectionCollapsed">
          <li
            v-for="person in unnamedPersons"
            :key="person.id"
            :id="'person-' + person.id"
          >
            <div
              :class="[
                'sidebar-item gap-2 group',
                selectedPerson && selectedPerson.id === person.id && !isRenamingPerson ? 'sidebar-item-selected' : 'sidebar-item-hover',
              ]"
              @click="selectPerson(person)"
              @contextmenu.prevent.stop="(e: MouseEvent) => handlePersonContextMenu(person, e)"
            >
              <!-- Face thumbnail -->
              <div class="w-8 h-8 rounded-none overflow-hidden bg-base-300/70 ring-1 ring-base-content/5 shrink-0 flex items-center justify-center">
                <img 
                  v-if="person.thumbnail" 
                  :src="'data:image/jpeg;base64,' + person.thumbnail" 
                  class="w-full h-full object-cover pointer-events-none"
                />
                <IconPerson v-else class="w-5 h-5 text-base-content/30" />
              </div>
              
              <!-- Name input or display -->
              <input v-if="selectedPerson && selectedPerson.id === person.id && isRenamingPerson"
                ref="personInputRef"
                type="text"
                maxlength="255"
                class="input px-1 flex-1 focus:border text-base"
                v-model="personNameDraft"
                @keydown.enter="handleRenamePerson"
                @keydown.esc="cancelRenamePerson"
                @blur="handleRenamePerson"
              />
              <template v-else>
                <span class="sidebar-item-label text-xs">
                  {{ getPersonDisplayName(person) }}<span v-if="person.count"> ({{ person.count.toLocaleString() }})</span>
                </span>
                <div class="ml-auto flex flex-row items-center text-base-content/30">
                  <div :class="[
                      selectedPerson?.id === person.id ? '' : 'hidden group-hover:flex'
                    ]"
                  >
                    <ContextMenu
                      :ref="(el: any) => { if (el) personContextMenus[person.id] = el }"
                      :iconMenu="IconMore"
                      :menuItems="getMoreMenuItems(person)"
                      :smallIcon="true"
                    />
                  </div>
                </div>
              </template>
            </div>
          </li>
          <li v-if="unnamedPersons.length === 0" class="px-4 py-2 text-xs text-base-content/40 italic">
            {{ $t('menu.person.no_unnamed_people') || 'No unnamed people' }}
          </li>
        </ul>
      </div>
    </div>

    <div v-else-if="isLoadingPersons" class="mt-2 px-2 flex flex-col items-center justify-center text-base-content/30">
      <span class="text-sm text-center">{{ $t('tooltip.loading') }}</span>
    </div>

    <div v-else-if="personSearch" class="sidebar-empty text-sm">
      <span class="text-center">{{ $t('tooltip.not_found.person') }}</span>
    </div>

    <!-- No Persons Found Message -->
    <div v-else-if="!isIndexing && incompleteCount > 0" class="mt-2 px-2 flex flex-col items-center justify-center text-base-content/30">
      <span class="text-sm text-center">{{ $t('face_index.incomplete', { count: incompleteCount.toLocaleString() }) }}</span>
      <button class="btn btn-primary btn-sm mt-4 rounded-box" @click="clickIndexFaces">
        <IconUpdate class="w-4 h-4" />
        {{ $t('face_index.resume') }}
      </button>
    </div>

    <div v-else-if="!isIndexing" class="mt-2 px-2 flex flex-col items-center justify-center text-base-content/30">
      <span class="text-sm text-center">{{ $t('tooltip.not_found.person') }}</span>
    </div>
    </div>

  </div>

  <!-- Delete person confirmation -->
  <MessageBox
    v-if="showDeletePersonMsgbox"
    :title="$t('msgbox.delete_person.title')"
    :message="`${$t('msgbox.delete_person.content', { person: getPersonDisplayName(selectedPerson) })}`"
    :OkText="$t('msgbox.delete_person.ok')"
    :cancelText="$t('msgbox.cancel')"
    :warningOk="true"
    @ok="clickDeletePerson"
    @cancel="showDeletePersonMsgbox = false"
  />

  <!-- Reset faces confirmation -->
  <MessageBox
    v-if="showResetFacesMsgbox"
    :title="$t('msgbox.reset_faces.title')"
    :message="$t('msgbox.reset_faces.content')"
    :OkText="$t('msgbox.reset_faces.ok')"
    :cancelText="$t('msgbox.cancel')"
    :warningOk="true"
    @ok="onResetFacesConfirm"
    @cancel="showResetFacesMsgbox = false"
  />

  <!-- Merge person confirmation -->
  <MessageBox
    v-if="showMergeConfirmMsgbox"
    :title="$t('menu.person.merge_title') || 'Merge People'"
    :message="mergeConfirmMessage"
    :OkText="$t('menu.person.merge') || 'Merge'"
    :cancelText="$t('msgbox.cancel') || 'Cancel'"
    :warningOk="false"
    @ok="confirmMerge"
    @cancel="cancelMergeConfirm"
  />

  <!-- Merge Person Modal Dialog -->
  <ModalDialog
    v-if="showMergeDialog"
    :title="$t('menu.person.merge_title') || 'Merge Person'"
    :width="360"
    @cancel="showMergeDialog = false"
  >
    <div class="flex flex-col gap-3 min-h-0">
      <div class="text-xs text-base-content/70">
        {{ $t('menu.person.merge_desc', { name: getPersonDisplayName(mergeSourcePerson) }) || `Merge '${getPersonDisplayName(mergeSourcePerson)}' into:` }}
      </div>

      <div class="h-8 flex items-center rounded-box border border-base-content/20 bg-base-100/40 px-2">
        <IconSearch class="w-4 h-4 text-base-content/30 mr-1.5 shrink-0" />
        <input
          v-model="mergeSearchText"
          type="text"
          :placeholder="$t('menu.person.search_target') || 'Search target person...'"
          class="w-full bg-transparent border-none focus:ring-0 text-xs focus:outline-none placeholder-base-content/30"
        />
        <button
          v-if="mergeSearchText"
          type="button"
          class="p-0.5 text-base-content/30 hover:text-base-content/70 cursor-pointer"
          @click="mergeSearchText = ''"
        >
          <IconClose class="w-3.5 h-3.5" />
        </button>
      </div>

      <div class="max-h-60 overflow-y-auto border border-base-content/10 rounded-box p-1 space-y-0.5">
        <div
          v-for="candidate in mergeCandidates"
          :key="candidate.id"
          :class="[
            'flex items-center gap-2 px-2 py-1.5 rounded-box cursor-pointer select-none text-xs transition-colors',
            selectedMergeTargetId === candidate.id ? 'bg-primary text-primary-content' : 'hover:bg-base-content/10',
          ]"
          @click="selectedMergeTargetId = candidate.id"
        >
          <div class="w-6 h-6 rounded-none overflow-hidden bg-base-300/70 shrink-0 flex items-center justify-center">
            <img 
              v-if="candidate.thumbnail" 
              :src="'data:image/jpeg;base64,' + candidate.thumbnail" 
              class="w-full h-full object-cover pointer-events-none"
            />
            <IconPerson v-else class="w-4 h-4 opacity-50" />
          </div>
          <span class="flex-1 truncate font-medium">{{ getPersonDisplayName(candidate) }}</span>
          <span v-if="candidate.count" class="text-[10px] opacity-70">{{ candidate.count }}</span>
        </div>
        <div v-if="mergeCandidates.length === 0" class="text-center py-4 text-xs text-base-content/40 italic">
          {{ $t('tooltip.not_found.person') || 'No people found' }}
        </div>
      </div>

      <div class="flex justify-end gap-2 mt-2">
        <button class="btn btn-sm btn-ghost" @click="showMergeDialog = false">
          {{ $t('msgbox.cancel') || 'Cancel' }}
        </button>
        <button
          class="btn btn-sm btn-primary"
          :disabled="!selectedMergeTargetId"
          @click="confirmModalMerge"
        >
          {{ $t('menu.person.merge') || 'Merge' }}
        </button>
      </div>
    </div>
  </ModalDialog>

  <teleport to="body">
    <transition name="fade">
      <div
        v-if="isBetaTooltipVisible && config.settings.showToolTip"
        ref="betaTooltipRef"
        class="fixed z-1000 px-2 py-1 text-xs whitespace-nowrap rounded-box bg-neutral text-neutral-content shadow-lg pointer-events-none"
        :style="betaTooltipStyle"
      >
        {{ $t('tooltip.beta.person') }}
      </div>
    </transition>
  </teleport>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed, nextTick, watch, type PropType } from 'vue';
import { listen } from '@tauri-apps/api/event';
import { useI18n } from 'vue-i18n';
import { invoke } from '@tauri-apps/api/core';
import { config, libConfig } from '@/common/config';
import { getPersonsPage, renamePerson, deletePerson, indexFaces, cancelFaceIndex, isFaceIndexing, listenFaceIndexProgress, listenFaceIndexFinished, listenClusterProgress, resetFaces, getFaceStats, computeFaceSuggestions } from '@/common/api';
import { SIDEBAR } from '@/common/constants';
import { 
  IconPerson, 
  IconMore, 
  IconRename, 
  IconTrash,
  IconUpdate,
  IconClose,
  IconSearch,
  IconRight,
  IconGroup,
} from '@/common/icons';

import ContextMenu from '@/components/ContextMenu.vue';
import TButton from '@/components/TButton.vue';
import MessageBox from '@/components/MessageBox.vue';
import ModalDialog from '@/components/ModalDialog.vue';

const props = defineProps({
  titlebar: {
    type: String,
    required: true
  },
  // A section of the stacked left pane (People, Albums, Folders in one scrolling column).
  stacked: {
    type: Boolean,
    default: false
  },
  collapsed: {
    type: Boolean,
    default: false
  },
  // The stacked pane's shared scrolling column (drives paging instead of this list).
  scroller: {
    type: null as unknown as PropType<HTMLElement | null>,
    default: null
  }
});

const emit = defineEmits(['editDataChanged', 'toggle-collapse']);
const personListRef = ref<HTMLElement | null>(null);

// Stacked pane: the header bar opens / closes the section (its buttons keep their own jobs).
function onStackedHeaderClick(event: MouseEvent) {
  if (!props.stacked) return;
  if (event.target instanceof Element && event.target.closest('button, input, [role="button"]')) return;
  emit('toggle-collapse');
}

// The People view is the one on screen (the stacked pane keeps this list mounted beside
// Albums and Folders, so it must not change the selection behind another view).
const isPeopleViewActive = () =>
  config.main.sidebarIndex === SIDEBAR.PERSON && libConfig.activePane === 'main';

/// i18n
const { locale, messages } = useI18n();
const localeMsg = computed(() => messages.value[locale.value] as any);

// persons
const allPersons = ref<any[]>([]);
const selectedPerson = ref<any>(null);
const isRenamingPerson = ref(false);
const originalPersonName = ref('');
const personNameDraft = ref('');
const personInputRef = ref<HTMLInputElement[]>([]);
const isIndexing = ref(false);
const indexProgress = ref({
  current: 0,
  total: 0,
  faces_found: 0,
  phase: 'indexing'
});
const clusterProgress = ref({
  phase: '',
  current: 0,
  total: 0
});
const incompleteCount = ref(0);
const personContextMenus = ref<Record<number, any>>({});
const isLoadingPersons = ref(true);
const isLoadingMorePersons = ref(false);
const hasMorePersons = ref(false);
const allPersonCount = ref(0);
const namedPersonCount = ref(0);
const unnamedPersonCount = ref(0);
const personSearch = ref('');
const isPersonSearchFocused = ref(false);
const PERSON_PAGE_SIZE = 100;
let personLoadRequest = 0;
let personSearchTimer: ReturnType<typeof setTimeout> | null = null;
let isPersonMounted = true;

function handlePersonContextMenu(person: any, event: MouseEvent) {
  selectPerson(person);
  personContextMenus.value[person.id]?.open?.(event.clientX, event.clientY);
}
const betaBadgeRef = ref<HTMLElement | null>(null);
const betaTooltipRef = ref<HTMLElement | null>(null);
const isBetaTooltipVisible = ref(false);
const betaTooltipStyle = ref<Record<string, string>>({});

// Event listener unsubscribe functions
let unlistenProgress: (() => void) | null = null;
let unlistenFinished: (() => void) | null = null;
let unlistenCluster: (() => void) | null = null;
let unlistenPicasa: (() => void) | null = null;

const sortedPersons = computed(() => allPersons.value);

// Conceptual sections: Named People vs Unnamed People
const isNamedSectionCollapsed = ref(false);
const localUnnamedCollapsed = ref(false);
// Stacked pane: the Unnamed list (thousands of groups) starts closed so the column can
// scroll on to Albums and Folders; remembered in config.
const isUnnamedSectionCollapsed = computed({
  get: () => props.stacked ? config.leftStack.unnamedCollapsed !== false : localUnnamedCollapsed.value,
  set: (value: boolean) => {
    if (props.stacked) config.leftStack.unnamedCollapsed = value;
    else localUnnamedCollapsed.value = value;
  },
});

const isNamedPerson = (person: any): boolean => {
  if (!person || !person.name) return false;
  return !person.name.trim().startsWith('Person ');
};

const namedPersons = computed(() => {
  return sortedPersons.value.filter(p => isNamedPerson(p));
});

const unnamedPersons = computed(() => {
  return sortedPersons.value.filter(p => !isNamedPerson(p));
});


// Merge dialog & confirmation
const showMergeConfirmMsgbox = ref(false);
const pendingMergeSource = ref<any>(null);
const pendingMergeTarget = ref<any>(null);

const showMergeDialog = ref(false);
const mergeSourcePerson = ref<any>(null);
const selectedMergeTargetId = ref<number | null>(null);
const mergeSearchText = ref('');

function openMergeDialog(person: any) {
  mergeSourcePerson.value = person;
  selectedMergeTargetId.value = null;
  mergeSearchText.value = '';
  showMergeDialog.value = true;
}

const mergeCandidates = computed(() => {
  if (!mergeSourcePerson.value) return [];
  const srcId = mergeSourcePerson.value.id;
  const q = mergeSearchText.value.trim().toLowerCase();
  return allPersons.value.filter(p => {
    if (p.id === srcId) return false;
    if (!q) return true;
    const name = (p.name || '').toLowerCase();
    return name.includes(q);
  });
});

function confirmModalMerge() {
  if (!selectedMergeTargetId.value || !mergeSourcePerson.value) return;
  const target = allPersons.value.find(p => p.id === selectedMergeTargetId.value);
  if (!target) return;

  pendingMergeSource.value = mergeSourcePerson.value;
  pendingMergeTarget.value = target;
  showMergeDialog.value = false;
  showMergeConfirmMsgbox.value = true;
}

const mergeConfirmMessage = computed(() => {
  const srcName = getPersonDisplayName(pendingMergeSource.value);
  const tgtName = getPersonDisplayName(pendingMergeTarget.value);
  const template = localeMsg.value.msgbox?.merge_person?.content;
  if (template) {
    return template.replace('{source}', srcName).replace('{target}', tgtName);
  }
  return `Are you sure you want to merge '${srcName}' into '${tgtName}'? All photos will be moved to '${tgtName}'.`;
});

function cancelMergeConfirm() {
  showMergeConfirmMsgbox.value = false;
  pendingMergeSource.value = null;
  pendingMergeTarget.value = null;
}

async function confirmMerge() {
  showMergeConfirmMsgbox.value = false;
  if (pendingMergeTarget.value && pendingMergeSource.value) {
    await executeMerge(pendingMergeTarget.value.id, pendingMergeSource.value.id);
  }
  pendingMergeTarget.value = null;
  pendingMergeSource.value = null;
}

async function executeMerge(targetId: number, sourceId: number) {
  try {
    // Calling Rust IPC command invoke('merge_person', { targetId, sourceId })
    await invoke('merge_person', { targetId, sourceId });

    if (selectedPerson.value?.id === sourceId) {
      const target = allPersons.value.find(p => p.id === targetId);
      if (target) {
        selectPerson(target);
      } else {
        selectedPerson.value = null;
        if (libConfig.person) {
          libConfig.person.id = targetId;
        }
      }
    }

    await loadPersons();

    if (libConfig.person?.id === targetId) {
      emit('editDataChanged');
    }
  } catch (error) {
    console.error('Failed to merge person:', error);
  }
}

// Computed property to format cluster progress text using i18n
const { t } = useI18n();
const getPersonDisplayName = (person: any) => person?.name || t('menu.person.unnamed');
const clusterProgressText = computed(() => {
  const { phase, current, total } = clusterProgress.value;
  switch (phase) {
    case 'graph':
      return t('face_index.cluster_graph', { percent: current });
    case 'iterate':
      return t('face_index.cluster_iterate', { current, total });
    case 'converged':
      return t('face_index.cluster_converged', { current });
    case 'assign':
      return t('face_index.cluster_assign', { current, total });
    case 'thumbnail':
      return t('face_index.cluster_thumbnail');
    default:
      return '';
  }
});

const personPanelMenuItems = computed(() => [
  {
    label: localeMsg.value.menu.person.index_faces,
    icon: IconUpdate,
    action: () => clickIndexFaces(),
    disabled: isIndexing.value,
  },
  {
    label: 'Find face suggestions',
    icon: IconPerson,
    action: () => clickFindSuggestions(),
    disabled: isIndexing.value || isFindingSuggestions.value,
  },
  { label: "-", action: null },
  {
    label: localeMsg.value.menu.person.reset_index,
    icon: IconTrash,
    action: () => clickResetFaces(),
    disabled: isIndexing.value,
  },
]);

// message boxes
const showDeletePersonMsgbox = ref(false);
const showResetFacesMsgbox = ref(false);

// more menuitems
const getMoreMenuItems = (person: any) => [
  {
    label: localeMsg.value.menu?.person?.rename || 'Rename',
    icon: IconRename,
    action: () => {
      selectPerson(person);
      isRenamingPerson.value = true;
      originalPersonName.value = person.name || '';
      personNameDraft.value = originalPersonName.value;
      nextTick(() => {
        if (personInputRef.value && personInputRef.value[0]) {
          personInputRef.value[0].focus();
        }
      });
    }
  },
  {
    label: localeMsg.value.menu?.person?.merge || 'Merge into...',
    icon: IconGroup,
    action: () => {
      openMergeDialog(person);
    }
  },
  { label: "-", action: null },
  {
    label: localeMsg.value.menu?.person?.delete || 'Delete',
    icon: IconTrash,
    action: () => {
      selectPerson(person);
      showDeletePersonMsgbox.value = true;
    },
  },
];

// Suggestions: match unconfirmed faces against named people.
const isFindingSuggestions = ref(false);
async function clickFindSuggestions() {
  isFindingSuggestions.value = true;
  try {
    await computeFaceSuggestions();
    await loadPersons();
    emit('editDataChanged');
  } finally {
    isFindingSuggestions.value = false;
  }
}

// The person view confirms/rejects suggestions; keep the row's "?" badge in step.
function reloadGloballyIgnored() { void loadPersons(); }

function onSuggestionsChanged(event: Event) {
  const { personId, suggestions } = (event as CustomEvent).detail || {};
  const person = allPersons.value.find(p => p.id === personId);
  if (person) {
    person.suggestions = suggestions;
    if (typeof (event as CustomEvent).detail?.count === 'number') person.count = (event as CustomEvent).detail.count;
  }
}

onMounted(async () => {
  window.addEventListener('lapcasa-global-ignores-changed', reloadGloballyIgnored);
  window.addEventListener('lapcasa-face-suggestions-changed', onSuggestionsChanged);
  unlistenPicasa = await listen('picasa-import-finished', () => { loadPersons(); });
  loadPersons();
  checkFaceStats();
  
  // Check if indexing is already running and restore progress
  const [isRunning, progress] = await isFaceIndexing();
  
  if (isRunning) {
    isIndexing.value = true;
    if (progress) {
      indexProgress.value = progress;
    }
  }
  
  // Set up event listeners for face indexing progress
  unlistenProgress = await listenFaceIndexProgress((event: any) => {
    isIndexing.value = true; // Show overlay when receiving progress events
    indexProgress.value = event.payload;
  });
  
  unlistenFinished = await listenFaceIndexFinished((event: any) => {
    isIndexing.value = false;
    indexProgress.value = { current: 0, total: 0, faces_found: 0, phase: 'indexing' };
    clusterProgress.value = { phase: '', current: 0, total: 0 };
    loadPersons(); // Reload persons after indexing completes
    checkFaceStats();
  });
  
  // Listen for detailed clustering progress
  unlistenCluster = await listenClusterProgress((event: any) => {
    clusterProgress.value = event.payload;
  });
});

watch(() => config.settings.categorySort, () => {
  loadPersons();
});
watch(() => [config.settings.face?.minimumUnnamedSightings, config.settings.face?.hideDuplicateUnnamed], () => {
  void loadPersons(true, true);
});

// The People view scrolls from person to person: keep the highlighted row in step.
watch(() => libConfig.person?.id, (id) => {
  if (!id || selectedPerson.value?.id === id || isRenamingPerson.value) return;
  const person = allPersons.value.find(p => p.id === id);
  if (!person) return;
  selectedPerson.value = person;
  nextTick(() => document.getElementById('person-' + id)?.scrollIntoView({ block: 'nearest' }));
});

watch(personSearch, () => {
  if (personSearchTimer) clearTimeout(personSearchTimer);
  personLoadRequest++;
  allPersons.value = [];
  hasMorePersons.value = false;
  isLoadingPersons.value = true;
  isLoadingMorePersons.value = false;
  personSearchTimer = setTimeout(() => {
    personSearchTimer = null;
    void loadPersons();
  }, 200);
});

onUnmounted(() => {
  window.removeEventListener('lapcasa-global-ignores-changed', reloadGloballyIgnored);
  observedScroller?.removeEventListener('scroll', handleStackedScroll);
  isPersonMounted = false;
  personLoadRequest++;
  if (personSearchTimer) clearTimeout(personSearchTimer);
  if (unlistenProgress) unlistenProgress();
  if (unlistenFinished) unlistenFinished();
  if (unlistenCluster) unlistenCluster();
  if (unlistenPicasa) unlistenPicasa();
  window.removeEventListener('lapcasa-face-suggestions-changed', onSuggestionsChanged);
});

async function loadPersons(reset = true, validateSelectedPerson = false) {
  if (!reset && (!hasMorePersons.value || isLoadingMorePersons.value || isLoadingPersons.value)) return;

  const requestId = reset ? ++personLoadRequest : personLoadRequest;
  const libraryId = libConfig._libraryId;
  const search = personSearch.value.trim();
  if (reset) {
    isLoadingPersons.value = true;
    allPersons.value = [];
    hasMorePersons.value = false;
  } else {
    isLoadingMorePersons.value = true;
  }

  try {
    const page = await getPersonsPage({
      sort: config.settings.categorySort,
      offset: reset ? 0 : allPersons.value.length,
      limit: PERSON_PAGE_SIZE,
      search,
      minimumUnnamedSightings: config.settings.face?.minimumUnnamedSightings ?? 2,
      hideDuplicateUnnamed: config.settings.face?.hideDuplicateUnnamed ?? false,
      refreshSummary: validateSelectedPerson
        ? { selectedPersonId: libConfig.person?.id ?? null }
        : null,
    });
    if (!isPersonMounted || requestId !== personLoadRequest || libraryId !== libConfig._libraryId) return;

    if (page) {
      namedPersonCount.value = page.named_total ?? namedPersons.value.length;
      unnamedPersonCount.value = page.unnamed_total ?? unnamedPersons.value.length;
      const selectedPersonWasFiltered = validateSelectedPerson && page.selected_person_visible === false;
      if (selectedPersonWasFiltered) {
        selectedPerson.value = null;
        if (libConfig.person) {
          libConfig.person.id = null;
          libConfig.person.name = null;
        }
      }
      allPersons.value = reset
        ? page.persons
        : [...allPersons.value, ...page.persons];
      hasMorePersons.value = page.has_more;
      if (page.visible_total != null) allPersonCount.value = page.visible_total;
      else if (!search) allPersonCount.value = page.total;
      if (allPersons.value.length > 0 && !selectedPerson.value && !selectedPersonWasFiltered && libConfig.person?.id !== -1) {
        const index = allPersons.value.findIndex(p => p.id === libConfig.person?.id);
        if (props.stacked && !isPeopleViewActive()) {
          // Another view is on screen: only show the remembered person as selected.
          if (index >= 0) selectedPerson.value = allPersons.value[index];
        } else {
          selectPerson(allPersons.value[index >= 0 ? index : 0]);
        }
      }
    } else if (libConfig.person) {
      libConfig.person.id = null;
    }
  } finally {
    if (requestId === personLoadRequest) {
      isLoadingPersons.value = false;
      isLoadingMorePersons.value = false;
    }
  }
}

function handlePersonListScroll(event: Event) {
  if (props.stacked) return;
  const target = event.currentTarget as HTMLElement;
  if (target.scrollTop + target.clientHeight < target.scrollHeight - 24) return;
  void loadPersons(false);
}

// Stacked pane: load the next page when the end of this list nears the bottom of the
// shared column. With Unnamed closed, nothing more is shown once the unnamed groups
// start (named people load first), so stop paging.
function handleStackedScroll() {
  const list = personListRef.value;
  const scroller = props.scroller;
  if (!list || !scroller || props.collapsed || !hasMorePersons.value) return;
  if (isUnnamedSectionCollapsed.value && unnamedPersons.value.length > 0) return;
  if (list.getBoundingClientRect().bottom > scroller.getBoundingClientRect().bottom + 200) return;
  void loadPersons(false);
}
let observedScroller: HTMLElement | null = null;
watch(() => props.scroller, (scroller) => {
  observedScroller?.removeEventListener('scroll', handleStackedScroll);
  observedScroller = (scroller as HTMLElement | null) || null;
  observedScroller?.addEventListener('scroll', handleStackedScroll, { passive: true });
}, { immediate: true });

function selectPerson(person: any) {
  if (isRenamingPerson.value) return;
  selectedPerson.value = person;
  if (!libConfig.person) {
    libConfig.person = { id: null, name: null };
  }
  libConfig.person.id = person.id;
  libConfig.person.name = person.name;
}

let previousIgnoredPersonId: number | null = null;
async function toggleGloballyIgnored() {
  if (libConfig.person?.id === -1) {
    libConfig.person.id = previousIgnoredPersonId;
    await loadPersons();
    const person = allPersons.value.find(p => p.id === previousIgnoredPersonId) || allPersons.value[0];
    if (person) selectPerson(person);
    else libConfig.person = { id: null, name: null };
  } else {
    previousIgnoredPersonId = libConfig.person?.id ?? null;
    selectedPerson.value = null;
    libConfig.person = { id: -1, name: 'Ignored faces' };
  }
  config.main.sidebarIndex = SIDEBAR.PERSON;
  libConfig.activePane = 'main';
}

async function handleRenamePerson() {
  if (!isRenamingPerson.value || !selectedPerson.value) return;

  const newName = personNameDraft.value.trim();

  if (newName.length === 0 || newName === originalPersonName.value) {
    isRenamingPerson.value = false;
    selectedPerson.value.name = originalPersonName.value;
    return;
  }

  // Check if a person with this new name already exists (case-insensitive)
  const existingPerson = allPersons.value.find(
    p => p.id !== selectedPerson.value.id && p.name && p.name.trim().toLowerCase() === newName.toLowerCase()
  );

  if (existingPerson) {
    // Prompt to merge
    pendingMergeSource.value = selectedPerson.value;
    pendingMergeTarget.value = existingPerson;
    showMergeConfirmMsgbox.value = true;
    isRenamingPerson.value = false;
    selectedPerson.value.name = originalPersonName.value;
    return;
  }

  const result = await renamePerson(selectedPerson.value.id, newName);
  if (result) {
    isRenamingPerson.value = false;
    await loadPersons();
  }
}

function cancelRenamePerson() {
  if (selectedPerson.value) {
    selectedPerson.value.name = originalPersonName.value;
  }
  isRenamingPerson.value = false;
}

async function clickDeletePerson() {
  if (selectedPerson.value) {
    showDeletePersonMsgbox.value = false;
    const result = await deletePerson(selectedPerson.value.id);
    if (result) {
      const index = allPersons.value.findIndex(p => p.id === selectedPerson.value.id);
      allPersons.value = allPersons.value.filter(p => p.id !== selectedPerson.value.id);
      allPersonCount.value = Math.max(0, allPersonCount.value - 1);
      if (index > 0) {
        selectPerson(allPersons.value[index - 1]);
      } else if (index === 0) {
        if (allPersons.value.length > 0) {
          selectPerson(allPersons.value[0]);
        } else {
          selectedPerson.value = null;
          if (libConfig.person) {
            libConfig.person.id = null;
          }
        }
      } else {
        selectedPerson.value = null;
        if (libConfig.person) {
          libConfig.person.id = null;
        }
      }
    }
  }
}

// Called from title bar context menu
async function clickIndexFaces() {
  if (isIndexing.value) {
    return;
  }
  
  isIndexing.value = true;
  try {
    // Get cluster threshold from array using index
    const face = config.settings.face;
    const thresholdIndex = face?.clusterThresholdIndex ?? 2; // Default: Medium (index 2)
    // Use getter for thresholds to ensure we get the latest values, even if state is old
    const thresholds = config.faceClusterThresholds ?? [0.35, 0.45, 0.55, 0.65];
    const clusterEpsilon = thresholds[thresholdIndex] ?? 0.55;
    console.log('clusterEpsilon', clusterEpsilon);
    await indexFaces(clusterEpsilon);
    await loadPersons();
    await checkFaceStats();
  } catch (e) {
    console.error('indexFaces error:', e);
    isIndexing.value = false;
  }
}

// Cancel face indexing
async function clickCancelIndex() {
  await cancelFaceIndex();
}

// Reset faces
async function clickResetFaces() {
  showResetFacesMsgbox.value = true;
}

async function onResetFacesConfirm() {
  showResetFacesMsgbox.value = false;
  
  // Reset selection and config
  selectedPerson.value = null;
  if (libConfig.person) {
    libConfig.person.id = null;
    libConfig.person.name = null;
  }

  await resetFaces();
  if (personSearch.value) {
    personSearch.value = '';
    await nextTick();
    if (personSearchTimer) clearTimeout(personSearchTimer);
    personSearchTimer = null;
  }
  allPersonCount.value = 0;
  await loadPersons();
  checkFaceStats();
}

async function checkFaceStats() {
  const stats = await getFaceStats();
  if (stats) {
    incompleteCount.value = stats.unprocessed;
  }
}

// Only refresh the active view. Inactive panel data is refreshed on re-entry.

watch(() => [config.main.sidebarIndex, libConfig.activePane], () => {
  if (libConfig.activePane === 'main' && config.main.sidebarIndex === SIDEBAR.PERSON) {
    if (props.stacked) {
      // The stacked pane keeps this list mounted and live: reloading here would empty it
      // (and the column's scroll place) on every switch from Albums or Folders.
      if (!libConfig.person?.id && allPersons.value.length > 0) selectPerson(allPersons.value[0]);
      void checkFaceStats();
      return;
    }
    void loadPersons(true, true);
    void checkFaceStats();
  }
});

async function showBetaTooltip() {
  if (!config.settings.showToolTip || !betaBadgeRef.value) return;

  isBetaTooltipVisible.value = true;
  await nextTick();

  if (!betaBadgeRef.value || !betaTooltipRef.value) return;

  const rect = betaBadgeRef.value.getBoundingClientRect();
  const tooltipRect = betaTooltipRef.value.getBoundingClientRect();
  const padding = 4;

  let top = rect.bottom + padding;
  let left = rect.left + (rect.width - tooltipRect.width) / 2;

  if (left + tooltipRect.width > window.innerWidth - padding) {
    left = window.innerWidth - tooltipRect.width - padding;
  }
  if (left < padding) {
    left = padding;
  }
  if (top + tooltipRect.height > window.innerHeight - padding) {
    top = rect.top - tooltipRect.height - padding;
  }

  betaTooltipStyle.value = {
    top: `${top}px`,
    left: `${left}px`,
  };
}

function hideBetaTooltip() {
  isBetaTooltipVisible.value = false;
}

defineExpose({
  clickIndexFaces,
  clickCancelIndex,
  loadPersons,
  clickResetFaces,
  isIndexing,
});

</script>
