<template>

  <div ref="editorRoot" tabindex="-1" class="flex flex-col overflow-hidden outline-none bg-base-300 text-base-content/70"
    :class="embedded ? 'w-full h-full' : 'w-screen h-screen'"
    :data-inline-editor="embedded ? true : undefined" :data-file-id="fileInfo?.id">
    <div v-if="embedded" class="h-9 shrink-0 flex items-center gap-2 px-3">
      <button class="btn btn-sm btn-ghost" @click="clickCancel">Back to Library</button>
      <button class="btn btn-sm btn-ghost" aria-label="Previous photo" :disabled="folderIndex <= 0" @click="navigatePhoto(-1)">←</button>
      <button class="btn btn-sm btn-ghost" aria-label="Next photo" :disabled="folderIndex >= folderFiles.length - 1" @click="navigatePhoto(1)">→</button>
      <span class="truncate">{{ fileInfo?.name }}</span>
    </div>
    <!-- Title Bar -->
    <TitleBar
      v-if="!embedded && showDesktopTitleBar"
      :titlebar="`${$t('msgbox.image_editor.title')} - ${shortenFilename(fileInfo?.name || '', 32)}`"
      :resizable="false"
      viewName="ImageEditor"
      class="shrink-0 z-50"
    />
    <div
      v-else-if="!embedded"
      class="h-10 shrink-0 flex items-center justify-center px-20 select-none"
      data-tauri-drag-region
    >
      <div class="min-w-0 max-w-full text-center text-sm font-medium text-base-content/70 truncate" data-tauri-drag-region>
        {{ $t('msgbox.image_editor.title') }} - {{ shortenFilename(fileInfo?.name || '', 32) }}
      </div>
    </div>

    <!-- Main Content -->
    <div v-if="fileInfo" class="flex-1 flex gap-3 p-3 min-h-0 select-none">
      <!-- Left: Image Preview -->
      <div class="flex-1 min-w-0 flex flex-col items-center justify-center gap-2">
        <div
          ref="containerRef"
          class="relative w-full flex-1 rounded-box overflow-hidden border border-base-content/5 bg-base-300/30 shadow-sm cursor-default"
          @pointerdown="handlePreviewPointerDown"
          @pointerup="handlePreviewPointerUp"
          @pointerleave="handlePreviewPointerUp"
          @pointercancel="handlePreviewPointerUp"
        >
            <transition name="fade">
              <div v-if="isProcessing" class="absolute inset-0 z-50 flex items-center justify-center bg-base-100/55 backdrop-blur-sm">
                <span class="loading loading-dots text-primary"></span>
              </div>
            </transition>

            <template v-if="imageSrc">
              <figure
                v-if="showDiffPreview && canShowDiffPreview"
                class="diff absolute inset-0 z-20 h-full w-full"
                tabindex="0"
              >
                <div class="diff-item-1 relative h-full w-full">
                  <img
                    :src="imageSrc"
                    :style="originalImageStyle"
                    class="block"
                    draggable="false"
                  />
                </div>
                <div class="diff-item-2 relative h-full w-full">
                  <img
                    :src="displayImageSrc || undefined"
                    :style="adjustedImageStyle"
                    class="block"
                    draggable="false"
                  />
                </div>
                <div class="diff-resizer"></div>
              </figure>

              <div
                v-if="showDiffPreview && canShowDiffPreview"
                class="pointer-events-none absolute z-50 rounded-box bg-base-100/80 px-2 py-1 text-[10px] font-semibold uppercase tracking-wide text-base-content/70 left-3 top-3"
              >
                {{ $t('msgbox.image_editor.original') }}
              </div>
              <div
                v-if="showDiffPreview && canShowDiffPreview"
                class="pointer-events-none absolute z-50 rounded-box bg-base-100/80 px-2 py-1 text-[10px] font-semibold uppercase tracking-wide text-base-content/70 right-3 top-3"
              >
                {{ currentPresetLabel || $t('msgbox.image_editor.adjusted') }}
              </div>

              <img
                v-show="imageReady && !(showDiffPreview && canShowDiffPreview)"
                ref="imageRef"
                :key="fileInfo.id"
                crossorigin="anonymous"
                :src="displayImageSrc || undefined"
                :style="imageStyle"
                class="block"
                draggable="false"
                @load="onImageLoad"
                @click="pickEditorPoint"
              />
            </template>

            <div v-if="cropStatus === 1 || cropApplied"
              :class="[
                cropStatus === 1 ? 'crop-box-active' : 'crop-box-done',
                isResizing ? 'no-transition' : '',
                cropStatus === 1
                  ? (
                    cropBoxFixed
                      ? (isDragging ? 'cursor-grabbing no-transition' : 'cursor-grab')
                      : (isDragging ? 'cursor-move no-transition' : 'cursor-move')
                  )
                  : ''
              ]"
              :style="[
                cropBoxStyle,
                activeEditorTab === 'adjust' ? { pointerEvents: 'none', zIndex: 30 } : { zIndex: 40 }
              ]"
              @mousedown="cropStatus===1 ? startDrag('move', $event) : null"
              @dblclick="clickDoCrop"
            >
              <template v-if="cropStatus===1 && (isDragging || isRotating)">
                <div v-if="isDragging" class="crop-dimensions-display">
                  {{ crop.width }} x {{ crop.height }}
                </div>
                <div class="grid-lines">
                  <div class="grid-line-h grid-line-h-1"></div>
                  <div class="grid-line-h grid-line-h-2"></div>
                  <div class="grid-line-v grid-line-v-1"></div>
                  <div class="grid-line-v grid-line-v-2"></div>
                </div>
              </template>
              <template v-if="cropStatus===1 && !cropBoxFixed">
                <div class="drag-handle top-left" @mousedown.stop="startDrag('top-left', $event)"></div>
                <div class="drag-handle top" @mousedown.stop="startDrag('top', $event)"></div>
                <div class="drag-handle top-right" @mousedown.stop="startDrag('top-right', $event)"></div>
                <div class="drag-handle left" @mousedown.stop="startDrag('left', $event)"></div>
                <div class="drag-handle right" @mousedown.stop="startDrag('right', $event)"></div>
                <div class="drag-handle bottom-left" @mousedown.stop="startDrag('bottom-left', $event)"></div>
                <div class="drag-handle bottom" @mousedown.stop="startDrag('bottom', $event)"></div>
                <div class="drag-handle bottom-right" @mousedown.stop="startDrag('bottom-right', $event)"></div>
              </template>
            </div>
        </div>

        <!-- straighten angle + crop confirm/cancel -->
        <div
          v-if="cropStatus === 1"
          class="w-full max-w-2xl shrink-0 flex items-center gap-3 px-4"
        >
          <div
            class="flex items-center gap-3 flex-1 min-w-0"
            @pointerdown="isRotating = true"
            @pointerup="isRotating = false"
            @pointercancel="isRotating = false"
          >
            <span class="text-xs font-medium text-base-content/50 shrink-0">{{ $t('msgbox.image_editor.angle') }}</span>
            <AngleSlider
              v-model="cropAngleModel"
              :min="-45"
              :max="45"
              :step="1"
              class="flex-1 min-w-0"
            />
          </div>

          <div class="w-px h-5 bg-base-content/10 shrink-0"></div>

          <div class="flex items-center gap-1 shrink-0">
            <TButton
              buttonSize="small"
              :icon="IconClose"
              :tooltip="$t('msgbox.image_editor.cancel_crop')"
              @click="clickCancelCrop"
            />
            <TButton
              buttonSize="small"
              :icon="IconOk"
              :selected="true"
              :tooltip="$t('msgbox.image_editor.confirm_crop')"
              @click="clickDoCrop"
            />
          </div>
        </div>

      </div>

      <aside class="order-first w-[278px] shrink-0 flex flex-col min-h-0 text-[11px]" data-testid="photo-editor">
        <div role="tablist" class="flex border-b border-base-content/15">
          <button v-for="tab in editorTabs" :key="tab.key" role="tab" :aria-selected="editorTab===tab.key"
            class="flex-1 px-1 py-2 border border-base-content/10 rounded-t text-[11px]"
            :class="editorTab===tab.key ? 'bg-base-100 font-bold' : 'bg-base-300'"
            :disabled="cropStatus===1 || isProcessing" @click="editorTab=tab.key">{{ tab.label }}</button>
        </div>
        <div class="flex-1 min-h-0 overflow-y-auto p-2" :class="isProcessing ? 'pointer-events-none opacity-60' : ''">
          <template v-if="!activeTool">
            <template v-if="editorTab==='basic'">
              <div class="grid grid-cols-3 gap-x-1 gap-y-3 py-2">
                <button v-for="tool in basicTools" :key="tool" class="flex flex-col items-center gap-1 hover:bg-base-100 rounded py-1"
                  :data-editor-tool="tool" :disabled="cropStatus===1 || !imageReady" @click="selectEditTool(tool)">
                  <span class="w-12 h-8 flex items-center justify-center text-base-content/70">
                    <span v-if="tool==='Text'" class="font-serif font-bold text-3xl">T</span>
                    <component v-else :is="basicToolIcons[tool]" class="w-7 h-7" />
                  </span>
                  <span class="leading-3 text-center">{{ tool }}</span>
                </button>
              </div>
              <label class="block text-center mt-3">Fill Light
                <input aria-label="Fill Light" type="range" min="0" max="100" v-model.number="editTools.fillLight"
                  class="range range-xs w-48 mt-1" @input="editTools.lastStep='Fill Light'" />
              </label>
            </template>
            <template v-else-if="editorTab==='tuning'">
              <label v-for="control in tuningTools" :key="control.key" class="block text-center my-4">{{ control.label }}
                <input :aria-label="control.label" type="range" :min="control.min" max="100" v-model.number="editTools[control.key]"
                  :title="control.key==='highlights' ? 'Brightens highlights or darkens blown areas; detail lost at capture cannot be restored.' : undefined"
                  class="range range-xs w-48 mt-1" @input="editTools.lastStep=control.label" />
              </label>
              <div class="text-center mt-3">Neutral Color Picker<br />
                <button class="btn btn-sm btn-ghost mt-1" :class="activeTool==='Neutral Color Picker' ? 'btn-active' : ''"
                  @click="activeTool='Neutral Color Picker'">Pick a neutral color</button>
              </div>
            </template>
            <div v-else class="grid grid-cols-3 gap-1 py-2">
              <button v-for="effect in effectTools" :key="effect" class="border border-base-content/15 p-1 hover:bg-base-100 rounded-sm"
                :data-editor-tool="effect" :disabled="!imageReady" @click="selectEditTool(effect)">
                <img :src="imageSrc" class="w-full h-12 object-cover" :style="{filter:toolPreview(effect)}" />
                <span class="block leading-3 mt-1">{{ effect }}</span>
              </button>
            </div>
          </template>
          <div v-else class="space-y-3 py-2">
            <h3 class="font-bold text-center text-sm">{{ activeTool }}</h3>
            <template v-if="activeTool==='Crop' || activeTool==='Straighten'">
              <label class="block">Crop shape
                <select v-model="config.imageEditor.cropShape" class="select select-xs w-full" @change="onChangeCropShape">
                  <option v-for="option in cropShapeOptions" :key="option.value" :value="option.value">{{ option.label }}</option>
                </select>
              </label>
              <p>Drag the frame to crop. Use the angle slider to straighten.</p>
              <button class="btn btn-sm" @click="clickDoCrop();activeTool=''">Apply</button>
              <button class="btn btn-sm btn-ghost" @click="restoreEditorControls();activeTool=''">Cancel</button>
            </template>
            <template v-else-if="activeTool==='Retouch'">
              <p>{{ retouchTarget ? 'Click an area to copy over the blemish.' : 'Click the blemish you want to retouch.' }}</p>
              <label>Brush size<input aria-label="Brush size" type="range" v-model.number="brushSize" min="1" max="20" class="range range-xs" /></label>
              <button class="btn btn-sm" :disabled="!retouchSource" @click="applyRetouch">Apply</button>
              <button class="btn btn-sm btn-ghost" @click="activeTool=''">Cancel</button>
            </template>
            <template v-else-if="activeTool==='Text'">
              <p>Type your text, then click the photo to place it.</p>
              <input aria-label="Text" v-model="toolText" class="input input-sm w-full" />
              <label>Font<select aria-label="Font" v-model="toolFont" class="select select-sm w-full"><option>Arial</option><option>Helvetica</option><option>Consolas</option></select></label>
              <label>Size<input aria-label="Text size" v-model.number="textSize" type="number" min="12" max="96" class="input input-sm w-16" /></label>
              <label>Color<input aria-label="Text color" v-model="effectColor" type="color" /></label>
              <button class="btn btn-sm" :disabled="!toolText" @click="applyText">Apply</button>
              <button class="btn btn-sm btn-ghost" @click="activeTool=''">Cancel</button>
            </template>
            <template v-else-if="activeTool==='Neutral Color Picker'">
              <p>Click a white or gray area of the photo.</p>
              <button class="btn btn-sm btn-ghost" @click="activeTool=''">Cancel</button>
            </template>
            <template v-else>
              <label>Amount<input aria-label="Effect amount" v-model.number="effectAmount" type="range" min="0" max="100" class="range range-xs w-full" /></label>
              <label v-if="['Tint','Filtered B&W','Graduated Tint'].includes(activeTool)">Color<input aria-label="Effect color" v-model="effectColor" type="color" /></label>
              <p v-if="['Soft Focus','Focal B&W'].includes(activeTool)">Click the photo to choose the center.</p>
              <button class="btn btn-sm" @click="applyEditEffect">Apply</button>
              <button class="btn btn-sm btn-ghost" @click="activeTool=''">Cancel</button>
            </template>
          </div>
        </div>
        <div class="shrink-0 px-2 pb-2">
          <div class="flex gap-1 mt-3">
            <button class="btn btn-sm flex-1 text-[11px] px-1" :disabled="!savedEditState.history?.length || isProcessing"
              data-testid="editor-undo" @click="queueHistory('undo')">{{ savedEditState.history?.length ? `Undo ${historyStepName(savedEditState)}` : 'Undo' }}</button>
            <button class="btn btn-sm flex-1 text-[11px] px-1" :disabled="!savedEditState.redo?.length || isProcessing"
              data-testid="editor-redo" @click="queueHistory('redo')">{{ savedEditState.redo?.length ? `Redo ${historyStepName(savedEditState.redo.at(-1))}` : 'Redo' }}</button>
          </div>
          <button class="btn btn-sm btn-ghost w-full mt-1" :disabled="(!savedEditState.recipe && !savedEditState.auto_enhance) || isProcessing"
            @click="saveRecipe('reset')">Undo All Edits</button>
          <p v-if="historyError" role="alert" class="text-error">{{ historyError }}</p>
        </div>
        <div class="shrink-0 p-2 mt-2 border border-base-content/10 rounded" data-editor-histogram>
          <ImageHistogram v-if="imageSrc" editor ref="histogramRef" :source="histogramSource" :adjustments="histogramAdjustments" :apply-adjustments="true" :crop="histogramCrop" :rotate="displayRotate" :flip-horizontal="isFlippedX" :flip-vertical="isFlippedY" />
          <PointCurves :model-value="editTools.curves" :disabled="!imageReady || isProcessing || cropStatus===1"
            @preview="curveDraft=$event" @change="commitCurves" />
          <p class="mt-1">{{ fileInfo?.e_model }} · {{ imageWidth }} × {{ imageHeight }}</p>
        </div>
      </aside>
    </div>

    <div v-if="embedded" data-editor-filmstrip class="h-24 shrink-0 px-3">
      <VirtualScroll ref="editorStrip" :items="folderFiles" :item-size="96" :grid-items="1"
        direction="horizontal" :buffer="3" :emit-update="true" @update="hydrateFilmstrip">
        <template #default="{ item }">
          <button class="w-[88px] h-[88px] p-1 border-2 rounded-[2px] overflow-hidden"
            :class="item.id === fileInfo?.id ? 'border-primary' : 'border-transparent'"
            :data-file-id="item.id" :title="item.name || ''" @click="queuePhoto(item.id)">
            <img v-if="item.file_path" :src="getThumbUrl(item.id, false, 0, item.modified_at, item.id === fileInfo?.id ? fileInfo.edits : item.edits)"
              class="w-full h-full object-contain" draggable="false" alt="" />
          </button>
        </template>
      </VirtualScroll>
    </div>

    <!-- Bottom Bar -->
    <div v-if="fileInfo" class="h-14 shrink-0 flex items-center justify-end px-4 gap-2">
      <button
        v-if="!embedded"
        class="px-4 py-1 rounded-box hover:bg-base-100 hover:text-base-content cursor-pointer text-sm mr-4"
        @click="clickCancel"
      >Back to Library</button>
      <TButton :icon="IconRotateLeft" :disabled="cropStatus===1 || cropApplied" tooltip="Rotate left" @click="clickRotate(-90)" />
      <TButton :icon="IconRotateRight" :disabled="cropStatus===1 || cropApplied" tooltip="Rotate right" data-editor-tool="Rotate right" @click="clickRotate(90)" />
      <template v-if="fileInfo">
        <select v-model="combinedFormatKey" class="select select-bordered select-xs" :disabled="cropStatus===1">
          <option v-for="option in combinedFormatOptions" :key="option.value" :value="option.value">{{ option.label }}</option>
        </select>
      </template>

        <div class="join">
          <button
            class="btn btn-sm btn-primary join-item px-4"
            :disabled="cropStatus === 1 || isProcessing"
            @click="clickExport"
          >Save a Copy</button>
        </div>
    </div>
  </div>


</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, nextTick, watch, type CSSProperties } from 'vue';
import { useRouter } from 'vue-router';
import { useUIStore } from '@/stores/uiStore';
import { useI18n } from 'vue-i18n';
import { config } from '@/common/config';
import { isWin, isLinux, setTheme, SCALE_VALUES, getFolderPath, getFileExtension, shortenFilename, getFullPath, combineFileName, getSelectOptions, getAssetSrc, getPreviewUrl, getThumbUrl, shouldUseBackendPreview, isEditableTarget, editHistoryAction } from '@/common/utils';
import { editImage, checkFileExists, getFileInfo, getQueryFileIds, getFilesByIds, saveImageRecipe, restoreImageRecipe } from '@/common/api';
import { getMaxInscribedRect, clampCenterInRotatedRect, maxResizeT } from '@/common/geometry';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { emit as tauriEmit, listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { ask as askDialog } from '@tauri-apps/plugin-dialog';

import TitleBar from '@/components/TitleBar.vue';
import TButton from '@/components/TButton.vue';
import SliderInput from '@/components/SliderInput.vue';
import AngleSlider from '@/components/AngleSlider.vue';
import ImageHistogram from '@/components/ImageHistogram.vue';
import PointCurves from '@/components/PointCurves.vue';
import { applyCurves, identityCurves, type Curves, type CurveChannel } from '@/common/curves';
import VirtualScroll from '@/components/VirtualScroll.vue';

import {
  IconCrop,
  IconCropLandscape,
  IconZoomIn,
  IconZoomOut,
  IconRotateLeft,
  IconRotateRight,
  IconFlipVertical,
  IconFlipHorizontal,
  IconClose,
  IconOk,
  IconRestore,
  IconLink,
  IconLinkOff,
  IconPalette,
  IconWand,
  IconUnhide,
  IconAdjustments,
  IconEdit,
  IconSplitOn,
} from '@/common/icons';

const props = withDefaults(defineProps<{ embedded?: boolean; initialFile?: any; folderQuery?: any }>(), { embedded: false });
const emit = defineEmits(['back']);
const router = useRouter();
const fileInfo = ref<any>(props.initialFile ? { ...props.initialFile } : null);
const folderFiles = ref<any[]>(props.initialFile ? [{ ...props.initialFile }] : []);
const folderIndex = computed(() => folderFiles.value.findIndex(file => file.id === fileInfo.value?.id));
const editorStrip = ref<any>(null);
const editorRoot = ref<HTMLElement | null>(null);
let editorUnmounted = false;
const fetchingStripIds = new Set<number>();
let photoNavigation: Promise<void> = Promise.resolve();

async function hydrateFilmstrip(start: number, end: number) {
  const ids = folderFiles.value.slice(start, end + 1).filter(file => !file.file_path && !fetchingStripIds.has(file.id)).map(file => file.id);
  if (!ids.length) return;
  ids.forEach(id => fetchingStripIds.add(id));
  try {
    const files = await getFilesByIds(ids);
    if (editorUnmounted) return;
    const byId = new Map((files || []).map((file: any) => [file.id, file]));
    for (let i = start; i <= end && i < folderFiles.value.length; i++) {
      const file = byId.get(folderFiles.value[i].id);
      if (file) folderFiles.value[i] = file;
    }
  } finally { ids.forEach(id => fetchingStripIds.delete(id)); }
}

async function loadFolderFilmstrip() {
  const ids = await getQueryFileIds(props.folderQuery);
  if (editorUnmounted) return;
  folderFiles.value = (ids || [fileInfo.value.id]).map((id: number) => id === fileInfo.value.id ? { ...fileInfo.value } : { id });
  await nextTick();
  editorStrip.value?.scrollToPosition(Math.max(0, folderIndex.value - 2) * 96);
  await hydrateFilmstrip(Math.max(0, folderIndex.value - 3), Math.min(folderFiles.value.length - 1, folderIndex.value + 20));
}

async function switchPhoto(fileId: number) {
  if (editorUnmounted || fileId === fileInfo.value?.id) return;
  await historyNavigation;
  await flushRecipeSteps();
  if (editorUnmounted) return;
  const previousIndex = folderIndex.value;
  if (previousIndex >= 0) folderFiles.value[previousIndex] = { ...fileInfo.value };
  restoringHistory = true;
  try {
    ++autoPresetRequestId;
    activeTool.value = '';
    showDiffPreview.value = false;
    uiStore.clearActiveAdjustments();
    isProcessing.value = true;
    await loadFileInfo(fileId);
    if (editorUnmounted) return;
    await nextTick();
    await initEditImage();
    await nextTick();
    const index = folderIndex.value;
    if (index >= 0) folderFiles.value[index] = { ...fileInfo.value };
    editorStrip.value?.scrollToPosition(Math.max(0, index - 2) * 96);
  } finally { restoringHistory = false; }
}

function queuePhoto(fileId: number) {
  photoNavigation = photoNavigation.then(() => switchPhoto(fileId)).catch(error => { historyError.value = String(error); });
}

function navigatePhoto(direction: number) {
  photoNavigation = photoNavigation.then(async () => {
    const file = folderFiles.value[folderIndex.value + direction];
    if (file) await switchPhoto(file.id);
  }).catch(error => { historyError.value = String(error); });
}
const initialImageSrc = ref('');

const { locale, messages } = useI18n();
const localeMsg = computed(() => messages.value[locale.value] as any);

const uiStore = useUIStore();
const appWindow = getCurrentWebviewWindow();
const showDesktopTitleBar = isWin || isLinux;

function sendToParent(payload: Record<string, any>) {
  void tauriEmit('message-from-image-editor', payload).catch((error) => {
    console.error('Failed to notify parent from image editor:', error);
  });
}

async function closeEditorWindow() {
  if (props.embedded) { emit('back'); return; }
  try {
    await appWindow.close();
  } catch (error) {
    try {
      await appWindow.destroy();
    } catch (destroyError) {
      console.error('Failed to close image editor window:', error, destroyError);
    }
    return;
  }

  await new Promise(resolve => window.setTimeout(resolve, 100));
  try {
    if (await appWindow.isVisible()) {
      await appWindow.destroy();
    }
  } catch {
    // The window is already closed.
  }
}

async function loadFileInfo(fileId: number) {
  try {
    imageReady.value = false;
    const file = await getFileInfo(fileId);
    if (editorUnmounted) return;
    if (file) {
      file.thumbnail = getThumbUrl(file.id);
      fileInfo.value = file;
      newFileName.value = file.name?.substring(0, file.name.lastIndexOf('.')) || file.name || '';
      const src = getPreviewUrl(
        file.id,
        file.file_path,
        false,
        Number(file.modified_at || 0),
        config.settings.rawThumbnailSource,
      );
      initialImageSrc.value = typeof src === 'string' ? src : '';
      initialImageSrc.value += `${initialImageSrc.value.includes('?') ? '&' : '?'}original=1`;
    }
  } catch {
    await closeEditorWindow();
  }
}

const isProcessing = ref(props.embedded);
const imageReady = ref(false);
const activeEditorTab = ref<'edit' | 'adjust'>('edit');
const editorTabs = [{key:'basic',label:'Basic Fixes'},{key:'tuning',label:'Tuning'},{key:'effects',label:'Effects'}];
const editorTab = ref('basic');
const basicTools = ['Crop','Straighten','Redeye',"I'm Feeling Lucky",'Auto Contrast','Auto Color','Retouch','Text'];
const basicToolIcons:Record<string,any> = {Crop:IconCrop,Straighten:IconRotateLeft,Redeye:IconUnhide,
  "I'm Feeling Lucky":IconWand,'Auto Contrast':IconAdjustments,'Auto Color':IconPalette,Retouch:IconEdit};
const effectTools = ['Sharpen','Sepia','B&W','Warmify','Film Grain','Tint','Saturation','Soft Focus','Glow','Filtered B&W','Focal B&W','Graduated Tint'];
const tuningTools = [{key:'fillLight',label:'Fill Light',min:0},{key:'highlights',label:'Highlights',min:-100},
  {key:'shadows',label:'Shadows',min:0},{key:'temperature',label:'Color Temperature',min:-100}];
const emptyTools = () => ({fillLight:0,highlights:0,shadows:0,temperature:0,neutral:null,effects:[],retouch:[],text:[],lastStep:''});
const editTools = ref<any>(emptyTools());
const imageSrc = ref('');
const curveDraft = ref<Curves | null>(null);
const curvePreviewSrc = ref('');
const curveNeedsPreview = computed(() => Object.values({...identityCurves(), ...(curveDraft.value || editTools.value.curves)} as Curves)
  .some(points => points.some(([x,y]) => x!==y)));
const displayImageSrc = computed(() => curvePreviewSrc.value || (curveNeedsPreview.value ? '' : imageSrc.value));
function commitCurves(curves: Curves, channel: CurveChannel) {
  editTools.value = {...editTools.value, curves, lastStep:`Curves: ${channel==='rgb' ? 'RGB' : channel[0].toUpperCase()+channel.slice(1)}`};
}
let curveBaseSource = '', curveBase: Promise<ImageData> | null = null, curvePaintId = 0;
watch([imageSrc, () => JSON.stringify(curveDraft.value || editTools.value.curves)], async () => {
  const id=++curvePaintId, source=imageSrc.value;
  const curves:Curves={...identityCurves(), ...(curveDraft.value || editTools.value.curves)};
  const identity=Object.values(curves).every(points=>points.every(([x,y])=>x===y));
  if (source!==curveBaseSource) {
    curveBaseSource=source;curveBase=null;curvePreviewSrc.value='';
    if (!identity) imageReady.value=false;
  }
  if (!source || identity) {curvePreviewSrc.value='';return;}
  try {
    curveBase ||= (async () => {
      const response=await fetch(source);
      if (!response.ok) throw new Error('Curve preview source unavailable');
      const bitmap=await createImageBitmap(await response.blob());
      const canvas=document.createElement('canvas');canvas.width=bitmap.width;canvas.height=bitmap.height;
      const ctx=canvas.getContext('2d',{willReadFrequently:true})!;ctx.drawImage(bitmap,0,0);bitmap.close();
      return ctx.getImageData(0,0,canvas.width,canvas.height);
    })();
    const base=await curveBase;
    if (id!==curvePaintId || editorUnmounted) return;
    const data=new ImageData(new Uint8ClampedArray(base.data),base.width,base.height);
    applyCurves(data.data,curves);
    const canvas=document.createElement('canvas');canvas.width=data.width;canvas.height=data.height;
    canvas.getContext('2d')!.putImageData(data,0,0);
    curvePreviewSrc.value=canvas.toDataURL('image/png');
  } catch (error) {curveBase=null;historyError.value=String(error);}
}, {flush:'post'});
const redEye = ref(false);
const activeTool = ref('');
const effectAmount = ref(50);
const effectColor = ref('#4090cc');
const toolText = ref('');
const toolFont = ref('Arial');
const textSize = ref(32);
const brushSize = ref(3);
const toolPoint = ref({x:0.5,y:0.5});
const retouchTarget = ref<{x:number;y:number}|null>(null);
const retouchSource = ref<{x:number;y:number}|null>(null);

const containerRef = ref<HTMLElement | null>(null);
const containerRect = ref<DOMRect | null>(null);
const containerBounds = ref({ top: 0, left: 0, width: 0, height: 0 });
const containerPadding = 5;

const imageRef = ref<HTMLImageElement | null>(null);
const imageRect = ref<DOMRect | null>(null);
const imageRectOriginal = ref<DOMRect | null>(null);
const imageWidth = ref(0);
const imageHeight = ref(0);
const isRawFile = computed(() => Number(fileInfo.value?.file_type || 0) === 3);
const normalizeRotate = (value: number) => {
  const normalized = Number(value || 0) % 360;
  return normalized < 0 ? normalized + 360 : normalized;
};
const initialDisplayRotate = computed(() => normalizeRotate(Number(fileInfo.value?.rotate || 0)));
const isPortraitForRotation = (width: number, height: number, rotation: number) => {
  const normalized = normalizeRotate(rotation);
  return normalized % 180 !== 0 ? width > height : height > width;
};

const enableTransition = ref(false);
const position = ref({ left: 0, top: 0 });
const isFlippedX = ref(false);
const isFlippedY = ref(false);
const scale = ref(1);
let wheelTransitionTimer: ReturnType<typeof setTimeout> | null = null;
function handleEditorWheel(event: WheelEvent) {
  const target=event.target as Node | null;
  if (!target || !editorRoot.value?.contains(target)) return;
  const overPhoto=containerRef.value?.contains(target);
  // Ctrl+wheel must never fall through to WebView2's page zoom.
  if (!overPhoto && !event.ctrlKey) return;
  event.preventDefault();
  event.stopImmediatePropagation();
  if (!overPhoto || !imageReady.value || isProcessing.value || !event.deltaY) return;
  const bounds=containerRef.value!.getBoundingClientRect();
  const old=scale.value;
  const delta=event.deltaY*(event.deltaMode===1 ? 16 : event.deltaMode===2 ? bounds.height : 1);
  const next=Math.max(.05,Math.min(8,old*Math.exp(-Math.max(-300,Math.min(300,delta))*.0015)));
  const ratio=next/old;
  const x=event.clientX-bounds.left,y=event.clientY-bounds.top;
  enableTransition.value=false;
  position.value={
    left:position.value.left+(1-ratio)*(x-position.value.left-imageWidth.value/2),
    top:position.value.top+(1-ratio)*(y-position.value.top-imageHeight.value/2),
  };
  scale.value=next;
  if (cropStatus.value===1 || cropApplied.value) {
    cropBox.value={left:x+(cropBox.value.left-x)*ratio,top:y+(cropBox.value.top-y)*ratio,
      width:cropBox.value.width*ratio,height:cropBox.value.height*ratio};
    void nextTick(()=>{imageRectOriginal.value=imageRef.value?.getBoundingClientRect() || null;});
  }
  if (wheelTransitionTimer) clearTimeout(wheelTransitionTimer);
  wheelTransitionTimer=setTimeout(()=>{enableTransition.value=true;wheelTransitionTimer=null;},200);
}
const rotate = ref(0);
const showDiffPreview = ref(false);
const showOriginalWhilePressed = ref(false);
const brightness = ref(0);
const contrast = ref(0);
const saturation = ref(100);
const hue = ref(0);
const blur = ref(0);
const selectedFilter = ref('');
const selectedPreset = ref('natural');
const autoPresetValues = ref<AdjustmentValues | null>(null);
let isApplyingPreset = false;
let skipNextCustomPresetLoad = false;
let autoPresetRequestId = 0;

let containerResizeObserver: ResizeObserver | null = null;
let unlistenUpdateFile: (() => void) | null = null;
const isResizing = ref(false);
type AdjustmentValues = {
  brightness: number;
  contrast: number;
  saturation: number;
  hue: number;
  blur: number;
  filter: string;
};

const histogramRef = ref<InstanceType<typeof ImageHistogram> | null>(null);
const histogramSource = computed(() => displayImageSrc.value || fileInfo.value?.thumbnail || '');
const histogramCrop = computed(() => (
  cropApplied.value && crop.value.width > 0 && crop.value.height > 0
    ? {
      x: crop.value.left,
      y: crop.value.top,
      width: crop.value.width,
      height: crop.value.height,
    }
    : null
));
const histogramAdjustments = computed<AdjustmentValues>(() => ({
  brightness: brightness.value,
  contrast: contrast.value,
  saturation: saturation.value,
  hue: hue.value,
  blur: blur.value,
  filter: selectedFilter.value,
}));

function buildAdjustmentFilter(values: AdjustmentValues) {
  return `
    brightness(${100 + values.brightness}%)
    contrast(${100 + values.contrast}%)
    blur(${values.blur}px)
    hue-rotate(${values.hue}deg)
    saturate(${values.saturation}%)
    ${values.filter === 'grayscale' ? 'grayscale(100%)' : ''}
    ${values.filter === 'sepia' ? 'sepia(100%)' : ''}
    ${values.filter === 'invert' ? 'invert(100%)' : ''}
  `;
}

const imageStyle = computed((): CSSProperties => ({
  display: 'block',
  width: `${imageWidth.value}px`,
  height: `${imageHeight.value}px`,
  maxWidth: 'none',
  maxHeight: 'none',
  position: 'absolute',
  filter: showOriginalWhilePressed.value ? 'none' : adjustmentFilter.value,
  transform: `
    translate(${position.value.left}px, ${position.value.top}px)
    rotate(${displayRotate.value}deg)
    scaleX(${isFlippedX.value ? -1 : 1})
    scaleY(${isFlippedY.value ? -1 : 1})
    scale(${scale.value})
  `,
  transition: enableTransition.value ? 'transform 0.3s ease' : 'none',
  backfaceVisibility: 'hidden',
  willChange: 'transform, filter',
}));
const originalImageStyle = computed((): CSSProperties => ({
  ...imageStyle.value,
  filter: 'none',
}));
const adjustedImageStyle = computed((): CSSProperties => ({
  ...imageStyle.value,
  filter: adjustmentFilter.value,
}));
const canShowDiffPreview = computed(() => activeEditorTab.value === 'adjust' && hasAdjustmentChanges.value);
const currentPresetLabel = computed(() => {
  if (showOriginalWhilePressed.value) {
    return presetOptions.value.find(o => o.value === 'natural')?.label || '';
  }
  const key = resolvePresetKey(getCurrentAdjustmentValues());
  return presetOptions.value.find(o => o.value === key)?.label || '';
});
const adjustmentFilter = computed(() => {
  return buildAdjustmentFilter({
    brightness: brightness.value,
    contrast: contrast.value,
    saturation: saturation.value,
    hue: hue.value,
    blur: blur.value,
    filter: selectedFilter.value,
  });
});

const cropStatus = ref(0);
const cropApplied = ref(false);
const isPortrait = ref(false);
const cropBoxFixed = ref(false);

const cropBox = ref({ left: 0, top: 0, width: 0, height: 0 });
const crop = ref({ left: 0, top: 0, width: 0, height: 0 });

const cropAngle = ref(0);
const isRotating = ref(false);

// The rotation shown on screen: the fixed 90° orientation plus the arbitrary
// crop-time straighten angle (only meaningful while cropping / after crop).
const displayRotate = computed(() =>
  cropStatus.value === 1 || cropApplied.value
    ? rotate.value + cropAngle.value
    : rotate.value,
);

const isDragging = ref(false);
const dragHandle = ref('');
const dragStartX = ref(0);
const dragStartY = ref(0);

const cropBoxStyle = computed(() => ({
  top: `${cropBox.value.top}px`,
  left: `${cropBox.value.left}px`,
  width: `${cropBox.value.width}px`,
  height: `${cropBox.value.height}px`,
}));

const baseOutputWidth = computed(() => {
  if ((cropStatus.value === 1 || cropApplied.value) && crop.value.width > 0) {
    return crop.value.width;
  }
  return rotate.value % 180 !== 0 ? imageHeight.value : imageWidth.value;
});
const baseOutputHeight = computed(() => {
  if ((cropStatus.value === 1 || cropApplied.value) && crop.value.height > 0) {
    return crop.value.height;
  }
  return rotate.value % 180 !== 0 ? imageWidth.value : imageHeight.value;
});
const resizeWidthInput = ref('');
const resizeHeightInput = ref('');
const resizePercentageInput = ref('100');
const keepAspectRatio = ref(true);
const resizeAspectRatio = computed(() => {
  if (!baseOutputWidth.value || !baseOutputHeight.value) return 1;
  return baseOutputWidth.value / baseOutputHeight.value;
});
const parsedResizeWidth = computed(() => {
  const width = Number.parseInt(resizeWidthInput.value, 10);
  return Number.isFinite(width) && width > 0 ? width : null;
});
const parsedResizeHeight = computed(() => {
  const height = Number.parseInt(resizeHeightInput.value, 10);
  return Number.isFinite(height) && height > 0 ? height : null;
});
const parsedResizePercentage = computed(() => {
  const percentage = Number.parseFloat(resizePercentageInput.value);
  return Number.isFinite(percentage) && percentage > 0 ? percentage : null;
});
const maxResizeWidth = computed(() => Math.max(1, baseOutputWidth.value));
const maxResizeHeight = computed(() => Math.max(1, baseOutputHeight.value));
const hasResizeChanges = computed(() => {
  return (
    resizeWidthInput.value !== String(baseOutputWidth.value) ||
    resizeHeightInput.value !== String(baseOutputHeight.value) ||
    !keepAspectRatio.value
  );
});
const resizeOutput = computed(() => {
  const widthInput = parsedResizeWidth.value;
  const heightInput = parsedResizeHeight.value;
  const baseWidth = baseOutputWidth.value;
  const baseHeight = baseOutputHeight.value;
  const ratio = resizeAspectRatio.value || 1;

  if (!widthInput && !heightInput) {
    return { width: baseWidth, height: baseHeight, hasResize: false };
  }

  const buildResizeResult = (width: number, height: number) => ({
    width,
    height,
    hasResize: width !== baseWidth || height !== baseHeight,
  });

  if (keepAspectRatio.value) {
    if (widthInput && heightInput) {
      return buildResizeResult(widthInput, heightInput);
    }
    if (widthInput) {
      return buildResizeResult(widthInput, Math.max(1, Math.round(widthInput / ratio)));
    }
    if (heightInput) {
      return buildResizeResult(Math.max(1, Math.round(heightInput * ratio)), heightInput);
    }
  }

  return buildResizeResult(widthInput || baseWidth, heightInput || baseHeight);
});
const hasEditImageChanges = computed(() =>
  normalizeRotate(rotate.value) !== initialDisplayRotate.value ||
  isFlippedX.value ||
  isFlippedY.value
);
const presets: Record<string, AdjustmentValues> = {
  natural: { brightness: 0, contrast: 0, saturation: 100, hue: 0, blur: 0, filter: '' },
  vivid: { brightness: 0, contrast: 10, saturation: 120, hue: 0, blur: 0, filter: '' },
  muted: { brightness: 0, contrast: -10, saturation: 80, hue: 0, blur: 0, filter: '' },
  warm: { brightness: 5, contrast: 0, saturation: 100, hue: 5, blur: 0, filter: '' },
  cool: { brightness: 5, contrast: 0, saturation: 100, hue: -5, blur: 0, filter: '' },
  bw: { brightness: 0, contrast: 0, saturation: 0, hue: 0, blur: 0, filter: 'grayscale' },
  vintage: { brightness: 10, contrast: -10, saturation: 60, hue: 0, blur: 0, filter: 'sepia' },
  invert: { brightness: 0, contrast: 0, saturation: 100, hue: 0, blur: 0, filter: 'invert' },
  kodak: { brightness: 10, contrast: 15, saturation: 120, hue: -5, blur: 0, filter: '' },
  toyo: { brightness: 5, contrast: 0, saturation: 110, hue: 5, blur: 0, filter: '' },
  cinematic: { brightness: 0, contrast: 20, saturation: 80, hue: 0, blur: 0, filter: '' },
  dramatic: { brightness: 0, contrast: 30, saturation: 110, hue: 0, blur: 0, filter: '' },
  cyberpunk: { brightness: 10, contrast: 20, saturation: 130, hue: -15, blur: 0, filter: '' },
};

function sameAdjustmentValues(a: AdjustmentValues, b: AdjustmentValues) {
  return a.brightness === b.brightness
    && a.contrast === b.contrast
    && a.saturation === b.saturation
    && a.hue === b.hue
    && a.blur === b.blur
    && a.filter === b.filter;
}

function resolvePresetKey(values: AdjustmentValues) {
  if (autoPresetValues.value && sameAdjustmentValues(autoPresetValues.value, values)) {
    return 'auto';
  }

  for (const [key, preset] of Object.entries(presets)) {
    if (sameAdjustmentValues(preset, values)) {
      return key;
    }
  }

  return 'custom';
}

function getCurrentAdjustmentValues() {
  return {
    brightness: brightness.value,
    contrast: contrast.value,
    saturation: saturation.value,
    hue: hue.value,
    blur: blur.value,
    filter: selectedFilter.value,
  };
}

function getConfiguredCustomPreset() {
  return {
    brightness: Number(config.imageEditor.custom?.brightness ?? 0),
    contrast: Number(config.imageEditor.custom?.contrast ?? 0),
    saturation: Number(config.imageEditor.custom?.saturation ?? 100),
    hue: Number(config.imageEditor.custom?.hue ?? 0),
    blur: Number(config.imageEditor.custom?.blur ?? 0),
    filter: String(config.imageEditor.custom?.filter ?? ''),
  };
}

function persistCustomPreset(values = getCurrentAdjustmentValues()) {
  config.imageEditor.custom = {
    brightness: values.brightness,
    contrast: values.contrast,
    saturation: values.saturation,
    hue: values.hue,
    blur: values.blur,
    filter: values.filter,
  };
}

function applyAdjustmentValues(values: AdjustmentValues) {
  brightness.value = values.brightness;
  contrast.value = values.contrast;
  saturation.value = values.saturation;
  hue.value = values.hue;
  blur.value = values.blur;
  selectedFilter.value = values.filter;
}

const presetOptions = computed(() => [
  { value: 'auto', label: localeMsg.value.msgbox.image_editor.presets.auto },
  { value: 'natural', label: localeMsg.value.msgbox.image_editor.presets.natural },
  { value: 'vivid', label: localeMsg.value.msgbox.image_editor.presets.vivid },
  { value: 'muted', label: localeMsg.value.msgbox.image_editor.presets.muted },
  { value: 'warm', label: localeMsg.value.msgbox.image_editor.presets.warm },
  { value: 'cool', label: localeMsg.value.msgbox.image_editor.presets.cool },
  { value: 'bw', label: localeMsg.value.msgbox.image_editor.presets.bw },
  { value: 'vintage', label: localeMsg.value.msgbox.image_editor.presets.vintage },
  { value: 'kodak', label: localeMsg.value.msgbox.image_editor.presets.kodak },
  { value: 'toyo', label: localeMsg.value.msgbox.image_editor.presets.toyo },
  { value: 'cinematic', label: localeMsg.value.msgbox.image_editor.presets.cinematic },
  { value: 'dramatic', label: localeMsg.value.msgbox.image_editor.presets.dramatic },
  { value: 'cyberpunk', label: localeMsg.value.msgbox.image_editor.presets.cyberpunk },
  { value: 'invert', label: localeMsg.value.msgbox.image_editor.presets.invert },
  { value: 'custom', label: localeMsg.value.msgbox.image_editor.presets.custom },
]);
const lightSliders = computed(() => [
  {
    key: 'brightness',
    label: localeMsg.value.msgbox.image_editor.brightness,
    model: brightness,
    min: -100,
    max: 100,
    step: 1,
    valueDisplay: `${brightness.value > 0 ? '+' : ''}${brightness.value}`,
  },
  {
    key: 'contrast',
    label: localeMsg.value.msgbox.image_editor.contrast,
    model: contrast,
    min: -100,
    max: 100,
    step: 1,
    valueDisplay: `${contrast.value > 0 ? '+' : ''}${contrast.value}`,
  },
]);
const colorSliders = computed(() => [
  {
    key: 'saturation',
    label: localeMsg.value.msgbox.image_editor.saturation,
    model: saturation,
    min: 0,
    max: 200,
    step: 1,
    valueDisplay: `${saturation.value}%`,
  },
  {
    key: 'hue',
    label: localeMsg.value.msgbox.image_editor.hue_rotate,
    model: hue,
    min: -180,
    max: 180,
    step: 1,
    valueDisplay: `${hue.value > 0 ? '+' : ''}${hue.value}`,
  },
  {
    key: 'blur',
    label: localeMsg.value.msgbox.image_editor.blur,
    model: blur,
    min: 0,
    max: 20,
    step: 1,
    valueDisplay: `${blur.value}`,
  },
]);
const hasAdjustmentChanges = computed(() => {
  const p = presets.natural;
  return (
    brightness.value !== p.brightness ||
    contrast.value !== p.contrast ||
    saturation.value !== p.saturation ||
    hue.value !== p.hue ||
    blur.value !== p.blur ||
    selectedFilter.value !== p.filter
  );
});

const cropShapeOptions = computed(() => {
  if (isPortrait.value) {
    return [
      { value: '0', label: localeMsg.value.msgbox.image_editor.crop_shape_custom },
      { value: '1', label: '1:1' },
      { value: '2', label: '3:4' },
      { value: '3', label: '2:3' },
      { value: '4', label: '10:16' },
      { value: '5', label: '9:16' },
      { value: '6', label: '1:2' },
    ];
  }

  return [
    { value: '0', label: localeMsg.value.msgbox.image_editor.crop_shape_custom },
    { value: '1', label: '1:1' },
    { value: '2', label: '4:3' },
    { value: '3', label: '3:2' },
    { value: '4', label: '16:10' },
    { value: '5', label: '16:9' },
    { value: '6', label: '2:1' },
  ];
});

const newFileName = ref('');

const fileFormatOptions = computed(() => getSelectOptions(localeMsg.value.msgbox.image_editor.format_options));
const fileQualityOptions = computed(() => getSelectOptions(localeMsg.value.msgbox.image_editor.quality_options));
const outputFormatValues = ['jpg', 'png', 'webp'] as const;

function getSelectedOutputFormat() {
  return outputFormatValues[config.imageEditor.format] || outputFormatValues[0];
}

const combinedFormatKey = computed({
  get: () => {
    if (config.imageEditor.format !== 0) return String(config.imageEditor.format);
    return `0-${config.imageEditor.quality}`;
  },
  set: (key: string) => {
    if (key.includes('-')) {
      const [f, q] = key.split('-').map(Number);
      config.imageEditor.format = f;
      config.imageEditor.quality = q;
    } else {
      config.imageEditor.format = Number(key);
      config.imageEditor.quality = 0;
    }
  },
});

const combinedFormatOptions = computed(() => {
  const fmt = fileFormatOptions.value;
  const qual = fileQualityOptions.value;
  const items: { value: string; label: string }[] = [];
  // JPEG with quality levels
  items.push({ value: '0-0', label: `${fmt[0].label} (${qual[0].label})` });
  items.push({ value: '0-1', label: `${fmt[0].label} (${qual[1].label})` });
  items.push({ value: '0-2', label: `${fmt[0].label} (${qual[2].label})` });
  // PNG, WebP — no quality variants
  for (let i = 1; i < fmt.length; i++) {
    items.push({ value: String(i), label: fmt[i].label });
  }
  return items;
});

const savedEditState = computed(() => {
  try { return JSON.parse(fileInfo.value?.edits || '{}'); } catch { return {}; }
});

const handleResizeWidthInput = () => {
  const width = parsedResizeWidth.value;
  if (!width) return;

  const clampedWidth = Math.min(maxResizeWidth.value, Math.max(1, width));
  if (clampedWidth !== width) {
    resizeWidthInput.value = String(clampedWidth);
  }

  syncResizePercentage();
  if (!keepAspectRatio.value) return;
  resizeHeightInput.value = String(Math.min(maxResizeHeight.value, Math.max(1, Math.round(clampedWidth / resizeAspectRatio.value))));
};

const handleResizeHeightInput = () => {
  const height = parsedResizeHeight.value;
  if (!height) return;

  const clampedHeight = Math.min(maxResizeHeight.value, Math.max(1, height));
  if (clampedHeight !== height) {
    resizeHeightInput.value = String(clampedHeight);
  }

  if (!keepAspectRatio.value) return;
  resizeWidthInput.value = String(Math.min(maxResizeWidth.value, Math.max(1, Math.round(clampedHeight * resizeAspectRatio.value))));
  syncResizePercentage();
};

const syncResizePercentage = () => {
  if (!baseOutputWidth.value || !parsedResizeWidth.value) return;
  const percentage = Math.round((parsedResizeWidth.value / baseOutputWidth.value) * 1000) / 10;
  resizePercentageInput.value = String(percentage);
};

const handleResizePercentageInput = () => {
  const percentage = parsedResizePercentage.value;
  if (!percentage) return;

  const clampedPercentage = Math.min(100, Math.max(1, percentage));
  if (clampedPercentage !== percentage) {
    resizePercentageInput.value = String(clampedPercentage);
  }
  resizeWidthInput.value = String(Math.max(1, Math.round(baseOutputWidth.value * clampedPercentage / 100)));
  resizeHeightInput.value = String(Math.max(1, Math.round(baseOutputHeight.value * clampedPercentage / 100)));
};

const resetResize = () => {
  resizeWidthInput.value = String(baseOutputWidth.value);
  resizeHeightInput.value = String(baseOutputHeight.value);
  resizePercentageInput.value = '100';
  keepAspectRatio.value = true;
};

watch(
  () => [baseOutputWidth.value, baseOutputHeight.value],
  ([width, height]) => {
    resizeWidthInput.value = width > 0 ? String(width) : '';
    resizeHeightInput.value = height > 0 ? String(height) : '';
    resizePercentageInput.value = '100';
  },
  { immediate: true }
);

watch(
  () => keepAspectRatio.value,
  (enabled) => {
    if (!enabled || !parsedResizeWidth.value) return;
    resizeHeightInput.value = String(Math.max(1, Math.round(parsedResizeWidth.value / resizeAspectRatio.value)));
  }
);

watch(selectedPreset, () => {
  if (selectedPreset.value === 'custom') {
    autoPresetRequestId++;
    if (skipNextCustomPresetLoad) {
      skipNextCustomPresetLoad = false;
      return;
    }

    const custom = getConfiguredCustomPreset();
    isApplyingPreset = true;
    applyAdjustmentValues(custom);
    nextTick(() => {
      isApplyingPreset = false;
    });
    return;
  }

  if (selectedPreset.value === 'auto') {
    applyAutoPreset();
    return;
  }

  const p = presets[selectedPreset.value];
  if (!p) return;
  autoPresetRequestId++;
  isApplyingPreset = true;
  applyAdjustmentValues(p);
  nextTick(() => {
    isApplyingPreset = false;
  });
});

watch(activeEditorTab, (tab) => {
  if (tab !== 'adjust') {
    showDiffPreview.value = false;
    showOriginalWhilePressed.value = false;
    return;
  }
});

watch(histogramSource, () => {
  autoPresetValues.value = null;
});


watch([brightness, contrast, saturation, hue, blur, selectedFilter], () => {
  if (isApplyingPreset) return;

  const currentValues = getCurrentAdjustmentValues();
  const resolvedPreset = resolvePresetKey(currentValues);

  if (resolvedPreset === 'natural') {
    showDiffPreview.value = false;
  }

  if (resolvedPreset === 'custom') {
    persistCustomPreset(currentValues);
  }

  if (selectedPreset.value !== 'custom') {
    const p = presets[selectedPreset.value];
    if (!p) {
      skipNextCustomPresetLoad = true;
      selectedPreset.value = 'custom';
      return;
    }
    if (
      (brightness.value !== p.brightness ||
        contrast.value !== p.contrast ||
        saturation.value !== p.saturation ||
        hue.value !== p.hue ||
        blur.value !== p.blur ||
        selectedFilter.value !== p.filter)
    ) {
      skipNextCustomPresetLoad = true;
      selectedPreset.value = 'custom';
    }
  }
});

watch(
  [brightness, contrast, saturation, hue, blur, selectedFilter, () => resizeOutput.value.width, () => resizeOutput.value.height],
  () => {
    if (!fileInfo.value?.file_path) return;
    uiStore.setActiveAdjustments(fileInfo.value.file_path, {
      brightness: brightness.value,
      contrast: contrast.value,
      saturation: saturation.value,
      hue: hue.value,
      blur: blur.value,
      filter: selectedFilter.value || null,
      resize: resizeOutput.value.hasResize ? {
        width: resizeOutput.value.width,
        height: resizeOutput.value.height,
      } : null,
    });
  },
  { immediate: true }
);

watch(() => config.settings.language, (newLanguage) => {
  locale.value = newLanguage;
});

watch(() => config.settings.appearance, (newAppearance) => {
  setTheme(newAppearance, newAppearance === 0 ? config.settings.lightTheme : config.settings.darkTheme);
});

watch(() => config.settings.lightTheme, (newLightTheme) => {
  setTheme(config.settings.appearance, newLightTheme);
});

watch(() => config.settings.darkTheme, (newDarkTheme) => {
  setTheme(config.settings.appearance, newDarkTheme);
});

watch(() => Number(config.settings.scale || 1), (newScale) => {
  const normalizedScale = SCALE_VALUES.find((item) => item === newScale) ?? 1;
  document.documentElement.style.fontSize = `${normalizedScale * 16}px`;
});

onMounted(async () => {
  if (props.embedded) editorRoot.value?.focus({ preventScroll: true });
  window.addEventListener('keydown', handleKeyDown, true);
  window.addEventListener('wheel', handleEditorWheel, {capture:true,passive:false});
  uiStore.pushInputHandler('EditImage');
  activeEditorTab.value = config.imageEditor.tab === 'adjust' ? 'adjust' : 'edit';

  const query = router.currentRoute.value.query;
  const fileId = Number(props.initialFile?.id || query.fileId || 0);
  if (fileId > 0) {
    await loadFileInfo(fileId);
  }
  if (editorUnmounted) return;

  if (!fileInfo.value) {
    await closeEditorWindow();
    return;
  }

  isProcessing.value = true;
  await nextTick();
  initEditImage();
  if (props.embedded) void loadFolderFilmstrip();

  unlistenUpdateFile = await listen('update-file', async (event: any) => {
    const newFileId = Number(event?.payload?.fileId || 0);
    if (newFileId > 0 && newFileId !== Number(fileInfo.value?.id || 0)) {
      await loadFileInfo(newFileId);
      if (fileInfo.value) {
        initEditImage();
      }
    }
  });
  if (editorUnmounted) { unlistenUpdateFile(); unlistenUpdateFile = null; return; }

  containerResizeObserver = new ResizeObserver(() => {
    isResizing.value = true;
    enableTransition.value = false;
    cropBoxFixed.value = false;
    containerRect.value = containerRef.value?.getBoundingClientRect() || null;
    if (containerRect.value) {
      containerBounds.value = {
        left: containerRect.value.left + containerPadding,
        top: containerRect.value.top + containerPadding,
        width: containerRect.value.width - containerPadding * 2,
        height: containerRect.value.height - containerPadding * 2,
      };
      autoFitVisualArea();
    }
    if (cropStatus.value === 1 || cropApplied.value) {
      requestAnimationFrame(() => {
        imageRectOriginal.value = imageRef.value?.getBoundingClientRect() || null;
        updateCropBoxFromCrop();
        enableTransition.value = true;
        isResizing.value = false;
      });
    } else {
      enableTransition.value = true;
      isResizing.value = false;
    }
  });
  if (containerRef.value) {
    containerResizeObserver.observe(containerRef.value);
  }
});

onUnmounted(() => {
  editorUnmounted = true;
  window.removeEventListener('keydown', handleKeyDown, true);
  window.removeEventListener('wheel', handleEditorWheel, true);
  if (wheelTransitionTimer) clearTimeout(wheelTransitionTimer);
  uiStore.removeInputHandler('EditImage');
  if (containerResizeObserver) {
    containerResizeObserver.disconnect();
    containerResizeObserver = null;
  }
  if (unlistenUpdateFile) {
    unlistenUpdateFile();
    unlistenUpdateFile = null;
  }
});

const onImageLoad = async () => {
  await nextTick();
  if (imageReady.value && curvePreviewSrc.value && imageRef.value?.currentSrc===curvePreviewSrc.value) return;

  if (imageRef.value && imageRef.value.naturalWidth > 0 && imageRef.value.naturalHeight > 0) {
    imageWidth.value = imageRef.value.naturalWidth;
    imageHeight.value = imageRef.value.naturalHeight;
    isPortrait.value = isPortraitForRotation(imageWidth.value, imageHeight.value, rotate.value);
  }

  autoFitVisualArea();
  const saved = savedEditState.value.recipe;
  if (saved?.crop?.width > 0 && saved.crop.height > 0) {
    crop.value = { left: saved.crop.x, top: saved.crop.y, width: saved.crop.width, height: saved.crop.height };
    cropAngle.value = saved.cropAngle || 0;
    await nextTick();
    imageRectOriginal.value = imageRef.value?.getBoundingClientRect() || null;
    updateCropBoxFromCrop();
    cropApplied.value = true;
    fitCropBoxToContainer();
  }
  await nextTick();
  if (saved?.resize?.width && saved.resize.height) {
    keepAspectRatio.value = false;
    await nextTick();
    resizeWidthInput.value = String(saved.resize.width);
    resizeHeightInput.value = String(saved.resize.height);
  }

  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      enableTransition.value = true;
      imageReady.value = true;
      isProcessing.value = false;
    });
  });
};

const initEditImageLoadingId = ref(0);

const initEditImage = async () => {
  imageReady.value = false;
  initEditImageLoadingId.value++;
  const loadingId = initEditImageLoadingId.value;

  // Use the same oriented recipe base for every photo, including Undo All.
  // Asset scope can lag behind newly added photos and cannot render corrections.
  {
    if (initialImageSrc.value) {
      imageSrc.value = initialImageSrc.value;
    }

    void (async () => {
      try {
        if (loadingId !== initEditImageLoadingId.value) return;
        const previewSrc = getPreviewUrl(
          fileInfo.value.id,
          fileInfo.value.file_path,
          false,
          Number(fileInfo.value.modified_at || 0),
          config.settings.rawThumbnailSource,
        );
        if (previewSrc) {
          imageSrc.value = `${previewSrc}${previewSrc.includes('?') ? '&' : '?'}original=1&baseEdit=1&rk=${savedEditState.value.render_key || ''}`;
        }
      } catch {
        if (loadingId !== initEditImageLoadingId.value) return;
      }
    })();
  }

  imageWidth.value = fileInfo.value.width;
  imageHeight.value = fileInfo.value.height;
  isPortrait.value = isPortraitForRotation(imageWidth.value, imageHeight.value, initialDisplayRotate.value);

  containerRect.value = containerRef.value?.getBoundingClientRect() || null;
  if (!containerRect.value) return;

  containerBounds.value = {
    left: containerRect.value.left + containerPadding,
    top: containerRect.value.top + containerPadding,
    width: containerRect.value.width - containerPadding * 2,
    height: containerRect.value.height - containerPadding * 2,
  };

  enableTransition.value = false;

  cropStatus.value = 0;
  cropApplied.value = false;
  crop.value = { left: 0, top: 0, width: 0, height: 0 };
  cropAngle.value = 0;
  const saved = savedEditState.value.recipe;
  editTools.value = { ...emptyTools(), ...JSON.parse(JSON.stringify(saved?.tools || {})) };
  baseSignature=recipeBaseSignature(savedEditState.value);
  redEye.value = !!saved?.redEye;
  if (saved) {
    rotate.value = Number(saved.rotate || 0);
    isFlippedX.value = !!saved.flipHorizontal;
    isFlippedY.value = !!saved.flipVertical;
    cropAngle.value = Number(saved.cropAngle || 0);
    brightness.value = saved.brightness || 0;
    contrast.value = saved.contrast || 0;
    saturation.value = (saved.saturation ?? 1) * 100;
    hue.value = saved.hue_rotate || 0;
    blur.value = saved.blur || 0;
    selectedFilter.value = saved.filter || '';
  } else if (uiStore.activeAdjustments.filePath === fileInfo.value.file_path) {
    const adj = uiStore.activeAdjustments;
    rotate.value = initialDisplayRotate.value;
    isFlippedX.value = false;
    isFlippedY.value = false;
    brightness.value = adj.brightness || 0;
    contrast.value = adj.contrast || 0;
    saturation.value = adj.saturation ?? 100;
    hue.value = adj.hue || 0;
    blur.value = adj.blur || 0;
    selectedFilter.value = adj.filter || '';
    const restoredPreset = resolvePresetKey({
      brightness: brightness.value,
      contrast: contrast.value,
      saturation: saturation.value,
      hue: hue.value,
      blur: blur.value,
      filter: selectedFilter.value,
    });
    if (restoredPreset === 'custom') {
      skipNextCustomPresetLoad = true;
    }
    selectedPreset.value = restoredPreset;
  } else {
    rotate.value = initialDisplayRotate.value;
    isFlippedX.value = false;
    isFlippedY.value = false;
    resetAdjustments();
  }

};

async function getAutoPresetValues() {
  if (autoPresetValues.value) return autoPresetValues.value;
  autoPresetValues.value = await histogramRef.value?.getAutoPresetValues() || presets.natural;
  return autoPresetValues.value;
}

async function applyAutoPreset() {
  const requestId = ++autoPresetRequestId;
  try {
    const values = await getAutoPresetValues();
    if (requestId !== autoPresetRequestId || selectedPreset.value !== 'auto') return;
    isApplyingPreset = true;
    applyAdjustmentValues(values);
  } finally {
    if (isApplyingPreset && requestId === autoPresetRequestId) {
      nextTick(() => {
        isApplyingPreset = false;
      });
    }
  }
}

function presetThumbnailFilter(presetKey: string) {
  const p = presetKey === 'custom'
    ? getConfiguredCustomPreset()
    : presetKey === 'auto'
      ? (autoPresetValues.value || presets.natural)
      : presets[presetKey];
  if (!p) return '';
  return buildAdjustmentFilter(
    presetKey === 'custom'
      ? { ...p, brightness: brightness.value, contrast: contrast.value, saturation: saturation.value, hue: hue.value, blur: blur.value, filter: selectedFilter.value }
      : p
  );
}

const resetAdjustments = () => {
  const p = presets.natural;
  brightness.value = p.brightness;
  contrast.value = p.contrast;
  saturation.value = p.saturation;
  hue.value = p.hue;
  blur.value = p.blur;
  selectedFilter.value = p.filter;
  selectedPreset.value = 'natural';
  showDiffPreview.value = false;
  showOriginalWhilePressed.value = false;
};

function setActiveEditorTab(tab: 'edit' | 'adjust') {
  if (cropStatus.value === 1) return;
  activeEditorTab.value = tab;
  config.imageEditor.tab = tab;
}

function handlePreviewPointerDown() {
  if (activeEditorTab.value !== 'adjust' || showDiffPreview.value || !hasAdjustmentChanges.value) return;
  showOriginalWhilePressed.value = true;
}

function handlePreviewPointerUp() {
  showOriginalWhilePressed.value = false;
}

function toggleDiffPreview() {
  if (!hasAdjustmentChanges.value) return;
  showOriginalWhilePressed.value = false;
  showDiffPreview.value = !showDiffPreview.value;
}

const clickStartCrop = () => {
  cropStatus.value = 1;
  cropApplied.value = false;
  cropBoxFixed.value = false;
  cropAngle.value = 0;
  initCropBox();
};

const toggleCropMode = () => {
  if (cropStatus.value === 1) {
    clearCrop();
    return;
  }

  if (cropApplied.value) {
    clickRestoreCrop();
    return;
  }

  if (cropStatus.value === 0) {
    clickStartCrop();
    return;
  }
};

const clearCrop = () => {
  cropStatus.value = 0;
  cropApplied.value = false;
  cropBoxFixed.value = false;
  crop.value = { left: 0, top: 0, width: 0, height: 0 };
  cropBox.value = { left: 0, top: 0, width: 0, height: 0 };
  cropAngle.value = 0;
  autoFitVisualArea();
};

const clickRestoreAll = () => {
  if (cropStatus.value === 1 || cropApplied.value) return;

  rotate.value = initialDisplayRotate.value;
  isFlippedX.value = false;
  isFlippedY.value = false;
  isPortrait.value = isPortraitForRotation(imageWidth.value, imageHeight.value, initialDisplayRotate.value);
  autoFitVisualArea();
};

const clickCancelCrop = () => {
  cropStatus.value = 0;
  cropApplied.value = false;
  crop.value = { left: 0, top: 0, width: 0, height: 0 };
  cropBox.value = { left: 0, top: 0, width: 0, height: 0 };
  cropAngle.value = 0;
  autoFitVisualArea();
};

const clickRestoreCrop = () => {
  cropStatus.value = 1;
  cropBoxFixed.value = false;
  autoFitVisualArea();
};

const autoFitVisualArea = () => {
  if (cropApplied.value) {
    fitCropBoxToContainer();
  } else {
    fitImageToContainer();
  }
};

const clickDoCrop = () => {
  editTools.value.lastStep = activeTool.value === 'Straighten' ? 'Straighten' : 'Crop';
  cropApplied.value = true;
  cropBoxFixed.value = false;
  fitCropBoxToContainer();

  cropStatus.value = 0;
  activeTool.value = '';
};

const togglePortraitAndLandscape = () => {
  isPortrait.value = !isPortrait.value;
  initCropBox();
};

const toggleCropBoxFixed = () => {
  cropBoxFixed.value = !cropBoxFixed.value;
  cropBoxFixed.value ? fitCropBoxToContainer() : fitImageToContainer();
};

const onChangeCropShape = () => {
  initCropBox();
};

const recomputeCropBoxForAngle = () => {
  if (cropStatus.value !== 1) return;
  containerRect.value = containerRef.value?.getBoundingClientRect() || null;
  imageRect.value = imageRef.value?.getBoundingClientRect() || null;
  if (!imageRect.value || !containerRect.value) return;

  // Effective dimensions after the fixed 90° rotation, plus the rotated AABB the
  // backend crops from (shared helper keeps all three mappings on one formula).
  const { imgWidth, imgHeight, bbW, bbH } = getCropSpaceDims();

  // Selected crop aspect ratio (null when "custom" / free).
  const selectedShape = cropShapeOptions.value.find(option => option.value === String(config.imageEditor.cropShape) && option.value !== '0');
  let aspectRatio: number | undefined;
  if (selectedShape && selectedShape.label) {
    const parts = selectedShape.label.split(':');
    aspectRatio = parseInt(parts[0], 10) / parseInt(parts[1], 10);
  }

  // Largest axis-aligned rectangle inside the rotated image, expressed in the
  // rotated image's bounding-box pixel space (what the backend crops from).
  const rect = getMaxInscribedRect(imgWidth, imgHeight, cropAngle.value, aspectRatio);

  const scaleX = imageRect.value.width / bbW;
  const scaleY = imageRect.value.height / bbH;
  const wScreen = rect.width * scaleX;
  const hScreen = rect.height * scaleY;

  const centerX = imageRect.value.left - containerRect.value.left + imageRect.value.width / 2;
  const centerY = imageRect.value.top - containerRect.value.top + imageRect.value.height / 2;

  cropBox.value = {
    left: centerX - wScreen / 2,
    top: centerY - hScreen / 2,
    width: wScreen,
    height: hScreen,
  };

  crop.value = {
    left: Math.round(rect.x),
    top: Math.round(rect.y),
    width: Math.round(rect.width),
    height: Math.round(rect.height),
  };
};

let cropAngleRaf = 0;

// Apply an angle and refresh the crop box on the next frame. The recompute is
// rAF-coalesced: rapid slider events collapse to one layout read + inscribed-rect
// solve per frame, avoiding forced-reflow-per-tick. Vue patches the DOM in a
// microtask, so the rAF callback already sees the updated rotation.
const applyCropAngle = (v: number) => {
  const clamped = Math.min(45, Math.max(-45, Number.isFinite(v) ? v : 0));
  cropAngle.value = clamped;
  enableTransition.value = false;
  if (cropAngleRaf) cancelAnimationFrame(cropAngleRaf);
  cropAngleRaf = requestAnimationFrame(() => {
    cropAngleRaf = 0;
    recomputeCropBoxForAngle();
    enableTransition.value = true;
  });
};

const setCropAngle = (v: number) => applyCropAngle(v);

const cropAngleModel = computed({
  get: () => cropAngle.value,
  set: (v: number) => setCropAngle(v),
});

const initCropBox = () => {
  containerRect.value = containerRef.value?.getBoundingClientRect() || null;
  imageRect.value = imageRef.value?.getBoundingClientRect() || null;
  if (!imageRect.value || !containerRect.value) return;

  const selectedShape = cropShapeOptions.value.find(option => option.value === String(config.imageEditor.cropShape) && option.value !== '0');
  if (selectedShape && selectedShape.label) {
    const parts = selectedShape.label.split(':');
    const aspectRatio = parseInt(parts[0]) / parseInt(parts[1]);

    let newWidth;
    let newHeight;
    if (imageRect.value.width / imageRect.value.height > aspectRatio) {
      newHeight = imageRect.value.height;
      newWidth = newHeight * aspectRatio;
    } else {
      newWidth = imageRect.value.width;
      newHeight = newWidth / aspectRatio;
    }

    const imageLeft = imageRect.value.left - containerRect.value.left;
    const imageTop = imageRect.value.top - containerRect.value.top;

    cropBox.value = {
      left: imageLeft + (imageRect.value.width - newWidth) / 2,
      top: imageTop + (imageRect.value.height - newHeight) / 2,
      width: newWidth,
      height: newHeight,
    };
  } else {
    cropBox.value = {
      left: imageRect.value.left - containerRect.value.left,
      top: imageRect.value.top - containerRect.value.top,
      width: imageRect.value.width,
      height: imageRect.value.height,
    };
  }

  updateCropFromCropBox();

  // If the image is already straightened, re-constrain the crop box to the
  // rotated image (honoring the selected aspect ratio).
  if (cropAngle.value !== 0) {
    recomputeCropBoxForAngle();
  }
};

// Dimensions of the pixel space the backend crops from: the axis-aligned
// bounding box (AABB) of the image after the fixed 90° rotation PLUS the
// arbitrary straighten angle. `imgWidth/imgHeight` are the 90°-rotated natural
// dimensions; `bbW/bbH` are the AABB dimensions the crop coordinates live in.
// When cropAngle is 0, bbW/bbH equal imgWidth/imgHeight (pre-existing behavior).
const getCropSpaceDims = () => {
  const imgWidth = rotate.value % 180 === 0 ? imageWidth.value : imageHeight.value;
  const imgHeight = rotate.value % 180 === 0 ? imageHeight.value : imageWidth.value;
  const theta = Math.abs(cropAngle.value) * Math.PI / 180;
  const cos = Math.cos(theta);
  const sin = Math.sin(theta);
  return {
    imgWidth,
    imgHeight,
    bbW: imgWidth * cos + imgHeight * sin,
    bbH: imgHeight * cos + imgWidth * sin,
  };
};

// `cached` lets a caller supply already-known rects to skip the two forced
// reflows from getBoundingClientRect(). Used while dragging the crop box: the
// image itself does not move, so its rect (and the container's) are constant for
// the whole gesture. Pass nothing (or a null field) to read fresh — required when
// the image moves (cropBoxFixed) or on layout/resize.
const updateCropFromCropBox = (cached?: { imageRect?: DOMRect | null; containerRect?: DOMRect | null } | null) => {
  if (cropBox.value.width === 0 || cropBox.value.height === 0) {
    crop.value = { left: 0, top: 0, width: 0, height: 0 };
    return;
  }

  containerRect.value = cached?.containerRect ?? containerRef.value?.getBoundingClientRect() ?? null;
  imageRect.value = cached?.imageRect ?? imageRef.value?.getBoundingClientRect() ?? null;
  if (!imageRect.value || !containerRect.value) return;

  // imageRect is the rotated AABB, so map screen px into the AABB pixel space
  // (bbW×bbH) the backend crops from — NOT imgWidth/imgHeight.
  const { bbW, bbH } = getCropSpaceDims();

  const scaleX = bbW / imageRect.value.width;
  const scaleY = bbH / imageRect.value.height;

  crop.value = {
    left: Math.round(scaleX * (cropBox.value.left + containerRect.value.left - imageRect.value.left)),
    top: Math.round(scaleY * (cropBox.value.top + containerRect.value.top - imageRect.value.top)),
    width: Math.round(scaleX * cropBox.value.width),
    height: Math.round(scaleY * cropBox.value.height),
  };
};

const updateCropBoxFromCrop = () => {
  if (crop.value.width === 0 || crop.value.height === 0) {
    cropBox.value = { left: 0, top: 0, width: 0, height: 0 };
    return;
  }

  imageRect.value = imageRectOriginal.value;
  if (!imageRect.value || !containerRect.value) return;

  // imageRectOriginal is likewise the rotated AABB; map crop (AABB pixel space)
  // back to screen px using bbW/bbH so it stays consistent with an active angle.
  const { bbW, bbH } = getCropSpaceDims();

  const scaleX = bbW / imageRect.value.width;
  const scaleY = bbH / imageRect.value.height;

  if (scaleX === 0 || scaleY === 0) return;

  cropBox.value = {
    left: (crop.value.left / scaleX) - containerRect.value.left + imageRect.value.left,
    top: (crop.value.top / scaleY) - containerRect.value.top + imageRect.value.top,
    width: crop.value.width / scaleX,
    height: crop.value.height / scaleY,
  };
};

const scaleFit = (imgWidth: number, imgHeight: number) => {
  scale.value = Math.min(containerBounds.value.width / imgWidth, containerBounds.value.height / imgHeight);
};

const fitImageToContainer = () => {
  containerRect.value = containerRef.value?.getBoundingClientRect() || null;
  if (!containerRect.value) return;

  position.value = {
    left: (containerRect.value.width - imageWidth.value) / 2,
    top: (containerRect.value.height - imageHeight.value) / 2,
  };

  rotate.value % 180 !== 0
    ? scaleFit(imageHeight.value, imageWidth.value)
    : scaleFit(imageWidth.value, imageHeight.value);

  updateCropBoxFromCrop();
};

const fitCropBoxToContainer = () => {
  if (!containerBounds.value || !containerRect.value) return;

  imageRectOriginal.value = imageRect.value;
  const oldScale = scale.value;

  scale.value = Math.min(
    (containerBounds.value.width / cropBox.value.width) * oldScale,
    (containerBounds.value.height / cropBox.value.height) * oldScale,
  );

  position.value = {
    left: position.value.left + (containerRect.value.width / 2 - (cropBox.value.left + cropBox.value.width / 2)) * scale.value / oldScale,
    top: position.value.top + (containerRect.value.height / 2 - (cropBox.value.top + cropBox.value.height / 2)) * scale.value / oldScale,
  };

  const newCropBoxWidth = cropBox.value.width * scale.value / oldScale;
  const newCropBoxHeight = cropBox.value.height * scale.value / oldScale;
  cropBox.value = {
    left: (containerRect.value.width - newCropBoxWidth) / 2,
    top: (containerRect.value.height - newCropBoxHeight) / 2,
    width: newCropBoxWidth,
    height: newCropBoxHeight,
  };

  imageRef.value?.addEventListener('transitionend', () => updateCropFromCropBox(), { once: true });
};

const clickRotate = (degree: number) => {
  editTools.value.lastStep = 'Rotate';
  rotate.value += degree;
  isPortrait.value = !isPortrait.value;
  scaleFit(
    rotate.value % 180 !== 0 ? imageHeight.value : imageWidth.value,
    rotate.value % 180 !== 0 ? imageWidth.value : imageHeight.value,
  );
};

const clickFlipX = () => {
  rotate.value % 180 !== 0
    ? isFlippedY.value = !isFlippedY.value
    : isFlippedX.value = !isFlippedX.value;
};

const clickFlipY = () => {
  rotate.value % 180 !== 0
    ? isFlippedX.value = !isFlippedX.value
    : isFlippedY.value = !isFlippedY.value;
};

const startDrag = (handle: string, event: MouseEvent) => {
  isDragging.value = true;
  dragHandle.value = handle;
  dragStartX.value = event.clientX;
  dragStartY.value = event.clientY;

  if (cropBoxFixed.value && dragHandle.value === 'move') {
    enableTransition.value = false;
  }

  const initialCropBoxData = { ...cropBox.value };
  const initialImagePosition = { ...position.value };
  const initialImageRect = imageRef.value?.getBoundingClientRect() || null;

  // Precompute the image geometry once (it is stable while dragging the crop box).
  // `cos`/`sin` are of the full display rotation; `hw`/`hh` the half display size
  // of the un-rotated image. The crop box is axis-aligned in screen space, so the
  // "box inside rotated image" constraint is solved in the rotated frame (see
  // geometry.ts) — O(1), exact and kink-free.
  const imgRect = initialImageRect;
  const cRect = containerRef.value?.getBoundingClientRect() || null;
  const geo = imgRect && cRect
    ? {
        cx: imgRect.left - cRect.left + imgRect.width / 2,
        cy: imgRect.top - cRect.top + imgRect.height / 2,
        cos: Math.cos((displayRotate.value * Math.PI) / 180),
        sin: Math.sin((displayRotate.value * Math.PI) / 180),
        hw: (imageWidth.value * scale.value) / 2,
        hh: (imageHeight.value * scale.value) / 2,
      }
    : null;

  const doDrag = (e: MouseEvent) => {
    if (!isDragging.value || !initialImageRect || !containerRect.value) return;

    const dx = e.clientX - dragStartX.value;
    const dy = e.clientY - dragStartY.value;

    if (cropBoxFixed.value && dragHandle.value === 'move') {
      const initialImageLeft = initialImageRect.left - containerRect.value.left;
      const initialImageRight = initialImageLeft + initialImageRect.width;
      const maxDx = cropBox.value.left - initialImageLeft;
      const minDx = (cropBox.value.left + cropBox.value.width) - initialImageRight;
      const clampedDx = Math.max(minDx, Math.min(dx, maxDx));

      const initialImageTop = initialImageRect.top - containerRect.value.top;
      const initialImageBottom = initialImageTop + initialImageRect.height;
      const maxDy = cropBox.value.top - initialImageTop;
      const minDy = (cropBox.value.top + cropBox.value.height) - initialImageBottom;
      const clampedDy = Math.max(minDy, Math.min(dy, maxDy));

      position.value.left = initialImagePosition.left + clampedDx;
      position.value.top = initialImagePosition.top + clampedDy;
    } else if (dragHandle.value === 'move') {
      if (!geo) return;
      const a = initialCropBoxData.width / 2;
      const b = initialCropBoxData.height / 2;
      // Desired box center (screen/container frame) from the raw drag delta.
      const dxo = initialCropBoxData.left + dx + a - geo.cx;
      const dyo = initialCropBoxData.top + dy + b - geo.cy;
      // Project the center onto the valid region (rotated rect) — smooth slide
      // along the boundary, no binary search / path dependence.
      const c = clampCenterInRotatedRect(dxo, dyo, a, b, geo.cos, geo.sin, geo.hw, geo.hh);
      cropBox.value.left = geo.cx + c.x - a;
      cropBox.value.top = geo.cy + c.y - b;
    } else {
      if (!geo) return;
      const proposedBox = { ...initialCropBoxData };

      if (dragHandle.value.includes('right')) proposedBox.width += dx;
      if (dragHandle.value.includes('left')) {
        proposedBox.width -= dx;
        proposedBox.left += dx;
      }
      if (dragHandle.value.includes('bottom')) proposedBox.height += dy;
      if (dragHandle.value.includes('top')) {
        proposedBox.height -= dy;
        proposedBox.top += dy;
      }

      const shape = String(config.imageEditor.cropShape);
      if (shape !== '0') {
        const selectedShape = cropShapeOptions.value.find(o => o.value === shape);
        if (selectedShape && selectedShape.label) {
          const parts = selectedShape.label.split(':');
          const aspectRatio = parseInt(parts[0]) / parseInt(parts[1]);

          if (dragHandle.value.includes('left') || dragHandle.value.includes('right')) {
            proposedBox.height = proposedBox.width / aspectRatio;
          } else {
            proposedBox.width = proposedBox.height * aspectRatio;
          }
          if (dragHandle.value.includes('top')) {
            proposedBox.top = initialCropBoxData.top + (initialCropBoxData.height - proposedBox.height);
          }
          if (dragHandle.value.includes('left')) {
            proposedBox.left = initialCropBoxData.left + (initialCropBoxData.width - proposedBox.width);
          }
        }
      }

      // Largest t in [0,1] toward the proposed box that still fits (closed form).
      // t === 1 applies the full proposal; t < 1 glides the handle to the exact
      // boundary instead of snapping back. Min size (10px) is enforced inside.
      const t = maxResizeT(
        initialCropBoxData, proposedBox,
        geo.cx, geo.cy, geo.cos, geo.sin, geo.hw, geo.hh, 10,
      );
      cropBox.value = {
        left: initialCropBoxData.left + t * (proposedBox.left - initialCropBoxData.left),
        top: initialCropBoxData.top + t * (proposedBox.top - initialCropBoxData.top),
        width: initialCropBoxData.width + t * (proposedBox.width - initialCropBoxData.width),
        height: initialCropBoxData.height + t * (proposedBox.height - initialCropBoxData.height),
      };
    }

    // The image only moves in the cropBoxFixed 'move' case; otherwise reuse the
    // drag-start rects to skip two forced reflows per frame.
    const imageMoved = cropBoxFixed.value && dragHandle.value === 'move';
    updateCropFromCropBox(
      imageMoved ? null : { imageRect: initialImageRect, containerRect: cRect },
    );
  };

  // Coalesce mousemove into one solve per animation frame (pointer events can
  // arrive faster than the display refresh on high-Hz mice / trackpads).
  let rafId = 0;
  let lastEvent: MouseEvent | null = null;
  const onMouseMove = (e: MouseEvent) => {
    lastEvent = e;
    if (rafId) return;
    rafId = requestAnimationFrame(() => {
      rafId = 0;
      if (lastEvent) doDrag(lastEvent);
    });
  };

  const stopDrag = () => {
    if (rafId) {
      cancelAnimationFrame(rafId);
      rafId = 0;
    }
    // Apply the final pointer position synchronously so no pending frame is lost.
    if (lastEvent) {
      doDrag(lastEvent);
      lastEvent = null;
    }
    if (cropBoxFixed.value && dragHandle.value === 'move') {
      enableTransition.value = true;
    }
    isDragging.value = false;
    window.removeEventListener('mousemove', onMouseMove);
    window.removeEventListener('mouseup', stopDrag);
  };

  window.addEventListener('mousemove', onMouseMove);
  window.addEventListener('mouseup', stopDrag);
};

function handleKeyDown(event: KeyboardEvent) {
  if (!uiStore.isInputActive('EditImage')) return;
  const historyAction = editHistoryAction(event);
  if (historyAction) {
    event.preventDefault();
    event.stopPropagation();
    queueHistory(historyAction);
    return;
  }
  if (isEditableTarget(event.target)) return;
  if (props.embedded && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) {
    event.preventDefault();
    event.stopImmediatePropagation();
    navigatePhoto(event.key === 'ArrowLeft' ? -1 : 1);
    return;
  }

  switch (event.key) {
    case 'ArrowLeft':
      if (activeEditorTab.value === 'adjust' && !isProcessing.value && cropStatus.value !== 1) {
        movePresetSelection(-1);
        event.preventDefault();
        event.stopPropagation();
      }
      break;
    case 'ArrowRight':
      if (activeEditorTab.value === 'adjust' && !isProcessing.value && cropStatus.value !== 1) {
        movePresetSelection(1);
        event.preventDefault();
        event.stopPropagation();
      }
      break;
    case 'Enter':
      if (isProcessing.value) break;
      if (cropStatus.value === 1) {
        clickDoCrop();
      } else {
        clickSave();
      }
      event.preventDefault();
      event.stopPropagation();
      break;
    case 'Escape':
      if (showDiffPreview.value) {
        showDiffPreview.value = false;
      } else if (cropStatus.value === 1) {
        clickCancelCrop();
      } else {
        clickCancel();
      }
      event.preventDefault();
      event.stopPropagation();
      break;
    case ' ':
      if (cropStatus.value === 1) {
        toggleCropBoxFixed();
        event.preventDefault();
        event.stopPropagation();
      }
      break;
    default:
      break;
  }
}

const clickCancel = async () => {
  await photoNavigation;
  await historyNavigation;
  await flushRecipeSteps();
  if (uiStore.activeAdjustments.filePath === fileInfo.value?.file_path) {
    uiStore.clearActiveAdjustments();
  }
  await closeEditorWindow();
};

function movePresetSelection(direction: number) {
  const currentIndex = presetOptions.value.findIndex(option => option.value === selectedPreset.value);
  if (currentIndex === -1) return;
  const nextIndex = Math.max(0, Math.min(presetOptions.value.length - 1, currentIndex + direction));
  selectedPreset.value = presetOptions.value[nextIndex].value;
}

const setEditParams = (overrides: { fileName?: string; destFilePath?: string; outputFormat?: string } = {}) => {
  let name = overrides.fileName || newFileName.value;
  let outputFormat = overrides.outputFormat || getSelectedOutputFormat();

  let destFilePath = overrides.destFilePath;
  if (!destFilePath) {
    destFilePath = getFullPath(getFolderPath(fileInfo.value.file_path), combineFileName(name, outputFormat));
  }

  return {
    sourceFilePath: fileInfo.value.file_path,
    destFilePath,
    outputFormat,
    quality: [90, 80, 60][config.imageEditor.quality] || 80,
    orientation: fileInfo.value.e_orientation || 1,
    flipHorizontal: isFlippedX.value,
    flipVertical: isFlippedY.value,
    rotate: rotate.value,
    cropAngle: cropAngle.value,
    crop: {
      x: crop.value.left,
      y: crop.value.top,
      width: crop.value.width,
      height: crop.value.height,
    },
    resize: {
      width: resizeOutput.value.hasResize ? resizeOutput.value.width : null,
      height: resizeOutput.value.hasResize ? resizeOutput.value.height : null,
    },
    filter: selectedFilter.value || null,
    brightness: brightness.value !== 0 ? brightness.value : null,
    contrast: contrast.value !== 0 ? contrast.value : null,
    blur: blur.value > 0 ? blur.value : null,
    hue_rotate: hue.value !== 0 ? hue.value : null,
    saturation: saturation.value !== 100 ? saturation.value / 100.0 : null,
    sourceFileId: Number(fileInfo.value.id),
    autoEnhance: !!savedEditState.value.auto_enhance,
    redEye: redEye.value,
    tools: JSON.parse(JSON.stringify(editTools.value)),
  };
};

const executeSave = async (overrides: { fileName?: string; destFilePath?: string; outputFormat?: string } = {}) => {
  const overwrite = !overrides.destFilePath || overrides.destFilePath === fileInfo.value.file_path;
  if (overwrite && !await askDialog(localeMsg.value.msgbox.image_editor.overwrite_confirm,
    { title: localeMsg.value.msgbox.image_editor.overwrite, kind: 'warning' })) return;
  isProcessing.value = true;
  let success = false;
  const sourceFileId = Number(fileInfo.value?.id || 0);
  const savedFilePath = overrides.destFilePath || fileInfo.value.file_path;
  const saveAsNew = savedFilePath !== fileInfo.value.file_path;
  try {
    success = await editImage(setEditParams(overrides), overwrite);
  } finally {
    isProcessing.value = false;
    if (success) {
      uiStore.updateFileVersion(fileInfo.value.file_path);
      if (savedFilePath !== fileInfo.value.file_path) {
        uiStore.updateFileVersion(savedFilePath);
      }
      if (uiStore.activeAdjustments.filePath === fileInfo.value.file_path) {
        uiStore.clearActiveAdjustments();
      }
      sendToParent({ type: 'success', sourceFileId, saveAsNew, filePath: savedFilePath });
    } else {
      sendToParent({ type: 'failed', sourceFileId });
    }
  }
};

const saveRecipe = async (action?: 'undo' | 'redo' | 'reset') => {
  if (isProcessing.value) return;
  // A crop frame is a draft until Confirm; Ctrl-Z cancels that draft first.
  if (cropStatus.value === 1) {
    if (action === 'undo') await restoreEditorControls();
    return;
  }
  isProcessing.value = true;
  const sourceFileId = Number(fileInfo.value.id);
  try {
    await flushRecipeSteps();
    if (action === 'undo' && !savedEditState.value.history?.length) return;
    if (action === 'redo' && !savedEditState.value.redo?.length) return;
    fileInfo.value.edits = action
      ? await restoreImageRecipe(sourceFileId, action === 'reset', action === 'redo')
      : await saveImageRecipe(sourceFileId, setEditParams());
    uiStore.clearActiveAdjustments();
    if (action) await restoreEditorControls();
    sendToParent({ type: action ? 'recipe-step' : 'success', recipeSaved: true, sourceFileId, saveAsNew: false, filePath: fileInfo.value.file_path });
  } catch (error) {
    historyError.value = String(error);
  } finally { isProcessing.value = false; }
};

const clickSave = () => saveRecipe();

const clickExport = async () => {
  if (cropStatus.value === 1 || isProcessing.value) return;
  await flushRecipeSteps();

  {
    isProcessing.value = true;
    try {
      const folderPath = getFolderPath(fileInfo.value.file_path);
      const ext = getSelectedOutputFormat();
      const baseName = newFileName.value;

      let counter = 1;
      let candidateName = `${baseName}_${counter}`;
      let candidatePath = getFullPath(folderPath, combineFileName(candidateName, ext));

      while (await checkFileExists(candidatePath)) {
        counter++;
        candidateName = `${baseName}_${counter}`;
        candidatePath = getFullPath(folderPath, combineFileName(candidateName, ext));
      }

      await executeSave({
        fileName: candidateName,
        destFilePath: candidatePath,
      });
    } catch {
      isProcessing.value = false;
      sendToParent({ type: 'failed', sourceFileId: Number(fileInfo.value?.id || 0) });
    }
  }
};

// Capture one completed control gesture. All writes are serialized so rapid
// tool changes and Ctrl-Z cannot race each other or lose a persisted step.
let restoringHistory = false;
let pendingRecipe: ReturnType<typeof setEditParams> | null = null;
let recipeTimer: ReturnType<typeof setTimeout> | null = null;
let recipeWrites: Promise<void> = Promise.resolve();
let historyNavigation: Promise<void> = Promise.resolve();
const historyError = ref('');

function persistRecipeStep(params: ReturnType<typeof setEditParams>) {
  const sourceFileId = Number(fileInfo.value.id);
  const filePath = fileInfo.value.file_path;
  recipeWrites = recipeWrites.then(async () => {
    const edits = await saveImageRecipe(sourceFileId, params);
    if (Number(fileInfo.value?.id) === sourceFileId) fileInfo.value.edits = edits;
    refreshBasePreview();
    historyError.value = '';
    sendToParent({ type: 'recipe-step', recipeSaved: true, sourceFileId, saveAsNew: false, filePath });
  }).catch(error => { historyError.value = String(error); });
}

async function flushRecipeSteps() {
  if (recipeTimer) clearTimeout(recipeTimer);
  recipeTimer = null;
  if (pendingRecipe) {
    const params = pendingRecipe;
    pendingRecipe = null;
    persistRecipeStep(params);
  }
  await recipeWrites;
  if (historyError.value) throw new Error(historyError.value);
}

async function restoreEditorControls() {
  restoringHistory = true;
  activeTool.value = '';
  ++autoPresetRequestId;
  uiStore.clearActiveAdjustments();
  await initEditImage();
  await onImageLoad();
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
  restoringHistory = false;
}

function queueHistory(action:'undo'|'redo') {
  historyNavigation = historyNavigation.then(() => saveRecipe(action));
}

function historyStepName(step:{step?:string}) {
  return step.step === 'Undo All Edits' ? 'All Edits' : step.step || 'Edit';
}

function toolPreview(name:string) {
  return ({'Sepia':'sepia(1)','B&W':'grayscale(1)','Warmify':'sepia(.3) saturate(1.3)',
    'Tint':'sepia(.5) hue-rotate(150deg)','Saturation':'saturate(2)','Soft Focus':'blur(1px)',
    'Glow':'brightness(1.2)','Filtered B&W':'grayscale(1) contrast(1.2)','Focal B&W':'grayscale(.6)',
    'Graduated Tint':'sepia(.4)','Sharpen':'contrast(1.2)'} as Record<string,string>)[name] || '';
}

async function selectEditTool(tool:string) {
  if (tool==='Crop' || tool==='Straighten') {
    activeTool.value=tool;
    toggleCropMode();
  } else if (tool==="I'm Feeling Lucky") {
    await flushRecipeSteps();
    fileInfo.value.edits = await invoke<string>('toggle_auto_enhance',{fileId:Number(fileInfo.value.id),enable:true});
    refreshBasePreview();
    sendToParent({type:'recipe-step',recipeSaved:true,sourceFileId:Number(fileInfo.value.id),filePath:fileInfo.value.file_path});
  } else if (tool==='Redeye') {
    redEye.value=true;editTools.value.lastStep='Redeye';
  } else if (tool==='Auto Contrast') {
    const values=await getAutoPresetValues();
    brightness.value=values.brightness;contrast.value=values.contrast || 20;
    editTools.value.lastStep=tool;
  } else if (tool==='Auto Color') {
    const image=imageRef.value;
    if (!image) return;
    const canvas=document.createElement('canvas');canvas.width=96;canvas.height=96;
    const context=canvas.getContext('2d')!;context.drawImage(image,0,0,96,96);
    const pixels=context.getImageData(0,0,96,96).data;
    const sum=[0,0,0];for(let i=0;i<pixels.length;i+=4) for(let c=0;c<3;c++) sum[c]+=pixels[i+c];
    const mean=(sum[0]+sum[1]+sum[2])/3;
    editTools.value.neutral=sum.map(value=>mean/Math.max(1,value));editTools.value.lastStep=tool;
  } else if (['Sepia','B&W','Warmify','Film Grain'].includes(tool)) {
    editTools.value.effects.push({name:tool,amount:0.5});editTools.value.lastStep=tool;
  } else {
    activeTool.value=tool;effectAmount.value=tool==='Saturation'?75:50;
    retouchTarget.value=null;retouchSource.value=null;
  }
}

function applyEditEffect() {
  editTools.value.effects.push({name:activeTool.value,amount:effectAmount.value/100,color:effectColor.value,cx:toolPoint.value.x,cy:toolPoint.value.y});
  editTools.value.lastStep=activeTool.value;activeTool.value='';
}

function pickEditorPoint(event:MouseEvent) {
  if (!activeTool.value || cropStatus.value===1 || !imageRef.value) return;
  const image=imageRef.value;const rect=image.getBoundingClientRect();
  const dx=(event.clientX-rect.left-rect.width/2)/scale.value;
  const dy=(event.clientY-rect.top-rect.height/2)/scale.value;
  const a=displayRotate.value*Math.PI/180;
  const x=(dx*Math.cos(a)+dy*Math.sin(a))*(isFlippedX.value?-1:1)+imageWidth.value/2;
  const y=(-dx*Math.sin(a)+dy*Math.cos(a))*(isFlippedY.value?-1:1)+imageHeight.value/2;
  const point={x:Math.max(0,Math.min(1,x/imageWidth.value)),y:Math.max(0,Math.min(1,y/imageHeight.value))};
  toolPoint.value=point;
  if (activeTool.value==='Retouch') {
    if (!retouchTarget.value) retouchTarget.value=point;else retouchSource.value=point;
  } else if (activeTool.value==='Neutral Color Picker') {
    const canvas=document.createElement('canvas');canvas.width=image.naturalWidth;canvas.height=image.naturalHeight;
    const context=canvas.getContext('2d')!;context.drawImage(image,0,0);
    const pixel=context.getImageData(Math.min(canvas.width-1,Math.floor(point.x*canvas.width)),Math.min(canvas.height-1,Math.floor(point.y*canvas.height)),1,1).data;
    const gray=(pixel[0]+pixel[1]+pixel[2])/3;
    editTools.value.neutral=[pixel[0],pixel[1],pixel[2]].map(v=>gray/Math.max(1,v));
    editTools.value.lastStep='Neutral Color Picker';activeTool.value='';
  }
}

function applyRetouch() {
  if (!retouchTarget.value || !retouchSource.value) return;
  editTools.value.retouch.push({sx:retouchSource.value.x,sy:retouchSource.value.y,tx:retouchTarget.value.x,ty:retouchTarget.value.y,radius:brushSize.value/100});
  editTools.value.lastStep='Retouch';activeTool.value='';
}

function applyText() {
  const canvas=document.createElement('canvas');const context=canvas.getContext('2d')!;
  const size=Math.max(12,Math.min(96,textSize.value));
  context.font=`${size}px ${toolFont.value}`;
  canvas.width=Math.min(2048,Math.ceil(context.measureText(toolText.value).width)+4);canvas.height=size*2;
  context.font=`${size}px ${toolFont.value}`;context.fillStyle=effectColor.value;
  context.textBaseline='top';context.fillText(toolText.value,2,2);
  editTools.value.text.push({png:canvas.toDataURL('image/png').split(',')[1],x:toolPoint.value.x,y:toolPoint.value.y,
    width:canvas.width/imageWidth.value,height:canvas.height/imageHeight.value});
  editTools.value.lastStep='Text';activeTool.value='';
}

let baseSignature='';
function recipeBaseSignature(doc:any) {
  const {curves, lastStep, ...tools}=doc.recipe?.tools || {};
  return JSON.stringify([doc.auto_enhance,doc.recipe?.redEye,tools]);
}
function refreshBasePreview() {
  const doc=savedEditState.value;
  const signature=recipeBaseSignature(doc);
  if (signature===baseSignature) return;
  baseSignature=signature;
  const src=getPreviewUrl(fileInfo.value.id,fileInfo.value.file_path,false,Number(fileInfo.value.modified_at||0),config.settings.rawThumbnailSource);
  imageSrc.value=`${src}${src.includes('?')?'&':'?'}original=1&baseEdit=1&rk=${doc.render_key||''}`;
}

watch([rotate, isFlippedX, isFlippedY, cropAngle, cropStatus, brightness, contrast, saturation, hue, blur, selectedFilter,
  () => crop.value.left, () => crop.value.top, () => crop.value.width, () => crop.value.height,
  () => resizeOutput.value.width, () => resizeOutput.value.height, redEye, () => JSON.stringify(editTools.value)], () => {
  if (restoringHistory || !imageReady.value || !fileInfo.value || cropStatus.value === 1) return;
  const params = setEditParams();
  const active = document.activeElement;
  if (active instanceof HTMLInputElement && active.type === 'range') {
    pendingRecipe = params;
    if (recipeTimer) clearTimeout(recipeTimer);
    recipeTimer = setTimeout(() => { void flushRecipeSteps().catch(() => {}); }, 200);
  } else {
    if (pendingRecipe) void flushRecipeSteps().catch(() => {});
    persistRecipeStep(params);
  }
}, { flush: 'post' });

</script>

<style scoped>
.crop-box-active {
  position: absolute;
  border: 1px solid #fff;
  box-shadow: 0 0 0 9999px color-mix(in srgb, var(--color-base-200) 80%, transparent);
  box-sizing: border-box;
  will-change: transform;
  transition: all 0.3s ease;
}

.crop-box-done {
  position: absolute;
  box-shadow: 0 0 0 9999px var(--color-base-200);
  box-sizing: border-box;
  will-change: transform;
}

.no-transition {
  transition: none !important;
}

.drag-handle {
  position: absolute;
  width: 10px;
  height: 10px;
  background-color: #fff;
  border: 1px solid #000;
  box-sizing: border-box;
}

.top-left {
  top: -5px;
  left: -5px;
  cursor: nwse-resize;
}

.top {
  top: -5px;
  left: 50%;
  transform: translateX(-50%);
  cursor: ns-resize;
}

.top-right {
  top: -5px;
  right: -5px;
  cursor: nesw-resize;
}

.left {
  top: 50%;
  left: -5px;
  transform: translateY(-50%);
  cursor: ew-resize;
}

.right {
  top: 50%;
  right: -5px;
  cursor: ew-resize;
}

.bottom-left {
  bottom: -5px;
  left: -5px;
  cursor: nesw-resize;
}

.bottom {
  bottom: -5px;
  left: 50%;
  transform: translateX(-50%);
  cursor: ns-resize;
}

.bottom-right {
  bottom: -5px;
  right: -5px;
  cursor: nwse-resize;
}

.grid-line-h,
.grid-line-v {
  position: absolute;
  background-color: rgba(255, 255, 255, 0.2);
}

.grid-line-h {
  width: 100%;
  height: 1px;
  left: 0;
}

.grid-line-v {
  width: 1px;
  height: 100%;
  top: 0;
}

.grid-line-h-1 {
  top: 33.33%;
}

.grid-line-h-2 {
  top: 66.66%;
}

.grid-line-v-1 {
  left: 33.33%;
}

.grid-line-v-2 {
  left: 66.66%;
}

.crop-dimensions-display {
  position: absolute;
  bottom: 10px;
  left: 50%;
  transform: translateX(-50%);
  background-color: rgba(0, 0, 0, 0.5);
  color: #aaa;
  padding: 2px 8px;
  border-radius: 14px;
  font-size: 12px;
  white-space: nowrap;
}
</style>
