<template>
  <!-- People view: one header per named person, then that person's face tiles -->
  <div
    class="w-full h-full px-2 flex items-center gap-3 border-t border-b select-none"
    :class="suggestionCount > 0 ? 'border-orange-300 bg-orange-50 text-gray-900' : 'border-base-content/10 bg-base-200'"
  >
    <div class="w-[60px] h-[60px] shrink-0 overflow-hidden bg-base-300/70 ring-2 ring-white shadow-[0_1px_4px_rgba(0,0,0,0.35)] flex items-center justify-center">
      <img v-if="avatar" :src="avatar" class="w-full h-full object-cover pointer-events-none" draggable="false" />
      <IconPerson v-else class="w-8 h-8 text-base-content/30" />
    </div>
    <div class="min-w-0 flex flex-col">
      <span class="truncate text-[1.7rem] leading-tight text-base-content/90" style="font-family: Georgia, 'Times New Roman', serif">{{ label }}</span>
      <span class="text-xs text-base-content/50 tabular-nums">{{ photoCount.toLocaleString() }} {{ photoCount === 1 ? 'photo' : 'photos' }}</span>
    </div>
    <div v-if="suggestionCount > 0" class="ml-auto flex flex-col items-end gap-0.5 shrink-0">
      <span class="text-[11px] text-orange-700 tabular-nums">
        {{ suggestionCount.toLocaleString() }} {{ suggestionCount === 1 ? 'suggestion' : 'suggestions' }}
      </span>
      <div class="flex items-center gap-1">
        <button
          type="button"
          class="btn btn-xs h-6 min-h-6 px-1.5 btn-ghost border border-base-content/15"
          title="Review this person's suggested matches only"
          @click.stop="reviewSuggestions"
        >
          <IconPerson class="w-4 h-4" />
          <span class="w-3.5 h-3.5 inline-flex items-center justify-center rounded-full bg-orange-500 text-white text-[9px] font-bold leading-none">?</span>
        </button>
        <button
          type="button"
          class="btn btn-xs h-7 min-h-7 px-3 border border-orange-600 bg-orange-600 text-white hover:bg-orange-700 hover:border-orange-700"
          :title="`Confirm every suggested face for ${label}`"
          @click.stop="resolveSuggestions(true, null, personId)"
        >Confirm all</button>
        <button type="button" class="btn btn-xs h-6 min-h-6 btn-ghost" title="Ignore every suggested face for this person" @click.stop="removeAll">Ignore all</button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { ask } from '@tauri-apps/plugin-dialog';
import { getPersonThumbnail } from '@/common/api';
import { IconPerson } from '@/common/icons';
import { ensurePersonFaces, personFaces, resolveSuggestions, stateOf } from '@/common/personFaces';

const props = defineProps<{
  personId: number;
  label: string;
  // (person, file) rows in this section, as listed by the backend.
  count: number;
}>();

// Avatars are shared by every header of the view; fetch each once.
const avatarCache = (globalThis as any).__lapcasaPersonAvatars ??= new Map<number, Promise<string>>();
const avatar = ref('');

watch(() => props.personId, async (id) => {
  avatar.value = '';
  if (!id || id < 0) return;
  ensurePersonFaces(id);
  if (!avatarCache.has(id)) {
    avatarCache.set(id, (getPersonThumbnail(id) as Promise<string | null>)
      .then(data => (data ? `data:image/jpeg;base64,${data}` : ''))
      .catch(() => ''));
  }
  const src = await avatarCache.get(id)!;
  if (props.personId === id) avatar.value = src;
}, { immediate: true });

const loaded = computed(() => personFaces.people.has(props.personId));
const suggestionCount = computed(() => stateOf(props.personId).suggestions.size);
const photoCount = computed(() => (props.personId > 0 && loaded.value ? stateOf(props.personId).confirmed : props.count));

// The "?" button: show only this person's pending suggestions.
function reviewSuggestions() {
  window.dispatchEvent(new CustomEvent('lapcasa-person-review', {
    detail: { personId: props.personId, name: props.label },
  }));
}

async function removeAll() {
  const n = suggestionCount.value;
  const ok = await ask(
    `Ignore all ${n.toLocaleString()} suggested ${n === 1 ? 'face' : 'faces'} for ${props.label}? You can restore them later from the person's Ignored list.`,
    { title: 'Ignore all suggestions', kind: 'warning', okLabel: 'Ignore all', cancelLabel: 'Cancel' },
  );
  if (ok) void resolveSuggestions(false, null, props.personId);
}
</script>
