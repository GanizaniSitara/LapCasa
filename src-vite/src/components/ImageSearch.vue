<template>
    
  <div class="sidebar-panel">
    <!-- title bar -->
    <div class="sidebar-panel-header" data-tauri-drag-region>
      <span class="sidebar-panel-header-title flex-1">
        {{ titlebar }}<template v-if="searchHistoryList.length > 0"> ({{ searchHistoryList.length.toLocaleString() }})</template>
      </span>
      <ContextMenu :menuItems="searchPanelMenuItems" :iconMenu="IconMore" :smallIcon="true" />
    </div>

    <div class="mx-1 mb-2 px-1 shrink-0">
      <SearchBox
        ref="searchInputRef"
        v-model="searchQuery"
        :initial-tokens="activeTokens"
        always-expanded
        @search="handleSearch"
      />
    </div>

    <!-- search history -->
    <div class="overflow-y-auto flex-1" >
        <div v-for="(item, index) in searchHistoryList" :key="index"
          :class="[
            'sidebar-item group',
            libConfig.search.searchHistoryIndex === index ? 'sidebar-item-selected' : 'sidebar-item-hover',
          ]"
          @click="handleSearchHistoryClick(index)"
        >
          <IconSearch class="mx-1 h-5 shrink-0" />
          
          <span class="sidebar-item-label">{{ typeof item === 'string' ? item : item.text }}</span>
          <div class="ml-auto flex flex-row items-center text-base-content/30">
            <span
              v-if="hasSearchHistoryCount(item)"
              class="sidebar-item-count shrink-0"
            >{{ formatSearchResultCount(getSearchHistoryCount(item)) }}</span>
            <div
              :class="[
                libConfig.search.searchHistoryIndex === index ? '' : 'hidden group-hover:flex'
              ]"
            >
              <ContextMenu
                :iconMenu="IconMore"
                :menuItems="() => getSearchHistoryMenuItems(index)"
                :smallIcon="true"
              />
            </div>
          </div>
        </div>  
    </div>

  </div>

  <!-- clear history messagebox -->
  <MessageBox
    v-if="showClearHistoryMsgbox"
    :title="$t('msgbox.clear_search_history.title')"
    :message="`${$t('msgbox.clear_search_history.content')}`"
    :OkText="$t('msgbox.clear_search_history.ok')"
    :cancelText="$t('msgbox.cancel')"
    :warningOk="true"
    @ok="clearHistory()"
    @cancel="showClearHistoryMsgbox = false"
  />

</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue';
import { useI18n } from 'vue-i18n';
import { config, libConfig } from '@/common/config';
import { useUIStore } from '@/stores/uiStore';
import MessageBox from '@/components/MessageBox.vue';
import { IconMore, IconTrash, IconSearch } from '@/common/icons';
import ContextMenu from '@/components/ContextMenu.vue';
import SearchBox, { type SearchToken } from '@/components/SearchBox.vue';

const props = defineProps({
  titlebar: {
    type: String,
    required: true
  }
});

/// i18n
const { locale, messages } = useI18n();
const localeMsg = computed(() => messages.value[locale.value] as any);
const uiStore = useUIStore();

const showClearHistoryMsgbox = ref(false);

const searchPanelMenuItems = computed(() => [
  {
    label: localeMsg.value.menu.search.clear_history,
    icon: IconTrash,
    action: () => showClearConfirmation(),
  },
]);

function getSearchHistoryMenuItems(index: number) {
  return [
    {
      label: localeMsg.value.menu.home.delete,
      icon: IconTrash,
      action: () => {
        deleteHistoryItem(index);
      }
    },
  ];
}

// search query
const searchInputRef = ref<InstanceType<typeof SearchBox> | null>(null);
const searchQuery = ref('');
const activeTokens = ref<SearchToken[]>([]);

function restoreQuery(text: string, tokens?: SearchToken[]) {
  const restored = tokens?.length ? tokens : (text ? [{ type: 'raw', value: text }] : []);
  libConfig.search.searchText = text;
  libConfig.search.searchTokens = restored;
  activeTokens.value = restored.filter(token => token.type !== 'raw');
  searchQuery.value = restored.filter(token => token.type === 'raw').map(token => token.value).join(' ');
}

function syncSearchState() {
  if (libConfig.search.searchHistoryIndex !== -1) {
    const history = libConfig.search.searchHistory as any[];
    const item = history[libConfig.search.searchHistoryIndex];
    if (item) {
      const text = typeof item === 'string' ? item : item.text;
      restoreQuery(text, typeof item === 'string' ? undefined : item.tokens);
    }
  } else {
    restoreQuery(libConfig.search.searchText || '', libConfig.search.searchTokens);
  }
}

watch(
  () => libConfig.search.searchHistoryIndex,
  () => {
    syncSearchState();
  }
);

function focusSearchInput() {
  searchInputRef.value?.focusInput();
}

onMounted(() => {
  syncSearchState();
  nextTick(() => focusSearchInput());
});

onUnmounted(() => {
  uiStore.removeInputHandler('ImageSearch');
});

function handleSearchHistoryClick(index: number) {
  const item = libConfig.search.searchHistory[index];
  const searchText = typeof item === 'string' ? item : item?.text;
  if (searchText) {
    restoreQuery(searchText, typeof item === 'string' ? undefined : item.tokens);
    uiStore.requestCountUpdate({ source: 'search', text: searchText });
  }
  libConfig.search.searchHistoryIndex = index;
}

function clearHistory() {
  restoreQuery('');
  libConfig.search.searchHistory = [];
  libConfig.search.searchHistoryIndex = -1;

  showClearHistoryMsgbox.value = false;
}

function deleteHistoryItem(index: number) {
  if (index < 0 || index >= libConfig.search.searchHistory.length) return;
  const selectedIndex = libConfig.search.searchHistoryIndex;
  libConfig.search.searchHistory.splice(index, 1);
  if (selectedIndex === index) {
    restoreQuery('');
    libConfig.search.searchHistoryIndex = -1;
  } else if (selectedIndex > index) {
    libConfig.search.searchHistoryIndex = selectedIndex - 1;
  }
}

function handleSearch(tokens: SearchToken[]) {
  if (tokens.length === 0) {
    restoreQuery('');
    libConfig.search.searchHistoryIndex = -1;
    return;
  }
  const labels: Record<string, string> = { person: 'Person', object: 'Object', tag: 'Tag', text: 'Text (OCR)' };
  const query = tokens.map(token => token.type === 'raw'
    ? token.value : `${labels[token.type] || token.type}: ${token.value}`).join(' · ');
  const history = libConfig.search.searchHistory as any[];
  
  // Find existing index considering both string and object formats
  const existingIndex = history.findIndex((item: any) => {
    const text = typeof item === 'string' ? item : item.text;
    const savedTokens = typeof item === 'string' ? [{ type: 'raw', value: item }] : item.tokens || [{ type: 'raw', value: text }];
    return text === query && JSON.stringify(savedTokens) === JSON.stringify(tokens);
  });
  
  if (existingIndex !== -1) {
    libConfig.search.searchHistoryIndex = existingIndex;
  } else {
    // Add new item as object
    history.unshift({ text: query, tokens: tokens.map(token => ({ ...token })), count: null });
    libConfig.search.searchHistoryIndex = 0;

    // Limit the history size
    if (history.length > config.search.maxSearchHistory) {
      history.pop();
    }
  }

  libConfig.search.searchText = query;
  libConfig.search.searchTokens = tokens;
  uiStore.requestCountUpdate({ source: 'search', text: query });
}

const searchHistoryList = computed(() => libConfig.search.searchHistory as any[]);
const getSearchHistoryCount = (item: any) => {
  return Number(typeof item === 'string' ? 0 : item?.count || 0);
};
const hasSearchHistoryCount = (item: any) => getSearchHistoryCount(item) > 0;
const formatSearchResultCount = (count: number) => {
  return count.toLocaleString();
};

function showClearConfirmation() {
  showClearHistoryMsgbox.value = true;
}

defineExpose({
  clearHistory,
  showClearConfirmation,
  focusSearchInput,
});

</script>
