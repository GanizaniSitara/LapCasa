<template>
  <div class="flex items-center space-x-1 text-xs text-base-content/80 select-none ml-2" @mousedown.stop>
    <!-- File Menu -->
    <ContextMenu :menuItems="fileMenuItems">
      <template #trigger="{ toggle }">
        <button
          type="button"
          class="px-2 py-0.5 rounded hover:bg-base-content/10 hover:text-base-content cursor-pointer transition-colors"
          @click="toggle"
        >
          File
        </button>
      </template>
    </ContextMenu>

    <!-- Edit Menu -->
    <ContextMenu :menuItems="editMenuItems">
      <template #trigger="{ toggle }">
        <button
          type="button"
          class="px-2 py-0.5 rounded hover:bg-base-content/10 hover:text-base-content cursor-pointer transition-colors"
          @click="toggle"
        >
          Edit
        </button>
      </template>
    </ContextMenu>

    <!-- View Menu -->
    <ContextMenu :menuItems="viewMenuItems">
      <template #trigger="{ toggle }">
        <button
          type="button"
          class="px-2 py-0.5 rounded hover:bg-base-content/10 hover:text-base-content cursor-pointer transition-colors"
          @click="toggle"
        >
          View
        </button>
      </template>
    </ContextMenu>

    <!-- Tools Menu -->
    <ContextMenu :menuItems="toolsMenuItems">
      <template #trigger="{ toggle }">
        <button
          type="button"
          class="px-2 py-0.5 rounded hover:bg-base-content/10 hover:text-base-content cursor-pointer transition-colors font-medium text-base-content"
          @click="toggle"
        >
          Tools
        </button>
      </template>
    </ContextMenu>

    <!-- Help Menu -->
    <ContextMenu :menuItems="helpMenuItems">
      <template #trigger="{ toggle }">
        <button
          type="button"
          class="px-2 py-0.5 rounded hover:bg-base-content/10 hover:text-base-content cursor-pointer transition-colors"
          @click="toggle"
        >
          Help
        </button>
      </template>
    </ContextMenu>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue';
import { emit as tauriEmit } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import ContextMenu from '@/components/ContextMenu.vue';
import { IconFolder, IconSettings, IconUpdate } from '@/common/icons';

const fileMenuItems = computed(() => [
  {
    label: 'Add Folder to LapCasa...',
    icon: IconFolder,
    shortcut: 'Ctrl+O',
    action: () => {
      void tauriEmit('open-folder-manager', {});
    }
  },
  {
    label: '-',
    action: null
  },
  {
    label: 'Exit',
    action: () => {
      void getCurrentWindow().close();
    }
  }
]);

const editMenuItems = computed(() => [
  {
    label: 'Select All',
    shortcut: 'Ctrl+A',
    action: () => {
      void tauriEmit('trigger-select-all');
    }
  },
  {
    label: 'Deselect',
    shortcut: 'Ctrl+D',
    action: () => {
      void tauriEmit('trigger-deselect-all');
    }
  },
  {
    label: 'Invert Selection',
    action: () => {
      void tauriEmit('trigger-invert-selection');
    }
  }
]);

const viewMenuItems = computed(() => [
  {
    label: 'Toggle Sidebar',
    action: () => {
      void tauriEmit('trigger-toggle-sidebar');
    }
  }
]);

const toolsMenuItems = computed(() => [
  {
    label: 'Folder Manager...',
    icon: IconFolder,
    action: () => {
      void tauriEmit('open-folder-manager', {});
    }
  },
  {
    label: 'Scan all folders now',
    icon: IconUpdate,
    action: () => {
      void tauriEmit('trigger-scan-all-folders');
    }
  },
  {
    label: '-',
    action: null
  },
  {
    label: 'Options...',
    icon: IconSettings,
    action: () => {
      void tauriEmit('app-open-preferences');
    }
  }
]);

const helpMenuItems = computed(() => [
  {
    label: 'About LapCasa',
    action: () => {
      void tauriEmit('app-open-about');
    }
  }
]);
</script>
