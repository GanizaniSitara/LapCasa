<template>
  <div
    :class="[
      'border-2 flex flex-col items-center cursor-pointer group',
      isActive || isSelected ? 'rounded-lg' : thumbnailCornerClass,
      isTransitionDisabled ? 'transition-none' : 'transition-[background-color,color] ease-in-out duration-150 ',
      filmstrip || config.settings.grid.style === 0 ? 'p-1 w-fit h-fit' : 'w-full h-full',
      isActive || isSelected
        ? (isContentActive ? 'border-primary' : 'border-primary/50')
        : 'border-transparent',
      config.settings.grid.style === 0 && isSelected ? 'bg-base-100 hover:bg-base-100' : 'hover:bg-base-100/30 hover:text-base-content ',
    ]"
    :data-selected="isSelected ? 'true' : undefined"
    :data-file-id="file.id"
    @click="(event: MouseEvent) => $emit('clicked', { shiftKey: event.shiftKey, metaKey: event.metaKey, ctrlKey: event.ctrlKey })"
    @dblclick="handleDoubleClick"
    @contextmenu="handleContextMenu"
  >
    <div
      ref="containerRef"
      data-thumbnail-container
      :class="[
        thumbnailCornerClass,
        'relative isolate flex items-center justify-center overflow-hidden',
        // Whole-picture mode: no filler around the fitted picture, while it
        // loads the placeholder icon still shows.
        isFitWhole && isThumbnailLoaded ? '' : 'bg-base-200/70',
        // Lightbox: face tiles get a white frame and a soft shadow
        faceCrop ? 'ring-2 ring-white shadow-[0_1px_4px_rgba(0,0,0,0.35)]' : '',
        confirmedFeedback ? 'face-confirmed-feedback' : '',
      ]"
      :style="layoutStyle"
      @pointerenter="startMediaPreview"
      @pointerleave="stopMediaPreview"
    >
      <!-- placeholder icon while thumbnail is loading/generating -->
      <div
        v-if="!isThumbnailLoaded && !suppressPlaceholder"
        class="absolute inset-0 flex items-center justify-center text-base-content/20 pointer-events-none"
      >
        <component :is="placeholderIcon" class="w-8 h-8 opacity-30 animate-pulse" />
      </div>

      <!-- image -->
      <img
        v-if="thumbnailSrc"
        :src="faceSrc || thumbnailSrc"
        draggable="false"
        :class="{
          'group-hover:scale-115': shouldScaleThumbnail,
          'scale-115': shouldScaleThumbnail && isSelected,
          'object-contain': wholePicture || (!faceCrop && !isGeometryGridStyle && config.settings.grid.scaling === 0),
          'object-cover': !wholePicture && (faceCrop || isGeometryGridStyle || config.settings.grid.scaling === 1),
          'object-fill': !wholePicture && !faceCrop && !isGeometryGridStyle && config.settings.grid.scaling === 2,
          'transition-all': !suppressPlaceholder && !isTransitionDisabled && normalizedRotate === 0,
          'opacity-0': !isThumbnailLoaded,
          'opacity-100': isThumbnailLoaded && !isDimmedIgnored,
          'opacity-30 grayscale': isThumbnailLoaded && isDimmedIgnored,
          // A little shadow around each whole picture
          'shadow-[0_1px_4px_rgba(0,0,0,0.35)]': isFitWhole && isThumbnailLoaded,
        }"
        :style="{ ...(faceSrc ? { width: '100%', height: '100%', objectFit: 'fill', opacity: 1 } : (faceCrop ? { ...imgStyle, ...faceCrop } : (isFitWhole && !props.file.rotate ? fitWholeStyle : imgStyle))), ...(suppressPlaceholder && !isThumbnailLoaded ? { opacity: 0 } : {}) }"
        :loading="suppressPlaceholder ? 'eager' : 'lazy'"
        @load="handleThumbnailLoad"
        @error="retryThumbnail"
      />
      <video
        v-if="showVideoPreview"
        ref="previewVideoRef"
        class="pointer-events-none absolute inset-0 transition-opacity duration-100"
        :class="{
          'object-contain': !isGeometryGridStyle && config.settings.grid.scaling === 0,
          'object-cover': isGeometryGridStyle || config.settings.grid.scaling === 1,
          'object-fill': !isGeometryGridStyle && config.settings.grid.scaling === 2,
          'scale-115': shouldScaleThumbnail,
          'opacity-100': isVideoPreviewReady,
          'opacity-0': !isVideoPreviewReady,
        }"
        :style="imgStyle"
        :poster="thumbnailSrc"
        draggable="false"
        muted
        autoplay
        loop
        playsinline
        preload="metadata"
        @canplay="isVideoPreviewReady = true"
        @playing="isVideoPreviewReady = true"
        @error="stopMediaPreview"
      ></video>
      <img
        v-if="showAnimatedImagePreview"
        :src="animatedImagePreviewSrc"
        draggable="false"
        class="pointer-events-none absolute inset-0 transition-opacity duration-100"
        :class="{
          'object-contain': !isGeometryGridStyle && config.settings.grid.scaling === 0,
          'object-cover': isGeometryGridStyle || config.settings.grid.scaling === 1,
          'object-fill': !isGeometryGridStyle && config.settings.grid.scaling === 2,
          'scale-115': shouldScaleThumbnail,
          'opacity-100': isAnimatedImagePreviewReady,
          'opacity-0': !isAnimatedImagePreviewReady,
        }"
        :style="previewMediaStyle"
        @load="isAnimatedImagePreviewReady = true"
        @error="stopMediaPreview"
      />

      <!-- status badges -->
      <div
        v-if="statusBadges.length > 0"
        class="pointer-events-none absolute inset-x-0 top-0 h-16"
      ></div>
      <div
        v-if="statusBadges.length > 0"
        class="pointer-events-none absolute top-2 left-2 z-10 flex max-w-[calc(100%-2.5rem)] flex-wrap gap-0.5"
      >
        <div
          v-for="badge in statusBadges"
          :key="badge.key"
          class="thumb-badge thumb-badge-muted"
        >
          <template v-if="badge.icons?.length">
            <div class="flex items-center gap-0.5">
              <component
                :is="entry.icon"
                v-for="(entry, index) in badge.icons"
                :key="`${badge.key}-${index}`"
                class="h-3.5 w-3.5 shrink-0"
                :style="entry.style"
              />
            </div>
          </template>
          <component
            v-else-if="badge.icon"
            :is="badge.icon"
            :class="['h-3.5 w-3.5 shrink-0', badge.iconClass]"
            :style="badge.iconStyle"
          />
          <span v-if="badge.label" class="leading-none">
            {{ badge.label }}
          </span>
          <component
            v-if="badge.trailingIcon"
            :is="badge.trailingIcon"
            :class="['h-3.5 w-3.5 shrink-0', badge.trailingIconClass]"
            :style="badge.trailingIconStyle"
          />
        </div>
      </div>

      <!-- bottom badges -->
      <div
        v-if="thumbnailBadge"
        :class="[
          'pointer-events-none absolute left-0.5 z-10 flex items-center gap-0.5',
          hasBottomMediaBadges ? 'bottom-6' : 'bottom-0.5',
        ]"
      >
        <div class="thumb-badge thumb-badge-muted">
          {{ thumbnailBadge }}
        </div>
      </div>
      <div
        v-if="hasBottomMediaBadges"
        class="pointer-events-none absolute left-0.5 bottom-0.5 z-10 flex items-center gap-0.5"
      >
        <div
          v-if="isLivePhoto || isMotionPhoto"
          class="thumb-badge thumb-badge-muted"
        >
          <IconLivePhoto class="h-3.5 w-3.5 shrink-0" />
          <span class="leading-none">{{ isLivePhoto ? t('image_viewer.live') : t('image_viewer.motion') }}</span>
        </div>
        <div
          v-if="isRawJpegPair"
          class="thumb-badge thumb-badge-muted"
        >
          <span class="leading-none">{{ rawJpegPairBadge }}</span>
        </div>
        <div
          v-if="videoDurationBadge"
          class="thumb-badge thumb-badge-muted"
        >
          {{ videoDurationBadge }}
        </div>
        <div
          v-if="dedupStatus"
          class="thumb-badge"
          :class="dedupStatus === 'keep' ? 'text-base-content/70' : 'text-error/70'"
        >
          {{ dedupStatus === 'keep' ? 'KEEP' : 'DUP' }}
        </div>
      </div>

      <!-- Suggestion review: small "?" in the bottom-left corner of the face -->
      <div
        v-if="isSuggestionTile"
        class="pointer-events-none absolute left-1 bottom-1 z-20 w-4 h-4 inline-flex items-center justify-center rounded-full bg-orange-500 text-white text-[10px] font-bold leading-none ring-1 ring-white"
        title="Suggested match - not confirmed"
      >?</div>

      <!-- Selected faces keep their right-click menu. -->
      <div v-if="!selectMode || facePersonId" class="absolute right-0.5 top-0.5">
        <ContextMenu
          ref="contextMenuRef"
          :class="[
            !isSelected ? 'invisible group-hover:visible bg-base-300/30 rounded-box' : 'bg-base-300/30 rounded-box'
          ]"
          :iconMenu="IconMore"
          :menuItems="() => tileMenuItems"
          :smallIcon="true"
        >
          <template v-if="selectMode" #trigger><span /></template>
        </ContextMenu>
      </div>
    </div>

    <template v-if="!filmstrip">
    <!-- Ignored face: tick = it is this person after all -->
    <div v-if="facePersonId === -1" class="h-7 flex items-center justify-center" @dblclick.stop>
      <button type="button" class="face-review-button !w-auto px-2 text-xs text-gray-700" title="Undo the ignore without changing the face's identity"
        data-testid="restore-globally-ignored" :data-file-id="file.id"
        :disabled="!personFaces.people.get(-1)?.ignored.has(file.id)"
        @click.stop="restoreGlobalIgnore([file.id])">Undo</button>
    </div>
    <div v-else-if="isIgnoredTile" class="h-7 flex items-center justify-center gap-2" @dblclick.stop>
      <button
        type="button"
        class="face-review-button text-green-700"
        title="It is this person after all"
        @click.stop="confirmIgnored([file.id], facePersonId)"
      ><IconOk class="w-3.5 h-3.5" /></button>
    </div>

    <!-- Ignored just now: the tile stays in place, dimmed, with its own Undo -->
    <div v-else-if="isDimmedIgnored" class="h-7 flex items-center justify-center" @dblclick.stop>
      <button
        type="button"
        class="h-6 px-2 inline-flex items-center rounded-[2px] border border-gray-400 bg-gray-100 text-xs text-gray-700 shadow-sm hover:bg-white active:bg-gray-300 active:shadow-inner"
        title="Not ignored after all: make it a suggestion again"
        @click.stop="undoIgnore([file.id], facePersonId)"
      >Undo</button>
    </div>

    <!-- Suggestion review: tick confirms the match, x ignores it -->
    <div v-else-if="isSuggestionTile" class="h-7 flex items-center justify-center gap-2" @dblclick.stop>
      <button
        type="button"
        class="face-review-button text-green-700"
        title="Confirm: this is the person"
        @click.stop="$emit('action', 'confirm-person')"
      ><IconOk class="w-3.5 h-3.5" /></button>
      <button
        type="button"
        class="face-review-button text-red-800"
        title="Ignore: not this person"
        @click.stop="$emit('action', 'exclude-person')"
      ><IconClose class="w-3.5 h-3.5" /></button>
    </div>

    <!-- Same height for every tile while the person view shows tick / x rows -->
    <div v-else-if="facePersonId && (personFaces.controlsRow || personFaces.ignoredMode) && config.settings.grid.style === 0" class="h-7"></div>
    </template>

    <!-- label -->
    <div 
      v-if="!filmstrip && config.settings.grid.style === 0 && config.settings.grid.showLabels !== false && (config.settings.grid.labelPrimary > 0 || config.settings.grid.labelSecondary > 0)"
      class="flex flex-col items-center" 
      :class="{ 
        'text-primary': isSelected && isContentActive,
        'text-primary/50': isSelected && !isContentActive,
      }"
      :style="{ width: layoutStyle.width }"
    >
      <span 
        v-if="config.settings.grid.labelPrimary > 0"
        class="w-full text-sm text-center whitespace-pre text-nowrap text-ellipsis overflow-hidden"
        :title="getGridLabelTooltip(file, config.settings.grid.labelPrimary)"
      >
        {{ getGridLabelText(file, config.settings.grid.labelPrimary) }}
      </span>
      <span 
        v-if="config.settings.grid.labelSecondary > 0"
        class="w-full text-xs text-center whitespace-pre text-nowrap text-ellipsis overflow-hidden"
        :title="getGridLabelTooltip(file, config.settings.grid.labelSecondary)"
      >
        {{ getGridLabelText(file, config.settings.grid.labelSecondary) }}
      </span>
    </div>

  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, ref, watch, toRef, onBeforeUnmount, type CSSProperties, type Component } from 'vue';
import { useI18n } from 'vue-i18n';
import { useUIStore } from '@/stores/uiStore';
import { config } from '@/common/config';
import { THUMBNAIL_BADGE } from '@/common/constants';
import { isMac, shortenFilename, formatFileSize, formatDimensionText, formatDuration, formatTimestamp, formatCaptureSettings, formatCaptureSettingValue, formatCameraInfo, getAssetSrc, getThumbUrl, getLapUrl, getFaceThumbUrl, getFileExtension, imageEditState } from '@/common/utils';
import { isWebViewVideoPlaybackDisabled, getGStreamerAvailability } from '@/common/video';
import { claimHoverPreview, releaseHoverPreview } from '@/common/hoverPreview';
import { faceCropStyle, personFaces, FACE_TILE_ASPECT, suggestedFaceId, ignoredFaceId, confirmIgnored, restoreGlobalIgnore, isSessionIgnored, undoIgnore, hasFaceBox, isConfirmedFor } from '@/common/personFaces';
import ContextMenu from '@/components/ContextMenu.vue';
import { useFileMenuItems } from '@/common/fileMenu';

import { 
  IconMore,
  IconTag,
  IconBookmark,
  IconRotate,
  IconComment,
  IconStarFilled,
  IconFlagFilled,
  IconFlagOff,
  IconLivePhoto,
  IconPhoto,
  IconVideo,
  IconOk,
  IconClose,
} from '@/common/icons';

const props = defineProps({
  file: {
    type: Object,
    required: true,
  },
  isSelected: {
    type: Boolean,
    default: false,
  },
  isActive: {
    type: Boolean,
    default: false,
  },
  selectMode: {
    type: Boolean,
    default: false,
  },
  querySource: {
    type: String,
    default: '',
  },
  dedupStatus: {
    type: String as () => 'keep' | 'dup' | '',
    default: '',
  },
  gridSize: {
    type: Number,
    required: true,
  },
  filmstrip: { type: Boolean, default: false },
  suppressPlaceholder: {
    type: Boolean,
    default: false,
  },
});

const emit = defineEmits([
    'clicked',
    'dblclicked',
    'select-toggled',
    'action',
]);

const isTransitionDisabled = ref(false);
let transitionTimeout: NodeJS.Timeout | null = null;

const containerRef = ref<HTMLElement | null>(null);
const contextMenuRef = ref<InstanceType<typeof ContextMenu> | null>(null);
const previewVideoRef = ref<HTMLVideoElement | null>(null);
const containerWidth = ref(0);
const containerHeight = ref(0);
let resizeObserver: ResizeObserver | null = null;
let previewTimer: ReturnType<typeof setTimeout> | null = null;
let animatedImagePreviewTimer: ReturnType<typeof setTimeout> | null = null;
const showVideoPreview = ref(false);
const isVideoPreviewReady = ref(false);
const showAnimatedImagePreview = ref(false);
const isAnimatedImagePreviewReady = ref(false);
const isVideoFile = computed(() => props.file?.file_type === 2);
const isLivePhoto = computed(() => props.file?.media_subtype === 'live_photo' && !!props.file?.live_photo_video_path);
const isMotionPhoto = computed(() => props.file?.media_subtype === 'motion_photo');
const isRawJpegPair = computed(() => props.file?.media_subtype === 'raw_jpeg_pair');
const rawJpegPairBadge = computed(() => {
  const extension = getFileExtension(props.file?.live_photo_video_path || '').toLowerCase();
  return ['heic', 'heif', 'hif'].includes(extension) ? 'RAW+HEIC' : 'RAW+JPEG';
});
const previewVideoPath = computed(() => isLivePhoto.value ? props.file.live_photo_video_path : props.file?.file_path);
const canPreviewVideo = computed(() => (
  (isVideoFile.value || isLivePhoto.value)
  && !!previewVideoPath.value
  && !isWebViewVideoPlaybackDisabled(previewVideoPath.value)
));
const ANIMATABLE_IMAGE_EXTENSIONS = new Set(['gif', 'png', 'apng', 'webp', 'avif']);
const isAnimatableImageFile = computed(() => ANIMATABLE_IMAGE_EXTENSIONS.has(
  getFileExtension(props.file?.name || props.file?.file_path || '').toLowerCase(),
));
const isGifFile = computed(() => getFileExtension(props.file?.name || props.file?.file_path || '').toLowerCase() === 'gif');
const animatedImagePreviewSrc = computed(() => getAssetSrc(props.file?.file_path || '', Number(props.file?.modified_at || 0)));
const canPreviewAnimatedImage = computed(() => isAnimatableImageFile.value && !!animatedImagePreviewSrc.value);
const isGeometryGridStyle = computed(() => config.settings.grid.style === 2 || config.settings.grid.style === 3);
// The person this tile stands for: its section in the continuous People view (the same
// photo can be in several sections), otherwise the current person (0 outside person views).
const facePersonId = computed(() => Number(props.file?.face_person_id ?? personFaces.personId ?? 0));
// Person view with "show faces" on: crop to this person's face (rotated thumbnails left whole).
const suggestedFace = computed(() => suggestedFaceId(props.file?.id, facePersonId.value));
// Ignored face, shown only in the person view's "Ignored" list.
const isIgnoredTile = computed(() =>
  ignoredFaceId(props.file?.id, facePersonId.value) !== undefined && config.settings.grid.style === 0
);
// Ignored since the person was opened: stays in place, dimmed, with Undo.
const isDimmedIgnored = computed(() =>
  isSessionIgnored(props.file?.id, facePersonId.value) && config.settings.grid.style === 0
);
// Pending suggestion in the person view (listed after the confirmed photos).
const isSuggestionTile = computed(() =>
  suggestedFace.value !== undefined && config.settings.grid.style === 0
);
const confirmedFeedback = ref(false);
let confirmedFeedbackTimer: ReturnType<typeof setTimeout> | undefined;
watch(isSuggestionTile, (pending, wasPending) => {
  if (!wasPending || pending || !isConfirmedFor(props.file.id, facePersonId.value)) return;
  confirmedFeedback.value = true;
  if (confirmedFeedbackTimer) clearTimeout(confirmedFeedbackTimer);
  confirmedFeedbackTimer = setTimeout(() => { confirmedFeedback.value = false; }, 700);
});
const faceCrop = computed(() => {
  if (props.filmstrip || config.settings.grid.personFaces === false || props.file.rotate) return undefined;
  // Portrait tile in the card grid; other grid styles keep their own cell shape.
  const aspect = config.settings.grid.style === 0 ? FACE_TILE_ASPECT : 1;
  return faceCropStyle(props.file.id, props.file.width, props.file.height, aspect, facePersonId.value);
});
const faceThumbnailFailed = ref(false);
// Load an already cropped, small image through the direct index, never the original in the webview.
const faceSrc = computed(() => {
  const path = props.file.file_path;
  const crop = faceCrop.value;
  if (faceThumbnailFailed.value || !crop || !path || !/\.(jpe?g|png|webp)$/i.test(path)) return '';
  const box = personFaces.people.get(facePersonId.value)?.boxes.get(props.file.id);
  if (!box) return '';
  const state = personFaces.people.get(facePersonId.value)!;
  const params = new URLSearchParams({
    fx: String(box.x),
    fy: String(box.y),
    fw: String(box.width),
    fh: String(box.height),
    fa: String(config.settings.grid.style === 0 ? FACE_TILE_ASPECT : 1),
    fid: String(state.confirmedFaces.get(props.file.id) ?? state.suggestions.get(props.file.id) ?? state.ignored.get(props.file.id) ?? 0),
    fs: String(props.gridSize
      * window.devicePixelRatio <= 128 ? 128 : config.settings.thumbnailSize),
    fp: path,
  });
  // An audited legacy box can change coordinate space without changing its pixels.
  if (box.coordinate_space) params.set('fc', box.coordinate_space);
  const edits = imageEditState(props.file.edits);
  if (edits.render_key) params.set('rk', edits.render_key);
  return getFaceThumbUrl(Number(props.file.id), Number(props.file.modified_at || 0), params);
});
watch(() => [props.file.id, facePersonId.value, props.file.edits], () => { faceThumbnailFailed.value = false; });
// Person view with "show whole pictures": never crop, whatever the global grid scaling.
const wholePicture = computed(() =>
  props.filmstrip || (config.settings.grid.personFaces === false && hasFaceBox(props.file.id, facePersonId.value))
);
// The picture is fitted inside its tile (object-contain), so the tile gets no background.
const isFitWhole = computed(() =>
  wholePicture.value || (!faceCrop.value && !isGeometryGridStyle.value && config.settings.grid.scaling === 0)
);
const shouldScaleThumbnail = computed(() => !props.filmstrip && (config.settings.grid.style === 1 || isGeometryGridStyle.value));
const thumbnailCornerClass = computed(() => (
  config.settings.grid.thumbnailCorners === 1 ? 'rounded-none' : 'rounded-box'
));
const placeholderIcon = computed(() => (isVideoFile.value ? IconVideo : IconPhoto));

const thumbnailSrc = ref(
  props.file?.thumbnail ||
  (props.file?.id ? getLapUrl(Number(props.file.id), false, config.settings.thumbnailSize, Number(props.file.modified_at || 0), props.file.edits) : '')
);
const isThumbnailLoaded = ref(false);
const thumbnailAspect = ref(0);
let thumbnailRetryCount = 0;

watch(
  () => [props.file?.id, props.file?.thumbnail, props.file?.edits],
  ([id, src], oldVals) => {
    const editsChanged = oldVals && oldVals[2] !== undefined && oldVals[2] !== props.file?.edits;
    if (editsChanged && props.file?.id) {
      thumbnailSrc.value = getThumbUrl(props.file.id, true, config.settings.thumbnailSize, Number(props.file.modified_at || 0), props.file.edits);
    } else if (imageEditState(props.file.edits).revision && id) {
      thumbnailSrc.value = getLapUrl(Number(id), false, config.settings.thumbnailSize, Number(props.file.modified_at || 0), props.file.edits);
    } else if (src) {
      thumbnailSrc.value = String(src);
    } else if (id && Number(id) > 0) {
      thumbnailSrc.value = getLapUrl(Number(id), false, config.settings.thumbnailSize, Number(props.file?.modified_at || 0), props.file.edits);
    } else {
      thumbnailSrc.value = '';
    }
    thumbnailRetryCount = 0;
  },
  { immediate: true },
);
// Metadata hydration can change the whole-photo thumbnail without changing the face image.
watch(() => faceSrc.value || thumbnailSrc.value, () => {
  isThumbnailLoaded.value = false;
  thumbnailAspect.value = 0;
}, { immediate: true, flush: 'sync' });

async function handleThumbnailLoad(event: Event) {
  const source = faceSrc.value || thumbnailSrc.value;
  const image = event.target as HTMLImageElement;
  if (props.suppressPlaceholder) {
    await image.decode().catch(() => {});
    if (source !== (faceSrc.value || thumbnailSrc.value)) return;
  }
  thumbnailAspect.value = image.naturalWidth / image.naturalHeight;
  isThumbnailLoaded.value = true;
}

async function retryThumbnail() {
  if (faceSrc.value) {
    faceThumbnailFailed.value = true;
    isThumbnailLoaded.value = false;
    return;
  }
  const isThumbnailProtocol = thumbnailSrc.value.startsWith('thumb://localhost')
    || thumbnailSrc.value.startsWith('http://thumb.localhost')
    || thumbnailSrc.value.startsWith('https://thumb.localhost')
    || thumbnailSrc.value.startsWith('lap://localhost')
    || thumbnailSrc.value.startsWith('http://lap.localhost')
    || thumbnailSrc.value.startsWith('https://lap.localhost');
  if (thumbnailRetryCount > 1 || !isThumbnailProtocol) {
    return;
  }
  thumbnailRetryCount++;
  isThumbnailLoaded.value = false;
  thumbnailSrc.value = getThumbUrl(props.file.id, true, config.settings.thumbnailSize, Number(props.file.modified_at || 0), props.file.edits);
}

// Robust ResizeObserver setup using watch to handle v-if
watch(containerRef, (el) => {
  if (resizeObserver) {
    resizeObserver.disconnect();
    resizeObserver = null;
  }
  if (el) {
    resizeObserver = new ResizeObserver((entries) => {
      for (const entry of entries) {
        containerWidth.value = entry.contentRect.width;
        containerHeight.value = entry.contentRect.height;
      }
    });
    resizeObserver.observe(el);
  }
});

onBeforeUnmount(() => {
  if (confirmedFeedbackTimer) clearTimeout(confirmedFeedbackTimer);
  if (resizeObserver) {
    resizeObserver.disconnect();
  }
  stopMediaPreview();
});

watch(() => config.settings.grid.style, () => {
  isTransitionDisabled.value = true;
  if (transitionTimeout) {
    clearTimeout(transitionTimeout);
  }
  transitionTimeout = setTimeout(() => {
    isTransitionDisabled.value = false;
  }, 500);
});

watch(() => props.file.rotate, () => {
  isTransitionDisabled.value = true;
  if (transitionTimeout) {
    clearTimeout(transitionTimeout);
  }
  transitionTimeout = setTimeout(() => {
    isTransitionDisabled.value = false;
  }, 500);
});

watch(() => [props.file.file_path, props.file.live_photo_video_path], () => {
  stopMediaPreview();
});

function startMediaPreview() {
  if (!canPreviewVideo.value && !canPreviewAnimatedImage.value) return;
  claimHoverPreview(stopMediaPreview);
  startVideoPreview();
  startAnimatedImagePreview();
}

function stopMediaPreview() {
  stopVideoPreview();
  stopAnimatedImagePreview();
  releaseHoverPreview(stopMediaPreview);
}

let videoPreviewRequest = 0;

function startVideoPreview() {
  if (!canPreviewVideo.value || previewTimer || showVideoPreview.value) return;

  const request = ++videoPreviewRequest;
  previewTimer = setTimeout(async () => {
    previewTimer = null;
    const available = await getGStreamerAvailability();
    if (request !== videoPreviewRequest || !available) return;
    if (!canPreviewVideo.value || !previewVideoPath.value) return;

    isVideoPreviewReady.value = false;
    showVideoPreview.value = true;
    await nextTick();

    const video = previewVideoRef.value;
    if (!video || request !== videoPreviewRequest) return;

    video.src = getAssetSrc(previewVideoPath.value);
    video.muted = true;

    try {
      await video.play();
    } catch {
      stopMediaPreview();
    }
  }, 400);
}

function stopVideoPreview() {
  videoPreviewRequest++;
  if (previewTimer) {
    clearTimeout(previewTimer);
    previewTimer = null;
  }

  const video = previewVideoRef.value;
  if (video) {
    video.pause();
    video.removeAttribute('src');
    video.load();
  }

  isVideoPreviewReady.value = false;
  showVideoPreview.value = false;
}

function startAnimatedImagePreview() {
  if (!canPreviewAnimatedImage.value || animatedImagePreviewTimer || showAnimatedImagePreview.value) return;

  animatedImagePreviewTimer = setTimeout(() => {
    animatedImagePreviewTimer = null;
    if (!canPreviewAnimatedImage.value) return;
    isAnimatedImagePreviewReady.value = false;
    showAnimatedImagePreview.value = true;
  }, 400);
}

function stopAnimatedImagePreview() {
  if (animatedImagePreviewTimer) {
    clearTimeout(animatedImagePreviewTimer);
    animatedImagePreviewTimer = null;
  }
  isAnimatedImagePreviewReady.value = false;
  showAnimatedImagePreview.value = false;
}

function handleDoubleClick(event: MouseEvent) {
  if (isGifFile.value) {
    // The hover preview and viewer must not share an animated GIF resource.
    // Tear down the thumbnail image first, then give the viewer a fresh asset
    // URL so Chromium creates a new decoder from frame zero.
    stopMediaPreview();
    const filePath = props.file?.file_path;
    if (filePath) {
      uiStore.updateFileVersion(filePath);
    }
  }

  emit('dblclicked', {
    shiftKey: event.shiftKey,
    metaKey: event.metaKey,
    ctrlKey: event.ctrlKey,
  });
}

function handleContextMenu(event: MouseEvent) {
  event.preventDefault();
  event.stopPropagation();
  // Face exclusions remain available when keyboard selection keeps the panel closed.
  if (props.selectMode && (!props.isSelected || !facePersonId.value)) return;
  if (!props.isSelected) {
    emit('clicked', false);
  }
  contextMenuRef.value?.open?.(event.clientX, event.clientY);
}


const layoutStyle = computed(() => {
  const { style } = config.settings.grid;
  const size = props.gridSize;
  if (props.filmstrip) return { width: `${size}px`, height: `${size}px` };
  if (style === 0) {
    // Match the long edge across portrait face crops and landscape photos.
    const width = faceCrop.value ? size * FACE_TILE_ASPECT : size;
    const height = faceCrop.value ? size : size * 3 / 4;
    return { width: `${width}px`, height: `${height}px` };
  }
  if (style === 1) return { width: '100%', height: `${size}px` };
  return { width: '100%', height: '100%' };
});

const imgStyle = computed((): CSSProperties => {
  const { style } = config.settings.grid;
  const size = props.gridSize;
  const isRotated = props.file.rotate && props.file.rotate % 180 !== 0;

  if (isRotated) {
    let w = containerWidth.value;
    let h = containerHeight.value;

    // Optimization: For fixed-size grid (style 0), we know dimensions immediately
    if ((w === 0 || h === 0) && style === 0) {
      w = size;
      h = size;
    }

    if (w > 0 && h > 0) {
      return {
        position: 'absolute',
        left: '50%',
        top: '50%',
        width: `${h}px`,
        height: `${w}px`,
        maxWidth: 'none',
        maxHeight: 'none',
        flex: 'none',
        transform: `translate(-50%, -50%) rotate(${props.file.rotate}deg)`,
        opacity: 1,
      };
    }
    
    // Fallback: Hide until dimensions are known to prevent blinking/glitches
    return { opacity: 0 };
  }

  // Standard behavior for non-swapped rotations (0, 180, 360...)
  return {
    ...layoutStyle.value,
    transform: `rotate(${props.file.rotate || 0}deg)`,
    opacity: 1,
  } as CSSProperties;
});

// Whole-picture mode: size the picture itself to fit the tile (keeping its shape),
// so its shadow follows the picture's edges rather than the square tile.
const fitWholeStyle = computed((): CSSProperties => {
  const width = containerWidth.value;
  const height = containerHeight.value;
  if (!props.filmstrip && thumbnailAspect.value > 0 && width > 0 && height > 0) {
    const fittedWidth = Math.min(width, height * thumbnailAspect.value);
    return { width: `${fittedWidth}px`, height: `${fittedWidth / thumbnailAspect.value}px`, opacity: 1 };
  }
  return {
    width: props.filmstrip ? String(layoutStyle.value.width) : 'auto',
    height: props.filmstrip ? String(layoutStyle.value.height) : 'auto',
    maxWidth: String(layoutStyle.value.width),
    maxHeight: String(layoutStyle.value.height),
    opacity: 1,
  };
});

const previewMediaStyle = computed((): CSSProperties => {
  const { opacity: _opacity, ...style } = imgStyle.value;
  return style;
});

const uiStore = useUIStore();
const isContentActive = computed(() =>
  uiStore.activePane === 'content' && uiStore.inputStack.length === 0
);
const { locale, messages, t } = useI18n();
const localeMsg = computed(() => messages.value[locale.value] as any);

const menuItems = useFileMenuItems(
  toRef(props, 'file'),
  localeMsg,
  isMac,
  t,
  (action) => emit('action', action),
);

// Person view: right-click "Not this person" on a confirmed face.
const tileMenuItems = computed(() => {
  const base = menuItems.value as any[];
  if (!facePersonId.value || (!props.selectMode && !isConfirmedFor(props.file?.id, facePersonId.value) && suggestedFace.value === undefined)) return base;
  return [
    { label: 'Not this person', action: () => emit('action', 'exclude-person') },
    ...(props.selectMode ? [] : [
    { label: '-', action: null },
    ...base,
    ]),
  ];
});

const getGridLabelText = (file: any, option: number) => {
  switch (option) {
    case 0: return '';
    case 1: return shortenFilename(file.name) || ' ';
    case 2: return formatFileSize(file.size) || ' ';
    case 3: return formatDimensionText(file.width, file.height) || ' ';
    case 4: return formatTimestamp(file.taken_date, localeMsg.value.format.date_time) || ' ';
    case 5: return file.geo_name || ' ';
    case 6: return formatCameraInfo(file.e_make, file.e_model) || ' ';
    case 7: return file.e_lens_model || ' ';
    case 8: return formatCaptureSettings(file.e_focal_length, file.e_exposure_time, file.e_f_number, file.e_iso_speed, file.e_exposure_bias) || ' ';
    default: return '';
  }
};

const getGridLabelTooltip = (file: any, option: number) => {
  if (option === 1) return file.name;
  const text = getGridLabelText(file, option);
  return text === ' ' ? '' : text;
};

type ThumbnailBadge = {
  key: string;
  icon?: Component;
  icons?: Array<{
    icon: Component;
    style?: CSSProperties;
  }>;
  label?: string;
  iconClass?: string;
  iconStyle?: CSSProperties;
  trailingIcon?: Component;
  trailingIconClass?: string;
  trailingIconStyle?: CSSProperties;
};

const normalizedRotate = computed(() => {
  const rotate = Number(props.file.rotate || 0) % 360;
  return rotate < 0 ? rotate + 360 : rotate;
});

const videoDurationBadge = computed(() => {
  if (props.file?.file_type !== 2) return '';

  const duration = Number(props.file?.duration);
  if (!Number.isFinite(duration)) return '';

  const formattedDuration = formatDuration(duration);
  return formattedDuration;
});

const thumbnailBadge = computed(() => {
  const summary = Number(config.settings.grid.thumbnailBadge || 0);
  if (summary === THUMBNAIL_BADGE.EMPTY) return '';

  const file = props.file || {};
  switch (summary) {
    case THUMBNAIL_BADGE.FILE_FORMAT: {
      const extension = getFileExtension(file.name || file.file_path || '').trim();
      return extension ? extension.toUpperCase() : '';
    }
    case THUMBNAIL_BADGE.ISO:
      return file.e_iso_speed ? `ISO ${formatCaptureSettingValue(file.e_iso_speed)}` : '';
    case THUMBNAIL_BADGE.SHUTTER_SPEED:
      return formatCaptureSettingValue(file.e_exposure_time);
    case THUMBNAIL_BADGE.APERTURE: {
      const fNumber = formatCaptureSettingValue(file.e_f_number).trim();
      return fNumber ? (fNumber.toLowerCase().startsWith('f/') ? fNumber : `f/${fNumber}`) : '';
    }
    case THUMBNAIL_BADGE.FOCAL_LENGTH:
      return formatCaptureSettingValue(file.e_focal_length);
    case THUMBNAIL_BADGE.EXPOSURE:
      return formatCaptureSettingValue(file.e_exposure_bias);
    default:
      return '';
  }
});

const hasBottomMediaBadges = computed(() => (
  isLivePhoto.value || isMotionPhoto.value || isRawJpegPair.value || Boolean(videoDurationBadge.value) || Boolean(props.dedupStatus)
));

const statusBadges = computed<ThumbnailBadge[]>(() => {
  const badges: ThumbnailBadge[] = [];
  const metaIcons: ThumbnailBadge['icons'] = [];
  const rating = Number(props.file.rating || 0);
  const cullingFlag = Number(props.file.culling_flag ?? props.file.cullingFlag ?? 0);
  const cullingIcon = cullingFlag === 1
    ? IconFlagFilled
    : cullingFlag === 2
      ? IconFlagOff
      : undefined;
  const cullingIconClass = cullingFlag === 1
    ? 'text-primary'
    : cullingFlag === 2
      ? 'text-error'
      : undefined;

  if (props.file.is_favorite) {
    badges.push({
      key: 'favorite',
      icon: IconStarFilled,
      iconClass: 'text-warning',
      label: rating > 0 ? `${rating}` : undefined,
      trailingIcon: cullingIcon,
      trailingIconClass: cullingIconClass,
    });
  } else if (rating > 0) {
    badges.push({
      key: 'rating',
      icon: IconStarFilled,
      iconClass: 'text-warning',
      label: `${rating}`,
      trailingIcon: cullingIcon,
      trailingIconClass: cullingIconClass,
    });
  } else if (cullingIcon) {
    badges.push({
      key: cullingFlag === 1 ? 'culling-pick' : 'culling-reject',
      icon: cullingIcon,
      iconClass: cullingIconClass,
    });
  }
  
  if (props.file.has_tags) metaIcons.push({ icon: IconTag });
  if (props.file.has_collections) metaIcons.push({ icon: IconBookmark });
  if (props.file.comments?.length > 0) metaIcons.push({ icon: IconComment });
  if (normalizedRotate.value > 0) {
    metaIcons.push({
      icon: IconRotate,
      style: { transform: `rotate(${normalizedRotate.value}deg)` },
    });
  }
  if (metaIcons.length > 0) {
    badges.push({
      key: 'meta',
      icons: metaIcons,
    });
  }

  return badges;
});
</script>

<style scoped>
.face-review-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 20px;
  border: 1px solid #aaa;
  border-radius: 2px;
  background: linear-gradient(#fff, #e5e5e5);
  box-shadow: 0 1px 1px #00000015;
  transition: background-color 80ms, box-shadow 80ms;
}
.face-review-button:hover { background: #fff; border-color: #888; }
.face-review-button:active { background: #d6d6d6; box-shadow: inset 0 1px 2px #00000040; }
.face-review-button:focus-visible { outline: 2px solid #4b83bd; outline-offset: 1px; }
.face-confirmed-feedback { animation: face-confirmed 700ms ease-out; }
@keyframes face-confirmed {
  from { outline: 2px solid #29883b; }
  to { outline: 2px solid transparent; }
}
</style>
