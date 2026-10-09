<template>
  <div
    ref="containerRef"
    class="group relative shrink-0 transition-all duration-200"
    :class="[
      alwaysExpanded ? 'w-full min-w-0' : isExpanded ? 'min-w-[14rem] max-w-md w-auto' : 'w-8',
    ]"
  >
    <!-- Search Bar Capsule / Container -->
    <div
      class="flex items-center min-h-[28px] h-7 px-1.5 rounded border transition-colors duration-150 cursor-text select-none"
      :class="[
        isFocused
          ? 'border-primary/80 bg-base-100 shadow-xs'
          : (tokens.length > 0 || inputValue.length > 0)
            ? 'border-base-content/20 bg-base-100/60 hover:border-base-content/40'
            : 'border-transparent hover:bg-base-100/40 justify-center'
      ]"
      @click="focusInput"
    >
      <!-- Search Icon -->
      <IconSearch
        class="w-3.5 h-3.5 shrink-0 transition-colors cursor-pointer"
        :class="[
          isFocused ? 'text-primary' : 'text-base-content/40 group-hover:text-base-content/80',
          isExpanded ? 'mr-1' : ''
        ]"
        @click.stop="focusInput"
      />

      <!-- Active Chips list -->
      <div
        v-if="isExpanded && tokens.length > 0"
        class="flex items-center gap-1 min-w-0 flex-nowrap overflow-x-auto no-scrollbar max-w-[65%] mr-1"
      >
        <div
          v-for="(token, index) in tokens"
          :key="`${token.type}-${token.value}-${index}`"
          class="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[11px] leading-tight select-none border shrink-0 transition-all"
          :class="getChipClass(token.type)"
        >
          <span class="text-xs shrink-0 leading-none">{{ getChipIcon(token.type) }}</span>
          <span class="font-medium opacity-70 shrink-0">{{ getChipLabel(token.type) }}:</span>
          <span class="font-semibold truncate max-w-[90px]">{{ token.value }}</span>
          <button
            type="button"
            class="ml-0.5 rounded-full hover:bg-base-content/20 p-0.5 text-base-content/50 hover:text-base-content cursor-pointer transition-colors"
            :title="`Remove ${token.value}`"
            @click.stop="removeToken(index)"
          >
            <IconClose class="w-2.5 h-2.5" />
          </button>
        </div>
      </div>

      <!-- Input element -->
      <input
        v-show="isExpanded"
        ref="searchInputRef"
        tabindex="0"
        type="text"
        aria-label="Search photos"
        role="combobox"
        :aria-expanded="showDropdown && facetOptions.length > 0"
        v-model="inputValue"
        maxlength="255"
        :placeholder="tokens.length === 0 ? $t('search.image_search_placeholder') : '+ filter...'"
        class="flex-1 min-w-[50px] bg-transparent border-none outline-none focus:outline-none focus:ring-0 text-xs p-0 text-base-content placeholder-base-content/30 cursor-text h-full"
        @focus="handleFocus"
        @blur="handleBlur"
        @input="handleInput"
        @keydown="handleInputKeyDown"
      />

      <!-- Cancel / Clear button -->
      <button
        v-if="isExpanded && (tokens.length > 0 || inputValue.length > 0)"
        type="button"
        class="ml-1 p-0.5 rounded text-base-content/40 hover:text-base-content hover:bg-base-content/10 cursor-pointer shrink-0 transition-colors"
        title="Clear search"
        @click.stop="clickCancel"
      >
        <IconClose class="w-3.5 h-3.5" />
      </button>
    </div>

    <!-- Dropdown underneath input -->
    <div
      v-if="showDropdown && facetOptions.length > 0"
      role="listbox"
      class="absolute left-0 right-0 top-full mt-1 z-50 bg-base-100 border border-base-content/15 rounded shadow-xl py-0.5 overflow-hidden text-xs"
    >
      <div
        v-for="(option, idx) in facetOptions"
        role="option"
        :aria-selected="selectedIndex === idx"
        :key="option.type"
        class="flex items-center gap-1.5 px-2 py-1 cursor-pointer select-none transition-colors"
        :class="[
          selectedIndex === idx
            ? 'bg-primary/20 text-primary font-medium'
            : 'hover:bg-base-200/80 text-base-content/80'
        ]"
        @mousedown.prevent="selectFacet(option)"
        @mouseenter="selectedIndex = idx"
      >
        <span class="text-sm shrink-0 leading-none">{{ option.icon }}</span>
        <span class="font-medium shrink-0">{{ option.label }}:</span>
        <span class="truncate font-semibold text-base-content">"{{ option.value }}"</span>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick, type PropType } from 'vue';
import { IconClose, IconSearch } from '@/common/icons';
import { listen } from '@tauri-apps/api/event';
import { useUIStore } from '@/stores/uiStore';

export interface SearchToken {
  type: 'person' | 'tag' | 'object' | 'text' | 'ocr' | 'raw' | string;
  value: string;
}

interface FacetOption {
  type: 'person' | 'tag' | 'object' | 'text';
  label: string;
  icon: string;
  value: string;
}

const props = defineProps({
  alwaysExpanded: {
    type: Boolean,
    default: false,
  },
  modelValue: {
    type: String,
    default: '',
  },
  initialTokens: {
    type: Array as PropType<SearchToken[]>,
    default: () => [],
  },
});

const emit = defineEmits([
  'update:modelValue',
  'search-tokens',
  'search',
]);

const uiStore = useUIStore();

const containerRef = ref<HTMLElement | null>(null);
const searchInputRef = ref<HTMLInputElement | null>(null);

const inputValue = ref(props.modelValue || '');
const tokens = ref<SearchToken[]>(props.initialTokens ? [...props.initialTokens] : []);
const isFocused = ref(false);
const showDropdown = ref(false);
const selectedIndex = ref(-1);

let unlistenKeydown: () => void;
let lastSubmitTime = 0;

const isExpanded = computed(() => {
  return props.alwaysExpanded || isFocused.value || tokens.value.length > 0 || inputValue.value.length > 0;
});

const facetOptions = computed<FacetOption[]>(() => {
  const query = inputValue.value.trim();
  if (!query) return [];
  return [
    { type: 'person', label: 'Person', icon: '🧑', value: query },
    { type: 'tag', label: 'Tag', icon: '🏷️', value: query },
    { type: 'object', label: 'Object', icon: '🖼️', value: query },
    { type: 'text', label: 'Text (OCR)', icon: '📝', value: query },
  ];
});

watch(() => props.modelValue, (newVal) => {
  if (newVal !== undefined && newVal !== inputValue.value) {
    inputValue.value = newVal || '';
  }
});

watch(() => props.initialTokens, (newTokens) => {
  if (newTokens) {
    tokens.value = [...newTokens];
  }
});

watch(inputValue, (newVal) => {
  if (newVal && newVal.trim().length > 0 && isFocused.value) {
    showDropdown.value = true;
    selectedIndex.value = -1;
  } else {
    showDropdown.value = false;
    selectedIndex.value = -1;
  }
});

onMounted(async () => {
  unlistenKeydown = await listen('global-keydown', handleGlobalKeyDown);
  document.addEventListener('pointerdown', handleClickOutside);
});

onUnmounted(() => {
  if (unlistenKeydown) {
    unlistenKeydown();
  }
  document.removeEventListener('pointerdown', handleClickOutside);
});

const handleClickOutside = (event: MouseEvent) => {
  if (containerRef.value && !containerRef.value.contains(event.target as Node)) {
    showDropdown.value = false;
  }
};

const getChipClass = (type: string) => {
  switch (type) {
    case 'person':
      return 'bg-sky-500/15 text-sky-700 dark:text-sky-300 border-sky-500/30';
    case 'tag':
      return 'bg-emerald-500/15 text-emerald-700 dark:text-emerald-300 border-emerald-500/30';
    case 'object':
      return 'bg-amber-500/15 text-amber-700 dark:text-amber-300 border-amber-500/30';
    case 'text':
    case 'ocr':
      return 'bg-purple-500/15 text-purple-700 dark:text-purple-300 border-purple-500/30';
    case 'raw':
    default:
      return 'bg-base-200 text-base-content border-base-content/20';
  }
};

const getChipIcon = (type: string) => {
  switch (type) {
    case 'person': return '🧑';
    case 'tag': return '🏷️';
    case 'object': return '🖼️';
    case 'text':
    case 'ocr': return '📝';
    default: return '🔍';
  }
};

const getChipLabel = (type: string) => {
  switch (type) {
    case 'person': return 'Person';
    case 'tag': return 'Tag';
    case 'object': return 'Object';
    case 'text':
    case 'ocr': return 'Text (OCR)';
    case 'raw': return 'Raw';
    default: return type;
  }
};

const focusInput = () => {
  isFocused.value = true;
  nextTick(() => {
    searchInputRef.value?.focus();
  });
};

const handleFocus = () => {
  isFocused.value = true;
  uiStore.pushInputHandler('SearchBox');
  if (inputValue.value.trim().length > 0) {
    showDropdown.value = true;
  }
};

const handleBlur = () => {
  setTimeout(() => {
    if (!containerRef.value?.contains(document.activeElement)) {
      isFocused.value = false;
      showDropdown.value = false;
      uiStore.removeInputHandler('SearchBox');
    }
  }, 150);
};

const handleInput = () => {
  if (inputValue.value.trim().length > 0) {
    showDropdown.value = true;
  } else {
    showDropdown.value = false;
  }
};

const selectFacet = (option: FacetOption) => {
  tokens.value.push({
    type: option.type,
    value: option.value,
  });
  inputValue.value = '';
  showDropdown.value = false;
  selectedIndex.value = -1;
  focusInput();
  emitSearchTokens();
};

const removeToken = (index: number) => {
  if (index >= 0 && index < tokens.value.length) {
    tokens.value.splice(index, 1);
    focusInput();
    emitSearchTokens();
  }
};

const emitSearchTokens = () => {
  const payload: SearchToken[] = tokens.value.map(t => ({
    type: t.type,
    value: t.value,
  }));
  const raw = inputValue.value.trim();
  if (raw.length > 0) {
    payload.push({
      type: 'raw',
      value: raw,
    });
  }

  emit('search-tokens', payload);
  emit('update:modelValue', raw);
  emit('search', payload);
};

const handleSearchSubmit = () => {
  const now = Date.now();
  if (now - lastSubmitTime < 100) return;
  lastSubmitTime = now;

  showDropdown.value = false;
  selectedIndex.value = -1;
  emitSearchTokens();
  searchInputRef.value?.blur();
  isFocused.value = false;
};

const handleInputKeyDown = (event: KeyboardEvent) => {
  if (event.key === 'ArrowDown') {
    if (showDropdown.value && facetOptions.value.length > 0) {
      event.preventDefault();
      selectedIndex.value = (selectedIndex.value + 1) % facetOptions.value.length;
    }
  } else if (event.key === 'ArrowUp') {
    if (showDropdown.value && facetOptions.value.length > 0) {
      event.preventDefault();
      selectedIndex.value = selectedIndex.value <= 0
        ? facetOptions.value.length - 1
        : selectedIndex.value - 1;
    }
  } else if (event.key === 'Enter') {
    event.preventDefault();
    if (showDropdown.value && selectedIndex.value >= 0 && selectedIndex.value < facetOptions.value.length) {
      selectFacet(facetOptions.value[selectedIndex.value]);
    } else {
      handleSearchSubmit();
    }
  } else if (event.key === 'Escape') {
    event.preventDefault();
    if (showDropdown.value) {
      showDropdown.value = false;
      selectedIndex.value = -1;
    } else if (isFocused.value) {
      clickCancel();
    }
  } else if (event.key === 'Backspace') {
    if (inputValue.value.length === 0 && tokens.value.length > 0) {
      tokens.value.pop();
      emitSearchTokens();
    }
  }
};

function handleGlobalKeyDown(event: any) {
  if (!uiStore.isInputActive('SearchBox')) return;

  const { key } = event.payload;
  switch (key) {
    case 'Enter':
      if (showDropdown.value && selectedIndex.value >= 0 && selectedIndex.value < facetOptions.value.length) {
        selectFacet(facetOptions.value[selectedIndex.value]);
      } else {
        handleSearchSubmit();
      }
      break;
    case 'Escape':
      if (showDropdown.value) {
        showDropdown.value = false;
        selectedIndex.value = -1;
      } else if (isFocused.value) {
        clickCancel();
      }
      break;
    default:
      break;
  }
}

const clickCancel = () => {
  inputValue.value = '';
  tokens.value = [];
  showDropdown.value = false;
  selectedIndex.value = -1;
  emitSearchTokens();
  searchInputRef.value?.blur();
  isFocused.value = false;
};

defineExpose({
  focusInput,
  isFocused,
  tokens,
  clearTokens: () => {
    tokens.value = [];
    emitSearchTokens();
  },
  setTokens: (newTokens: SearchToken[]) => {
    tokens.value = [...newTokens];
    emitSearchTokens();
  },
});
</script>
