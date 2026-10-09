<template>
  <div class="w-screen h-screen flex flex-col bg-base-300 text-base-content/70 overflow-hidden select-none">
    <!-- Title Bar -->
    <TitleBar titlebar="Folder Manager" :resizable="true" viewName="FolderManager" class="shrink-0 z-50" />

    <!-- Main Content -->
    <div class="flex-1 min-h-0 p-2 overflow-hidden flex flex-col bg-base-300">
      <FolderManager
        :isStandalone="true"
        :initialPath="initialPath"
        @ok="closeWindow"
        @cancel="closeWindow"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import TitleBar from '@/components/TitleBar.vue';
import FolderManager from '@/components/FolderManager.vue';

const appWindow = getCurrentWebviewWindow();
const initialPath = ref('');

const closeWindow = async () => {
  try {
    await appWindow.destroy();
  } catch (e) {
    console.error('Failed to close Folder Manager window:', e);
  }
};

onMounted(async () => {
  const params = new URLSearchParams(window.location.search);
  const path = params.get('path');
  if (path) {
    initialPath.value = path;
  }

  await appWindow.show();

  await appWindow.onCloseRequested(async (event) => {
    event.preventDefault();
    await appWindow.destroy();
  });
});
</script>
