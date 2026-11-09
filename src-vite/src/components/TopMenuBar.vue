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

    <!-- Picture Menu -->
    <ContextMenu :menuItems="pictureMenuItems">
      <template #trigger="{ toggle }">
        <button
          type="button"
          data-testid="menu-picture"
          class="px-2 py-0.5 rounded hover:bg-base-content/10 hover:text-base-content cursor-pointer transition-colors"
          @click="toggle"
        >
          Picture
        </button>
      </template>
    </ContextMenu>

    <!-- Tools Menu -->
    <ContextMenu :menuItems="toolsMenuItems">
      <template #trigger="{ toggle }">
        <button
          type="button"
          data-testid="menu-tools"
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
import { useConfigStore } from '@/stores/configStore';
import { setShowHiddenPictures } from '@/common/api';
import { getShortcutLabel, getShortcutLabels, ShortcutActionId } from '@/common/shortcuts';

const config = useConfigStore();

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
    // Picasa's File > Export Picture to Folder (LAP-049): the selection, or the photo in focus
    label: 'Export Picture to Folder...',
    shortcut: 'Ctrl+Shift+S',
    action: () => {
      void tauriEmit('trigger-export-to-folder');
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
    shortcut: getShortcutLabel('file.invertSelection'),
    action: () => {
      void tauriEmit('trigger-invert-selection');
    }
  }
]);

// The same actions as the keys (LAP-101); Content runs them.
const shortcutItem = (label: string, actionId: ShortcutActionId) => ({
  label,
  shortcut: getShortcutLabels(actionId).find(item => /\d$/.test(item)) ?? getShortcutLabel(actionId),
  action: () => {
    void tauriEmit('trigger-shortcut', { actionId });
  }
});

const viewMenuItems = computed(() => [
  shortcutItem('Small Thumbnails', 'view.thumbnailsSmall'),
  shortcutItem('Normal Thumbnails', 'view.thumbnailsNormal'),
  shortcutItem('Edit Picture', 'file.editImage'),
  shortcutItem('Slideshow', 'slideshow.toggle'),
  shortcutItem('Timeline', 'view.timeline'),
  {
    label: '-',
    action: null
  },
  {
    label: 'Toggle Sidebar',
    action: () => {
      void tauriEmit('trigger-toggle-sidebar');
    }
  },
  {
    label: '-',
    action: null
  },
  {
    label: config.settings.showHiddenPictures ? '✓ Hidden Pictures' : 'Hidden Pictures',
    action: async () => {
      const next = !config.settings.showHiddenPictures;
      config.setShowHiddenPictures(next);
      await setShowHiddenPictures(next);
      void tauriEmit('trigger-toggle-hidden-pictures', { show: next });
    }
  }
]);

const pictureMenuItems = computed(() => [
  {
    label: 'Hide',
    action: () => {
      void tauriEmit('trigger-picture-hide');
    }
  },
  {
    label: 'Unhide',
    action: () => {
      void tauriEmit('trigger-picture-unhide');
    }
  }
]);

const toolsMenuItems = computed(() => [
  {
    label: 'AI acceleration...',
    action: () => {
      void tauriEmit('app-open-ai-acceleration');
    }
  },
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
    // Picasa's Tools > Adjust Date and Time (LAP-049): the selected photos, or the one in focus
    label: 'Adjust Date and Time...',
    action: () => {
      void tauriEmit('trigger-adjust-date-time');
    }
  },
  {
    // Picasa's Tools > Create Picture Collage (LAP-049): the selection, or the photo's folder
    label: 'Create Picture Collage...',
    action: () => {
      void tauriEmit('trigger-create-collage');
    }
  },
  {
    // Picasa's Tools > Make a Poster (LAP-049): the photo in focus, cut into printable sheets
    label: 'Make a Poster...',
    action: () => {
      void tauriEmit('trigger-make-poster');
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
