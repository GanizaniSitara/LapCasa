<template>

  <div ref="editorRoot" tabindex="-1" class="flex flex-col overflow-hidden outline-none bg-base-300 text-base-content/70 [contain:strict]"
    :class="embedded ? 'w-full h-full' : 'w-screen h-screen'"
    :data-inline-editor="embedded ? true : undefined" :data-file-id="fileInfo?.id">
    <div v-if="embedded" class="h-9 shrink-0 flex items-center gap-2 px-3">
      <span class="truncate">{{ fileInfo?.name }}</span>
      <!-- Equal grid columns: A, A|B and A|A all take the widest button's size (LAP-105) -->
      <div class="ml-auto grid grid-flow-col auto-cols-fr gap-1" role="group" aria-label="Photo comparison">
        <button class="btn btn-xs" :class="comparisonMode === 'single' ? 'btn-active' : ''" aria-label="Single photo" :aria-pressed="comparisonMode === 'single'" @click="setComparisonMode('single')">A</button>
        <button class="btn btn-xs" :class="comparisonMode === 'photos' ? 'btn-active' : ''" aria-label="Compare two photos" :aria-pressed="comparisonMode === 'photos'" @click="setComparisonMode('photos')">A|B</button>
        <button class="btn btn-xs" :class="comparisonMode === 'original' ? 'btn-active' : ''" aria-label="Compare original and edited" :aria-pressed="comparisonMode === 'original'" @click="setComparisonMode('original')">A|A</button>
      </div>
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
        <div v-if="embedded" data-editor-filmstrip class="h-11 shrink-0 max-w-full flex items-center"
          :style="{ width: `${Math.min(folderFiles.length * 42 + 48, 720)}px` }">
          <button class="w-6 h-10 shrink-0 hover:bg-base-content/10 disabled:opacity-25"
            aria-label="Previous photo" @click="navigatePhoto(-1)">‹</button>
          <VirtualScroll ref="editorStrip" :items="folderFiles" :item-size="42" :grid-items="1"
            class="min-w-0 flex-1 no-scrollbar" direction="horizontal" :buffer="3" :emit-update="true" @update="hydrateFilmstrip">
            <template #default="{ item }">
              <button class="w-10 h-10 p-0.5 border rounded-[2px] overflow-hidden"
                :class="item.id === fileInfo?.id ? 'border-primary' : 'border-transparent'"
                :data-file-id="item.id" :title="item.name || ''" @click="queuePhoto(item.id)">
                <img v-if="item.file_path" :src="getThumbUrl(item.id, false, 0, item.modified_at, item.id === fileInfo?.id ? fileInfo.edits : item.edits)"
                  class="w-full h-full object-contain" draggable="false" alt="" />
              </button>
            </template>
          </VirtualScroll>
          <button class="w-6 h-10 shrink-0 hover:bg-base-content/10 disabled:opacity-25"
            aria-label="Next photo" @click="navigatePhoto(1)">›</button>
        </div>
        <div class="flex w-full flex-1 min-h-0 gap-2" :data-editor-comparison="comparisonMode !== 'single' ? comparisonMode : undefined">
          <figure v-if="comparisonMode !== 'single'" class="relative flex-1 min-w-0 overflow-hidden rounded-box border border-base-content/5"
            :class="comparisonMode === 'photos' ? 'cursor-pointer' : ''"
            :style="{ order: comparisonMode === 'photos' && activeComparisonSide === 'left' ? 1 : 0 }"
            :data-comparison-pane="comparisonMode === 'photos' && activeComparisonSide === 'left' ? 'right' : 'left'"
            :data-file-id="comparisonMode === 'original' ? fileInfo.id : otherComparisonFile?.id"
            :role="comparisonMode === 'photos' ? 'button' : undefined"
            :tabindex="comparisonMode === 'photos' ? 0 : undefined"
            :aria-label="comparisonMode === 'photos' ? 'Select other comparison photo' : 'Original photo'"
            @click="selectOtherComparisonPhoto" @keydown.enter.prevent="selectOtherComparisonPhoto" @keydown.space.prevent="selectOtherComparisonPhoto">
            <figcaption class="absolute left-2 top-2 z-10 bg-base-300/80 px-1 rounded">{{ comparisonMode === 'original' ? 'Original' : otherComparisonFile?.name }}</figcaption>
            <span class="absolute left-2 top-8 z-10 bg-base-100/90 text-base-content px-1.5 rounded-full text-[10px] font-semibold pointer-events-none"
              data-comparison-filter-pill="other">{{ otherPaneFilterPill }}</span>
            <img :src="comparisonMode === 'original' ? originalComparisonSrc : otherComparisonSrc" class="block w-full h-full object-contain" :alt="comparisonMode === 'original' ? 'Original photo' : otherComparisonFile?.name" draggable="false" />
          </figure>
          <div
          ref="containerRef"
          data-editor-image-space
          :data-comparison-pane="comparisonMode === 'single' ? undefined : comparisonMode === 'original' ? 'right' : activeComparisonSide"
          :data-file-id="fileInfo.id"
          :style="{ order: comparisonMode === 'original' || activeComparisonSide === 'right' ? 1 : 0 }"
          class="relative min-w-0 flex-1 rounded-box overflow-hidden border bg-base-300/30 shadow-sm cursor-default"
          :class="[
            comparisonMode === 'single' ? 'border-base-content/5' : 'border-primary',
            activeTool === 'Crop' && cropStatus === 1 && !isPreviewingCrop ? 'cursor-crosshair' : ''
          ]"
          @mousedown="onContainerMouseDown"
          @pointerdown="handlePreviewPointerDown"
          @pointerup="handlePreviewPointerUp"
          @pointerleave="handlePreviewPointerUp"
          @pointercancel="handlePreviewPointerUp"
        >

            <!-- Filmstrip navigation: the previous picture stays exactly in place until the
                 next one is decoded, then swaps in one frame (no stretched preview). -->
            <div v-if="!imageReady && heldImage" ref="heldHostRef"
              class="absolute inset-0 pointer-events-none"></div>


            <span v-if="comparisonMode !== 'single'" class="absolute left-2 top-2 z-40 bg-base-300/80 px-1 rounded pointer-events-none">{{ comparisonMode === 'original' ? 'Edited' : 'Selected' }}</span>
            <!-- Retouch: the blemish (target) and the area copied over it (source) -->
            <template v-if="activeTool === 'Retouch'">
              <div v-for="ring in retouchRings" :key="ring.kind" :data-retouch-ring="ring.kind"
                class="absolute z-40 rounded-full pointer-events-none border-2"
                :class="ring.kind === 'target' ? 'border-red-500' : 'border-dashed border-sky-400'"
                :style="{ left: `${ring.x - ring.r}px`, top: `${ring.y - ring.r}px`, width: `${ring.r * 2}px`, height: `${ring.r * 2}px`, boxShadow: '0 0 0 1px rgba(0,0,0,0.5)' }"></div>
            </template>
            <span v-if="comparisonMode !== 'single'" class="absolute left-2 top-8 z-40 bg-base-100/90 text-base-content px-1.5 rounded-full text-[10px] font-semibold pointer-events-none"
              data-comparison-filter-pill="selected">{{ selectedPaneFilterPill }}</span>

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
              <EffectMask v-if="maskMode && imageReady" ref="effectMaskRef" :width="imageWidth" :height="imageHeight"
                :layer-style="maskLayerStyle" :effect-src="effectPreviewSrc" :mode="maskTool" :brush="maskBrush"
                :feather="maskFeather" :inverted="maskInverted" :show-overlay="maskOverlay" :scale="scale" :to-image="clientToImage"
                :region="maskRegion" :points="selectPoints" :outline="activeTool === 'Automask'" :hover-src="hoverMaskSrc"
                @select-click="onSelectClick" @select-hover="onSelectHover" @select-leave="onSelectLeave" />
            </template>

            <div v-if="(cropStatus === 1 && (activeTool === 'Crop' ? cropBoxDrawn : true)) || (!activeTool && cropApplied)"
              :class="[
                cropStatus === 1 ? (isPreviewingCrop ? 'crop-box-preview' : 'crop-box-active') : 'crop-box-done',
                isResizing ? 'no-transition' : '',
                cropStatus === 1 && !isPreviewingCrop
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
              @mousedown.stop="cropStatus===1 && !isPreviewingCrop ? startDrag('move', $event) : null"
              @dblclick="clickDoCrop"
            >
              <div v-if="activeTool === 'Straighten' && cropStatus === 1" data-straighten-grid class="straighten-grid"></div>
              <template v-if="cropStatus===1 && !cropBoxFixed && !isPreviewingCrop">
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

        </div>

        <!-- Straighten: angle slider and Apply / Cancel right under the photo (LAP-102) -->
        <div v-if="activeTool === 'Straighten'" data-straighten-bar class="shrink-0 w-full max-w-[720px] flex items-center gap-2 text-[11px]"
          @pointerdown="isRotating = true" @pointerup="isRotating = false" @pointercancel="isRotating = false">
          <AngleSlider v-model="cropAngleModel" :min="-20" :max="20" :step="0.1" :minor-step="1" :major-step="5" :snap-zero="0.05"
            :aria-label="$t('msgbox.image_editor.angle')" class="flex-1 min-w-0" />
          <div class="flex items-center gap-0.5 shrink-0">
            <input
              ref="straightenInputRef"
              v-model="straightenAngleInput"
              type="text"
              inputmode="decimal"
              class="input input-bordered input-xs w-14 text-right font-mono text-xs px-1 h-7"
              placeholder="0.0"
              aria-label="Straighten angle"
              data-testid="straighten-angle-input"
              @keydown.stop="onStraightenInputKeyDown"
              @blur="onStraightenInputBlur"
              @focus="onStraightenInputFocus"
            />
            <span class="text-xs font-mono text-base-content/70">°</span>
          </div>
          <button class="btn btn-sm btn-primary w-20" data-testid="straighten-apply" @click="clickDoCrop();activeTool=''">Apply</button>
          <button class="btn btn-sm btn-ghost w-20" data-testid="straighten-cancel" @click="cancelActiveTool">Cancel</button>
        </div>

      </div>

      <aside class="order-first w-[278px] shrink-0 flex flex-col min-h-0 text-[11px]" data-testid="picasa-editor">
        <!-- Back to the library alone on the top row; Undo / Redo sit under the tools, away from it -->
        <div v-if="embedded" class="flex items-center mb-1.5" data-editor-header>
          <button class="btn btn-sm btn-ghost shrink-0 px-1.5 text-[11px] gap-0.5" data-testid="editor-back" @click="clickCancel"><span aria-hidden="true">‹</span><span>Back to Library</span></button>
        </div>
        <div role="tablist" class="flex border-b border-base-content/15">
          <button v-for="tab in editorTabs" :key="tab.key" role="tab" :aria-selected="editorTab===tab.key" :title="editorTabShortcut(tab.key)"
            class="flex-1 px-1 py-2 border border-base-content/10 rounded-t text-[11px]"
            :class="editorTab===tab.key ? 'bg-base-100 font-bold' : 'bg-base-300'"
            :disabled="cropStatus===1 || isProcessing" @click="editorTab=tab.key">{{ tab.label }}</button>
        </div>
        <div class="flex-1 min-h-0 overflow-y-auto p-2" :class="isProcessing ? 'pointer-events-none opacity-60' : ''">
          <!-- LAP-107: Automask step 1, where it applies: always the first row -->
          <template v-if="activeTool && MASK_EFFECTS.includes(activeTool)">
            <p class="font-bold" data-automask-step="1">1. Where it applies</p>
            <p class="text-[10px] opacity-70 mb-1">Point at the photo and click an area, or pick one here.</p>
            <div class="flex flex-wrap items-center gap-1" data-testid="effect-apply-to" role="group" aria-label="Where it applies">
              <button class="btn btn-xs" :class="!maskMode && !lightKeepsMask ? 'btn-active' : ''" data-apply="whole" :aria-pressed="!maskMode && !lightKeepsMask"
                :title="`The whole photo (${regionKey('whole')})`" @click="chooseWholePhoto()">Whole photo<span class="opacity-50 ml-0.5">{{ regionKey('whole') }}</span></button>
              <button v-for="r in MASK_REGIONS" :key="r.key" class="btn btn-xs" :class="(maskMode && maskRegion === r.key) || (lightKeepsMask && lightDraft.region === r.key) ? 'btn-active' : ''"
                :data-apply="r.key" :aria-pressed="maskMode && maskRegion === r.key" :disabled="!!selectDisabledReason || isProcessing"
                :title="selectDisabledReason || `${r.title} (${regionKey(r.key)})`" @click="applyRegion(r.key)">{{ selectBusy && selectPreparing && maskRegion === r.key ? 'Preparing selection...' : r.label }}<span class="opacity-50 ml-0.5">{{ regionKey(r.key) }}</span></button>
              <button class="btn btn-xs" :class="maskMode && maskTool === 'select' && !maskRegion ? 'btn-active' : ''" data-apply="select"
                :aria-pressed="maskMode && maskTool === 'select' && !maskRegion" :disabled="!!selectDisabledReason || isProcessing"
                :title="selectDisabledReason || 'Click an object in the photo to select it; Shift+click adds, Alt+click takes away (S)'" @click="startMaskTool('select')">Click to select<span class="opacity-50 ml-0.5">S</span></button>
              <button class="btn btn-xs" :class="maskMode && maskTool === 'paint' ? 'btn-active' : ''" data-apply="brush" data-testid="effect-mask"
                :aria-pressed="maskMode && maskTool === 'paint'" title="Brush: touch up the selection, right-drag or Alt erases (B)" @click="startMaskTool('paint')">Brush<span class="opacity-50 ml-0.5">B</span></button>
              <button class="btn btn-xs" :class="maskMode && maskTool === 'gradient' ? 'btn-active' : ''" data-apply="gradient" data-testid="effect-mask-gradient"
                :aria-pressed="maskMode && maskTool === 'gradient'" title="Linear gradient with two handles (G)" @click="startMaskTool('gradient')">Gradient<span class="opacity-50 ml-0.5">G</span></button>
            </div>
            <p v-if="!maskMode" class="text-[10px] opacity-70" data-light-hint>{{ lightKeepsMask ? `Applies to: ${lightLabel(lightDraft)} (kept). Pick another area to change it.` : 'Applies to the whole photo.' }}</p>
            <p v-if="maskMode" class="text-[10px] opacity-70" data-mask-hint>
              <template v-if="selectBusy && selectPreparing">Preparing selection...</template>
              <template v-else-if="selectBusy">Selecting…</template>
              <template v-else-if="selectError">{{ selectError }}</template>
              <template v-else-if="maskTool === 'select'">The area under the pointer lights up; click to pick it. Shift+click adds, Alt+click takes away.</template>
              <template v-else-if="maskTool === 'gradient'">Drag the two handles: full effect at the white one, none at the black one.</template>
              <template v-else>Paint to add; right-drag or Alt erases. [&nbsp;]&nbsp;brush&nbsp;{{ Math.round(maskBrush * 100) }}%.</template>
              Shift+[&nbsp;]&nbsp;feather&nbsp;{{ Math.round(maskFeather * 1000) / 10 }}% · I&nbsp;invert{{ maskInverted ? ' (on)' : '' }} · X&nbsp;show&nbsp;mask · Backspace&nbsp;clear
            </p>
          </template>
          <template v-if="!activeTool">
            <template v-if="editorTab==='basic'">
              <div class="grid grid-cols-3 gap-x-1 gap-y-2 pt-1 pb-2">
                <template v-for="tool in basicTools" :key="tool">
                  <button class="flex flex-col items-center gap-1 hover:bg-base-100 rounded py-1"
                    :class="tool === &quot;I'm Feeling Lucky&quot; && savedEditState.auto_enhance ? 'bg-primary/15 ring-1 ring-primary/40' : ''"
                    :data-editor-tool="tool" :disabled="cropStatus===1 || !imageReady" @click="selectPicasaTool(tool)">
                    <span class="w-12 h-8 flex items-center justify-center text-base-content/70">
                      <span v-if="tool==='Text'" class="font-serif font-bold text-3xl">T</span>
                      <component v-else :is="basicToolIcons[tool]" class="w-7 h-7" />
                    </span>
                    <span class="leading-3 text-center">{{ tool }}</span>
                  </button>
                  <!-- I'm Feeling Lucky is a six-step cycle: each click moves one step (LAP-043 / LAP-085);
                       each step is also its own button, one click and one undoable step (LAP-100).
                       The styles sit straight under the Lucky row, a notch pointing up at Lucky (LAP-105). -->
                  <div v-if="tool==='Auto Color'" class="col-span-3 relative -mt-1 rounded border border-base-content/20 bg-base-300/60 px-1.5 pt-1.5 pb-1"
                    data-lucky-cycle title="I'm Feeling Lucky: 5 styles + Original. Click a style, or click I'm Feeling Lucky to move to the next one.">
                    <span aria-hidden="true" class="absolute -top-[5px] left-[calc(16.67%-5px)] w-2.5 h-2.5 rotate-45 border-l border-t border-base-content/20 bg-base-300"></span>
                    <div class="relative flex items-center gap-1">
                      <button v-for="step in LUCKY_CYCLE" :key="step.index" type="button" :data-lucky-step="step.index"
                        :data-active="step.index === activeLuckyStep ? 'true' : undefined" :aria-pressed="step.index === activeLuckyStep"
                        class="flex-1 h-7 rounded border text-[11px]"
                        :class="step.index === activeLuckyStep ? 'bg-primary text-primary-content border-primary font-bold' : 'border-base-content/25 text-base-content/80 hover:bg-base-100 hover:border-base-content/50'"
                        :disabled="cropStatus===1 || !imageReady" :title="step.title" @click="selectLuckyStep(step.index)">{{ step.short }}</button>
                    </div>
                    <span class="block text-center text-[10px] text-primary font-medium mt-0.5" data-lucky-cycle-label>{{ luckyCycleLabel }}</span>
                  </div>
                </template>
                <!-- Rotation in the third cell of the last row, each button labelled (LAP-105) -->
                <div class="flex items-start justify-center gap-0.5" data-editor-rotation>
                  <button v-for="r in ROTATE_BUTTONS" :key="r.degree" type="button" class="flex-1 flex flex-col items-center gap-1 hover:bg-base-100 rounded py-1"
                    :data-editor-tool="r.degree > 0 ? 'Rotate right' : undefined" :data-rotate="r.degree"
                    :aria-label="r.label" :title="`${r.label} (${getShortcutLabel(r.shortcut)})`"
                    :disabled="cropStatus===1 || cropApplied" @click="clickRotate(r.degree)">
                    <span class="h-8 flex items-center justify-center text-base-content/70"><component :is="r.icon" class="w-6 h-6" /></span>
                    <span class="leading-3 text-center">{{ r.label }}</span>
                  </button>
                </div>
              </div>
              <label class="relative block text-center mt-1">Fill Light
                <input aria-label="Fill Light" type="range" min="0" max="100" v-model.number="picasaTools.fillLight"
                  class="range range-xs w-48 mt-1" title="Double-click to reset" @input="picasaTools.lastStep='Fill Light'"
                  @change="onSliderChange" @dblclick="resetFillLight" />
                <span class="absolute right-1 -top-3.5 text-[10px] tabular-nums text-base-content/60 pointer-events-none" data-fill-light-value>{{ picasaTools.fillLight }}</span>
              </label>
              <button class="btn btn-sm btn-ghost w-full mt-3" :disabled="cropStatus === 1 || isProcessing"
                @click="exportDialog?.showModal()">Export…</button>
            </template>
            <template v-else-if="editorTab==='tuning'">
              <label v-for="control in tuningTools" :key="control.key" class="relative block text-center my-4">{{ control.label }}
                <input :aria-label="control.label" type="range" :min="control.min" max="100" v-model.number="picasaTools[control.key]"
                  :title="control.key==='highlights' ? 'Brightens highlights or darkens blown areas; detail lost at capture cannot be restored.' : control.key==='fillLight' ? 'Double-click to reset' : undefined"
                  class="range range-xs w-48 mt-1" @input="picasaTools.lastStep=control.label" @change="onSliderChange"
                  @dblclick="control.key==='fillLight' && resetFillLight()" />
                <span v-if="control.key==='fillLight'" class="absolute right-1 -top-3.5 text-[10px] tabular-nums text-base-content/60 pointer-events-none" data-fill-light-value>{{ picasaTools.fillLight }}</span>
                <!-- Picasa's whitept (LAP-071): click a neutral area, the temperature follows -->
                <button v-if="control.key==='temperature'" type="button" class="btn btn-xs btn-ghost btn-square align-middle ml-1"
                  :class="activeTool==='Temperature Picker' ? 'btn-active' : ''" title="Pick neutral color" aria-label="Pick neutral color"
                  data-temperature-eyedropper :disabled="!imageReady" @click.prevent.stop="activeTool='Temperature Picker'">
                  <svg viewBox="0 0 24 24" class="w-4 h-4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M14.5 4.5l5 5M17 2.5a2.1 2.1 0 0 1 3 3l-2 2-3-3z" /><path d="M15 7l-9.5 9.5L4 20l3.5-1.5L17 9" />
                  </svg>
                </button>
              </label>
              <div class="text-center mt-3">Neutral Color Picker<br />
                <button class="btn btn-sm btn-ghost mt-1" :class="activeTool==='Neutral Color Picker' ? 'btn-active' : ''"
                  @click="activeTool='Neutral Color Picker'">Pick a neutral color</button>
              </div>
            </template>
            <div v-else data-effects-pages>
              <!-- Three pages of up to twelve effects, so none needs scrolling (LAP-100) -->
              <div role="tablist" aria-label="Effects pages" class="flex gap-1">
                <button v-for="(page, i) in effectPages" :key="i" role="tab" :aria-selected="effectPage===i" :data-effects-page="i+1"
                  class="flex-1 h-4 rounded-sm border text-[10px] leading-[14px]"
                  :class="effectPage===i ? 'bg-base-100 border-base-content/30 font-bold' : 'border-base-content/15 text-base-content/60 hover:bg-base-100'"
                  :title="`Effects ${i+1} of ${effectPages.length}`" @click="effectPage=i">{{ i+1 }}</button>
              </div>
              <div class="grid grid-cols-3 gap-1 py-1">
                <button v-for="effect in effectPages[effectPage]" :key="effect" class="border border-base-content/15 p-1 hover:bg-base-100 rounded-sm"
                  :data-editor-tool="effect" :disabled="!imageReady" @click="selectPicasaTool(effect)">
                  <img :src="imageSrc" class="w-full h-11 object-cover" :style="{filter:toolPreview(effect)}" />
                  <span class="block leading-3 mt-1">{{ effect }}</span>
                </button>
              </div>
            </div>
          </template>
          <div v-else :class="activeTool === 'Crop' ? 'py-1' : 'space-y-3 py-2'">
            <h3 v-if="activeTool !== 'Crop'" class="font-bold text-center text-sm flex items-center justify-center gap-1.5">
              <component v-if="activeTool === 'Straighten'" :is="IconStraighten" class="w-5 h-5" data-straighten-title-icon />{{ activeTool }}
            </h3>
            <template v-if="activeTool==='Straighten'">
              <p class="text-center px-1">Line the photo up with the grid using the slider under it. ← / → nudge 0.1°, Shift 1°.</p>
            </template>
            <template v-else-if="activeTool==='Crop'">
              <div class="crop-panel-container">
                <!-- Header: Icon + Heading -->
                <div class="crop-panel-header">
                  <img :src="picasaCropIcon" class="crop-header-icon" alt="" />
                  <span class="crop-header-title">Crop Photo</span>
                </div>

                <!-- Instruction Text -->
                <p class="crop-instruction-text">
                  Select a dimension below and then click and drag on the image to select the portion of the image you wish to crop.
                </p>

                <!-- Dimension Dropdown -->
                <div class="crop-dropdown-wrap">
                  <select
                    v-model="selectedAspectPresetId"
                    class="crop-dimension-select"
                    data-testid="crop-aspect-select"
                    @change="onChangeCropPreset"
                  >
                    <option v-for="preset in picasaAspectPresets" :key="preset.id" :value="preset.id">
                      {{ preset.label }}
                    </option>
                  </select>
                </div>

                <!-- Row of 3 Suggested Crop Thumbnails -->
                <div class="crop-thumbnails-row">
                  <button
                    v-for="(suggestion, idx) in cropSuggestions"
                    :key="idx"
                    type="button"
                    class="crop-thumbnail-card"
                    :class="{ 'crop-thumbnail-card-active': activeSuggestionIndex === idx }"
                    :title="suggestion.label"
                    @click="applyCropSuggestion(idx)"
                  >
                    <div class="crop-thumbnail-inner">
                      <img
                        :src="displayImageSrc || imageSrc"
                        class="crop-thumbnail-img"
                        :style="{ objectPosition: suggestion.objectPosition }"
                        alt=""
                      />
                    </div>
                  </button>
                </div>

                <!-- Rotate + Preview buttons side by side -->
                <div class="crop-btn-row">
                  <button
                    type="button"
                    class="picasa-btn flex-1"
                    :disabled="!cropBoxDrawn || selectedAspectPresetId === '1:1'"
                    data-testid="crop-rotate-aspect"
                    @click="toggleCropOrientation"
                  >
                    Rotate
                  </button>
                  <button
                    type="button"
                    class="picasa-btn flex-1"
                    :disabled="!cropBoxDrawn"
                    data-testid="crop-preview"
                    @click="toggleCropPreview"
                  >
                    {{ isPreviewingCrop ? 'Back' : 'Preview' }}
                  </button>
                </div>

                <!-- Reset centered below -->
                <div class="crop-btn-row-center">
                  <button
                    type="button"
                    class="picasa-btn crop-btn-reset"
                    :disabled="!cropBoxDrawn"
                    data-testid="crop-reset"
                    @click="resetCropBox"
                  >
                    Reset
                  </button>
                </div>

                <!-- Apply + Cancel side by side -->
                <div class="crop-btn-row">
                  <button
                    type="button"
                    class="picasa-btn flex-1"
                    data-testid="crop-apply"
                    @click="clickDoCrop(); activeTool=''"
                  >
                    <span>Apply</span>
                    <svg class="crop-btn-icon" viewBox="0 0 16 16" fill="none">
                      <circle cx="8" cy="8" r="7.5" :fill="cropBoxDrawn ? '#3da851' : '#b7d9bd'" />
                      <path d="M4.5 8.2l2.5 2.5 4.5-5" stroke="#ffffff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
                    </svg>
                  </button>
                  <button
                    type="button"
                    class="picasa-btn flex-1"
                    data-testid="crop-cancel"
                    @click="clickCancelCrop(); restoreEditorControls(); activeTool=''"
                  >
                    <span>Cancel</span>
                    <svg class="crop-btn-icon" viewBox="0 0 16 16" fill="none">
                      <circle cx="8" cy="8" r="7.5" fill="#cc3333" />
                      <path d="M5.5 5.5l5 5M10.5 5.5l-5 5" stroke="#ffffff" stroke-width="1.8" stroke-linecap="round" />
                    </svg>
                  </button>
                </div>
              </div>
            </template>
            <template v-else-if="activeTool==='Retouch'">
              <p>{{ retouchTarget ? (retouchSource ? 'The photo shows the result. Click again to pick another source area.' : 'Click an area to copy over the blemish.') : 'Click the blemish you want to retouch.' }}</p>
              <label>Brush size<input aria-label="Brush size" type="range" v-model.number="brushSize" min="1" max="20" class="range range-xs" /></label>
              <button class="btn btn-sm" :disabled="!retouchSource" @click="applyRetouch">Apply</button>
              <button class="btn btn-sm btn-ghost" @click="activeTool=''">Cancel</button>
            </template>
            <template v-else-if="activeTool==='Text'">
              <p>Type your text, then click the photo to place it.</p>
              <input aria-label="Text" v-model="toolText" class="input input-sm w-full" />
              <label>Font<select aria-label="Font" v-model="toolFont" class="select select-sm w-full"><option>Arial</option><option>Helvetica</option><option>Consolas</option></select></label>
              <label>Size<input aria-label="Text size" v-model.number="textSize" type="number" min="12" max="96" class="input input-sm w-16" /></label>
              <div class="flex gap-1" role="group" aria-label="Text style">
                <button type="button" class="btn btn-xs font-bold" :class="textBold ? 'btn-active' : ''" aria-label="Bold" :aria-pressed="textBold" @click="textBold = !textBold">B</button>
                <button type="button" class="btn btn-xs italic font-serif" :class="textItalic ? 'btn-active' : ''" aria-label="Italic" :aria-pressed="textItalic" @click="textItalic = !textItalic">I</button>
                <button type="button" class="btn btn-xs" :class="textOutline ? 'btn-active' : ''" aria-label="Outline" :aria-pressed="textOutline" @click="textOutline = !textOutline">Outline</button>
              </div>
              <label>Color<input aria-label="Text color" v-model="effectColor" type="color" /></label>
              <button class="btn btn-sm" :disabled="!toolText" @click="applyText">Apply</button>
              <button class="btn btn-sm btn-ghost" @click="activeTool=''">Cancel</button>
            </template>
            <template v-else-if="activeTool==='Automask'">
              <div class="flex items-center justify-between mt-3">
                <p class="font-bold" data-light-panel>2. Adjust{{ lightEditing >= 0 ? ` (Automask ${lightEditing + 1})` : '' }}</p>
                <button class="btn btn-xs" data-testid="automask-auto" :disabled="lightAutoDisabled"
                  :title="lightKeepsMask ? 'Pick the area again to use Auto' : 'Set the four sliders from the brightness of this area'" @click="autoLight">Auto</button>
              </div>
              <label v-for="sl in LIGHT_SLIDERS" :key="sl.key" class="block my-1.5" :title="sl.title">{{ sl.label }}
                <span class="float-right tabular-nums">{{ lightDraft[sl.key] > 0 ? '+' : '' }}{{ lightDraft[sl.key] }}</span>
                <span class="relative block">
                  <span class="absolute left-1/2 top-0 bottom-0 w-px bg-base-content/50 pointer-events-none z-10" aria-hidden="true"></span>
                  <input :aria-label="`Automask ${sl.label}`" :data-light-slider="sl.key" type="range" min="-100" max="100" step="1"
                    v-model.number="lightDraft[sl.key]" class="range range-xs w-full [--range-fill:0]" @dblclick="lightDraft[sl.key] = 0" />
                </span>
              </label>
              <p class="text-[10px] opacity-60">Left darker, right lighter. Double-click a slider to reset it.</p>
              <div class="flex gap-1 mt-2" data-automask-actions>
                <button class="btn btn-sm btn-primary flex-1" data-testid="light-apply" :disabled="selectBusy" @click="commitLight">Apply<span class="opacity-70 ml-1 text-[10px]">Enter</span></button>
                <button class="btn btn-sm flex-1" data-testid="light-cancel" @click="cancelActiveTool">Cancel<span class="opacity-70 ml-1 text-[10px]">Esc</span></button>
              </div>
              <details class="mt-2" data-automask-curve :open="lightCurveBent">
                <summary class="cursor-pointer text-[11px]">Curve (optional)</summary>
                <PointCurves :model-value="lightCurveModel" :channels="['rgb']" :result="lightResult"
                  @preview="lightCurvePreview = $event ? $event.rgb : null" @change="onLightCurve" />
              </details>
            </template>
            <template v-else-if="activeTool==='Neutral Color Picker'">
              <p>Click a white or gray area of the photo.</p>
              <button class="btn btn-sm btn-ghost" @click="activeTool=''">Cancel</button>
            </template>
            <template v-else-if="activeTool==='Temperature Picker'">
              <p>Click something in the photo that should be white or gray: Color Temperature is set so it turns neutral.</p>
              <button class="btn btn-sm btn-ghost" @click="activeTool=''">Cancel</button>
            </template>
            <template v-else>
              <label>Amount<input aria-label="Effect amount" v-model.number="effectAmount" type="range" min="0" max="100" class="range range-xs w-full" /></label>
              <label v-if="['Tint','Filtered B&W','Graduated Tint'].includes(activeTool)">Color<input aria-label="Effect color" v-model="effectColor" type="color" /></label>
              <p v-if="['Soft Focus','Focal B&W','Focal Zoom'].includes(activeTool)">Click the photo to choose the center.</p>
              <button class="btn btn-sm" @click="maskMode ? commitMask() : applyPicasaEffect()">Apply</button>
              <button class="btn btn-sm btn-ghost" @click="activeTool=''">Cancel</button>
            </template>
          </div>
          <!-- LAP-107: the Automask list - what is applied, each switchable, reopenable, removable -->
          <div v-if="activeTool === 'Automask' || (!activeTool && editorTab === 'tuning')" class="mt-4 pt-2 border-t border-base-content/15" data-light-list>
            <div class="flex items-center justify-between gap-1">
              <span class="font-bold">{{ activeTool === 'Automask' ? '3. Your Automasks' : 'Automask' }}</span>
              <span v-if="!activeTool" class="flex gap-1">
                <button class="btn btn-xs" data-testid="automask-balance" :disabled="!imageReady || cropStatus===1 || isProcessing"
                  title="Find the too-dark and too-bright areas and balance each one (one undo step)" @click="autoBalance">{{ balancing ? 'Balancing…' : 'Auto Balance' }}</button>
                <button class="btn btn-xs" data-testid="light-add" :disabled="!imageReady || cropStatus===1"
                  :title="`Add an Automask: pick an area, then lift or hold back its brightness (${getShortcutLabel('editor.automask')})`" @click="openLight(-1)">Add<span class="opacity-50 ml-0.5">{{ getShortcutLabel('editor.automask') }}</span></button>
              </span>
            </div>
            <p v-if="lightStatus" class="text-[10px] mt-1" data-automask-status>{{ lightStatus }}</p>
            <p v-if="!lightList.length" class="text-[10px] opacity-70 mt-1">None yet. Auto Balance finds the too-dark and too-bright areas; Add picks one yourself.</p>
            <div v-for="(adj, i) in lightList" :key="i" class="flex items-center gap-1 mt-1 rounded px-0.5" :data-light-item="i"
              :class="activeTool === 'Automask' && i === lightEditing ? 'bg-primary/15' : ''">
              <input type="checkbox" class="checkbox checkbox-xs" :checked="adj.enabled !== false" :aria-label="`Use Automask ${i + 1}`"
                :title="adj.enabled === false ? 'Off: switch it on' : 'On: switch it off'" @change="toggleLight(i)" />
              <button class="flex-1 min-w-0 text-left truncate hover:underline" :class="adj.enabled === false ? 'opacity-50 line-through' : ''"
                :title="`Change: ${lightSummary(adj)}`" @click="openLight(i)">{{ i + 1 }}. {{ lightLabel(adj) }}: {{ lightSummary(adj) }}</button>
              <span class="text-[10px] opacity-60 shrink-0">{{ adj.enabled === false ? 'off' : 'on' }}</span>
              <button class="btn btn-xs btn-ghost px-1" :aria-label="`Remove Automask ${i + 1}`" title="Remove" @click="removeLight(i)">×</button>
            </div>
          </div>
        </div>
        <div class="shrink-0 px-2 pb-2">
          <!-- A long step name wraps to a second line inside the button rather than being cut off;
               normal weight keeps the usual ones ("Undo I'm Feeling Lucky") on one line (LAP-105) -->
          <div class="flex gap-1 mt-3" data-editor-history>
            <button class="btn btn-sm flex-1 min-w-0 text-[11px] px-1 relative" :disabled="!savedEditState.history?.length || isProcessing"
              :title="savedEditState.history?.length ? `Undo ${historyStepName(savedEditState)}` : 'Undo'"
              data-testid="editor-undo" @click="queueHistory('undo')"><span class="leading-3 break-words font-normal">{{ savedEditState.history?.length ? `Undo ${historyStepName(savedEditState)}` : 'Undo' }}</span><svg v-if="savedEditState.history?.length && stepMasked(savedEditState)" data-mask-badge class="absolute top-0.5 right-0.5" width="9" height="9" viewBox="0 0 10 10" aria-label="masked"><title>Applied through a mask</title><circle cx="5" cy="5" r="4.2" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M5 .8A4.2 4.2 0 0 1 5 9.2Z" fill="currentColor"/></svg></button>
            <button class="btn btn-sm flex-1 min-w-0 text-[11px] px-1 relative" :disabled="!savedEditState.redo?.length || isProcessing"
              :title="savedEditState.redo?.length ? `Redo ${historyStepName(savedEditState.redo.at(-1))}` : 'Redo'"
              data-testid="editor-redo" @click="queueHistory('redo')"><span class="leading-3 break-words font-normal">{{ savedEditState.redo?.length ? `Redo ${historyStepName(savedEditState.redo.at(-1))}` : 'Redo' }}</span><svg v-if="savedEditState.redo?.length && stepMasked(savedEditState.redo.at(-1))" data-mask-badge class="absolute top-0.5 right-0.5" width="9" height="9" viewBox="0 0 10 10" aria-label="masked"><title>Applied through a mask</title><circle cx="5" cy="5" r="4.2" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M5 .8A4.2 4.2 0 0 1 5 9.2Z" fill="currentColor"/></svg></button>
          </div>
          <div class="flex gap-1 mt-1">
            <button class="btn btn-sm btn-ghost flex-1 text-[11px] px-1" :disabled="(!savedEditState.recipe && !savedEditState.auto_enhance) || isProcessing"
              @click="saveRecipe('reset')">Undo All Edits</button>
            <button class="btn btn-sm btn-ghost flex-1 text-[11px] px-1" data-testid="editor-revert-original"
              :title="hasOriginalCopy ? 'Bring back the original saved in .picasaoriginals and clear the edits' : 'Clear the edits and show the original photo'"
              :disabled="!canRevertOriginal || isProcessing" @click="revertToOriginal">Revert to Original</button>
          </div>
          <p v-if="historyError" role="alert" class="text-error">{{ historyError }}</p>
        </div>
        <div class="shrink-0 p-2 mt-2 border border-base-content/10 rounded" data-editor-histogram>
          <ImageHistogram v-if="histogramReady" editor ref="histogramRef" :source="histogramSource" :adjustments="histogramAdjustments" :apply-adjustments="true" :crop="histogramCrop" :rotate="displayRotate" :flip-horizontal="isFlippedX" :flip-vertical="isFlippedY" />
          <!-- Curves fold away behind a toggle, closed by default; the choice is remembered (LAP-105) -->
          <button type="button" class="flex items-center gap-1 mt-1 px-1 rounded hover:bg-base-100 text-[11px]" data-curves-toggle
            :aria-expanded="config.imageEditor.curvesOpen" @click="config.imageEditor.curvesOpen = !config.imageEditor.curvesOpen">
            <span aria-hidden="true" class="inline-block w-2.5 text-center">{{ config.imageEditor.curvesOpen ? '▾' : '▸' }}</span>Curves
          </button>
          <PointCurves v-if="config.imageEditor.curvesOpen" :model-value="picasaTools.curves" :disabled="!imageReady || isProcessing || cropStatus===1"
            @preview="curveDraft=$event" @change="commitCurves" />
          <p class="mt-1">{{ fileInfo?.e_model }} · {{ imageWidth }} × {{ imageHeight }}</p>
        </div>
      </aside>
    </div>

    <StatusBar v-if="fileInfo" is-embedded show-scale :show-position="embedded" class="shrink-0"
      :selected-file="{ ...fileInfo, width: imageWidth || fileInfo.width, height: imageHeight || fileInfo.height }"
      :selected-item-index="folderIndex" :total-file-count="folderFiles.length" :image-scale="scale" />

    <div v-if="fileInfo && !embedded" data-editor-toolbar class="h-10 shrink-0 flex items-center px-4 gap-2">
      <button
        class="px-4 py-1 rounded-box hover:bg-base-100 hover:text-base-content cursor-pointer text-sm mr-4"
        @click="clickCancel"
      >Back to Library</button>
    </div>

    <dialog ref="exportDialog" class="modal" aria-labelledby="editor-export-title">
      <div class="modal-box max-w-sm bg-base-200 text-base-content">
        <h2 id="editor-export-title" class="font-medium mb-3">Export a copy</h2>
        <p class="text-sm mb-4">Create a separate image with your edits applied.</p>
        <label class="flex items-center justify-between gap-3 text-sm">
          Format and quality
          <select v-model="combinedFormatKey" class="select select-bordered select-sm" aria-label="Format and quality">
            <option v-for="option in combinedFormatOptions" :key="option.value" :value="option.value">{{ option.label }}</option>
          </select>
        </label>
        <form method="dialog" class="modal-action">
          <button class="btn btn-sm" autofocus>Cancel</button>
          <button type="button" class="btn btn-sm btn-primary" :disabled="cropStatus === 1 || isProcessing"
            @click="exportDialog?.close(); clickExport()">Save a Copy</button>
        </form>
      </div>
    </dialog>
  </div>


</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, nextTick, watch, type CSSProperties } from 'vue';
import { useRouter } from 'vue-router';
import { useUIStore } from '@/stores/uiStore';
import { useI18n } from 'vue-i18n';
import { config } from '@/common/config';
import { isWin, isLinux, setTheme, SCALE_VALUES, getFolderPath, getFileExtension, shortenFilename, getFullPath, combineFileName, getSelectOptions, getAssetSrc, getPreviewUrl, getThumbUrl, getLapUrl, shouldUseBackendPreview, isEditableTarget, editHistoryAction, imageEditState } from '@/common/utils';
import { getShortcutLabel, matchesShortcut } from '@/common/shortcuts';
import { sliderSteps, latestDraft } from '@/common/sliderSteps';
import { editImage, checkFileExists, getFileInfo, getQueryFileIds, getFilesByIds, saveImageRecipe, restoreImageRecipe } from '@/common/api';
import { loadFolderSections } from '@/common/folderSections';
import { getMaxInscribedRect, clampCenterInRotatedRect, maxResizeT } from '@/common/geometry';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { emit as tauriEmit, listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { ask as askDialog } from '@tauri-apps/plugin-dialog';

import TitleBar from '@/components/TitleBar.vue';
import StatusBar from '@/components/StatusBar.vue';
import SliderInput from '@/components/SliderInput.vue';
import AngleSlider from '@/components/AngleSlider.vue';
import ImageHistogram from '@/components/ImageHistogram.vue';
import PointCurves from '@/components/PointCurves.vue';
import EffectMask, { type SelectPoint } from '@/components/EffectMask.vue';
import { applyCurves, curveLut, identityCurves, type Curves, type CurveChannel, type CurvePoint } from '@/common/curves';
import { bend as automaskBend } from '@/common/automask';
import VirtualScroll from '@/components/VirtualScroll.vue';
import picasaCropIcon from '@/assets/images/picasa_crop_icon.png';

import {
  IconCrop,
  IconCropLandscape,
  IconZoomIn,
  IconZoomOut,
  IconRotateLeft,
  IconStraighten,
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
const exportDialog = ref<HTMLDialogElement | null>(null);
const emit = defineEmits(['back']);
const router = useRouter();
const fileInfo = ref<any>(props.initialFile ? { ...props.initialFile } : null);
const comparisonMode = ref<'single' | 'photos' | 'original'>('single');
const activeComparisonSide = ref<'left' | 'right'>('left');
const otherComparisonFile = ref<any>(null);
let otherComparisonFolder = '';
let otherComparisonFiles: any[] = [];
// A|A's Original: never edited (original=1), and no per-edit cache buster (no file path), so
// edit steps neither change nor reload it (LAP-087).
const originalComparisonSrc = computed(() => {
  const file = fileInfo.value;
  if (!file?.id) return '';
  const src = getPreviewUrl(file.id, undefined, false, Number(file.modified_at || 0), config.settings.rawThumbnailSource);
  return `${src}${src.includes('?') ? '&' : '?'}original=1`;
});
// A|B's other pane shows its photo as it was when it became the other pane: a snapshot. When both
// panes are the same photo, edits to the selected pane must not reach it (LAP-087): its URL names
// the snapshot's revision (er=, rendered as that revision), never the live cache buster, and a
// snapshot without edits asks for the unedited photo.
const otherComparisonSrc = computed(() => {
  const file = otherComparisonFile.value;
  if (!file?.id) return '';
  const same = file.id === fileInfo.value?.id;
  const src = getPreviewUrl(file.id, same ? undefined : file.file_path, false, Number(file.modified_at || 0), config.settings.rawThumbnailSource, file.edits);
  const snapshot = imageEditState(file.edits);
  return same && !snapshot.revision && !snapshot.auto_enhance ? `${src}${src.includes('?') ? '&' : '?'}original=1` : src;
});
const folderFiles = ref<any[]>(props.initialFile ? [{ ...props.initialFile }] : []);
const folderIndex = computed(() => folderFiles.value.findIndex(file => file.id === fileInfo.value?.id));
const editorStrip = ref<any>(null);
const editorRoot = ref<HTMLElement | null>(null);
let editorUnmounted = false;
const fetchingStripIds = new Set<number>();
let photoNavigation: Promise<void> = Promise.resolve();

function centerFilmstripSelection() {
  const strip = editorStrip.value;
  if (!strip?.$el) return;
  const width = strip.$el.clientWidth;
  const max = Math.max(0, folderFiles.value.length * 42 - width);
  strip.scrollToPosition(Math.max(0, Math.min(max, (folderIndex.value + 0.5) * 42 - width / 2)));
}

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

// The folder the filmstrip shows; it moves on when navigation runs past an album's end.
let stripFolder = String(props.folderQuery?.searchFolder || '');

function setComparisonMode(mode: 'single' | 'photos' | 'original') {
  photoNavigation = photoNavigation.then(() => {
    if (mode === 'photos' && comparisonMode.value !== 'photos') {
      otherComparisonFile.value = { ...fileInfo.value };
      otherComparisonFolder = stripFolder;
      otherComparisonFiles = folderFiles.value;
      activeComparisonSide.value = 'left';
    }
    comparisonMode.value = mode;
  }).catch(error => { historyError.value = String(error); });
}

function selectOtherComparisonPhoto() {
  photoNavigation = photoNavigation.then(async () => {
    if (comparisonMode.value !== 'photos' || !otherComparisonFile.value) return;
    await historyNavigation;
    await flushRecipeSteps();
    if (editorUnmounted) return;
    const otherId = otherComparisonFile.value.id;
    otherComparisonFile.value = { ...fileInfo.value };
    [stripFolder, otherComparisonFolder] = [otherComparisonFolder, stripFolder];
    [folderFiles.value, otherComparisonFiles] = [otherComparisonFiles, folderFiles.value];
    activeComparisonSide.value = activeComparisonSide.value === 'left' ? 'right' : 'left';
    await switchPhoto(otherId);
    await nextTick();
    centerFilmstripSelection();
  }).catch(error => { historyError.value = String(error); });
}

async function loadFolderFilmstrip() {
  const ids = await getQueryFileIds(props.folderQuery);
  if (editorUnmounted) return;
  folderFiles.value = (ids || [fileInfo.value.id]).map((id: number) => id === fileInfo.value.id ? { ...fileInfo.value } : { id });
  await nextTick();
  centerFilmstripSelection();
  await hydrateFilmstrip(Math.max(0, folderIndex.value - 3), Math.min(folderFiles.value.length - 1, folderIndex.value + 20));
}

async function switchPhoto(fileId: number) {
  if (editorUnmounted || fileId === fileInfo.value?.id) return;
  await historyNavigation;
  await flushRecipeSteps();
  if (editorUnmounted) return;
  if (otherComparisonFile.value?.id === fileInfo.value?.id) otherComparisonFile.value = { ...fileInfo.value };
  const previousIndex = folderIndex.value;
  if (previousIndex >= 0) folderFiles.value[previousIndex] = { ...fileInfo.value };
  restoringHistory = true;
  try {
    ++autoPresetRequestId;
    activeTool.value = '';
    showDiffPreview.value = false;
    uiStore.clearActiveAdjustments();
    isProcessing.value = true;
    // Keep the decoded element itself (a fresh <img> would need a frame to decode).
    heldElement = imageReady.value ? imageRef.value : null;
    heldImage.value = heldElement ? true : null;
    imageReady.value = false;
    imageSrc.value = '';
    initialImageSrc.value = '';
    curvePreviewSrc.value = '';
    fileInfo.value = { ...(folderFiles.value.find(file => file.id === fileId) || { id: fileId }) };
    await loadFileInfo(fileId);
    if (editorUnmounted) return;
    await nextTick();
    await initEditImage();
    await nextTick();
    const index = folderIndex.value;
    if (index >= 0) folderFiles.value[index] = { ...fileInfo.value };
    centerFilmstripSelection();
  } finally { restoringHistory = false; }
}

function queuePhoto(fileId: number) {
  photoNavigation = photoNavigation.then(() => switchPhoto(fileId)).catch(error => { historyError.value = String(error); });
}

// Past the first or last photo, continue into the neighbouring folder in the main
// window's Folders order (as Up/Down does there), skipping folders without pictures.
async function crossFolder(direction: number) {
  if (!props.folderQuery || !stripFolder) return;
  const sections = await loadFolderSections();
  const start = sections.findIndex(section => section.path === stripFolder);
  if (start < 0) return;
  for (let i = start + direction; i >= 0 && i < sections.length; i += direction) {
    const ids: number[] = (await getQueryFileIds({ ...props.folderQuery, searchFolder: sections[i].path })) || [];
    if (editorUnmounted) return;
    if (!ids.length) continue;
    stripFolder = sections[i].path;
    folderFiles.value = ids.map(id => ({ id }));
    await switchPhoto(direction > 0 ? ids[0] : ids[ids.length - 1]);
    await hydrateFilmstrip(Math.max(0, folderIndex.value - 20), Math.min(folderFiles.value.length - 1, folderIndex.value + 20));
    return;
  }
}

function waitForImageReady(timeoutMs = 3000) {
  return new Promise<void>(resolve => {
    if (imageReady.value) return resolve();
    const timer = setTimeout(() => { stop(); resolve(); }, timeoutMs);
    const stop = watch(imageReady, ready => { if (ready) { clearTimeout(timer); stop(); resolve(); } });
  });
}

// Presses that arrive while a photo is loading add up into one jump, so a held arrow key
// skips photos instead of queueing a full-size decode for each one.
let pendingSteps = 0;
function navigatePhoto(direction: number) {
  pendingSteps += direction;
  photoNavigation = photoNavigation.then(async () => {
    const steps = pendingSteps;
    pendingSteps = 0;
    if (!steps) return;
    const last = folderFiles.value.length - 1;
    const target = folderIndex.value + steps;
    if (target >= 0 && target <= last) await switchPhoto(folderFiles.value[target].id);
    else if (folderIndex.value === (steps > 0 ? last : 0)) await crossFolder(Math.sign(steps));
    else await switchPhoto(folderFiles.value[steps > 0 ? last : 0].id);
    await waitForImageReady();
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

function applyFileInfo(file: any) {
  fileInfo.value = { ...file, thumbnail: getLapUrl(file.id, false, config.settings.thumbnailSize, Number(file.modified_at || 0), file.edits) };
  newFileName.value = file.name?.substring(0, file.name.lastIndexOf('.')) || file.name || '';
  const src = getPreviewUrl(file.id, file.file_path, false, Number(file.modified_at || 0), config.settings.rawThumbnailSource);
  initialImageSrc.value = `${src}${src.includes('?') ? '&' : '?'}original=1`;
}

async function loadFileInfo(fileId: number) {
  try {
    imageReady.value = false;
    const file = await getFileInfo(fileId);
    if (editorUnmounted) return;
    if (file) {
      applyFileInfo(file);
    }
  } catch {
    await closeEditorWindow();
  }
}

const isProcessing = ref(props.embedded);
const imageReady = ref(false);
const histogramReady = ref(false);
watch(imageReady, ready => { if (!ready) histogramReady.value = false; }, { flush: 'sync' });
// The picture shown while the next filmstrip photo loads (see template).
const heldImage = ref<true | null>(null);
const heldHostRef = ref<HTMLElement | null>(null);
let heldElement: HTMLImageElement | null = null;
watch(heldHostRef, host => {
  if (!host || !heldElement) return;
  heldElement.dataset.editorHeldImage = '';
  host.appendChild(heldElement);
}, { flush: 'post' });
const activeEditorTab = ref<'edit' | 'adjust'>('edit');
const editorTabs = [{key:'basic',label:'Basic Fixes'},{key:'tuning',label:'Tuning'},{key:'effects',label:'Effects'}];
const editorTab = ref('basic');
const basicTools = ['Crop','Straighten','Redeye',"I'm Feeling Lucky",'Auto Contrast','Auto Color','Retouch','Text'];
const basicToolIcons:Record<string,any> = {Crop:IconCrop,Straighten:IconStraighten,Redeye:IconUnhide,
  "I'm Feeling Lucky":IconWand,'Auto Contrast':IconAdjustments,'Auto Color':IconPalette,Retouch:IconEdit};
// LAP-049: after Picasa's first twelve, its 3.9 effects (recipes from Picasa's filterdesc.xml, in
// t_edit_tools.rs), each with its Amount slider and live preview.
const PICASA39_EFFECTS = ['Vignette','Pixelate','Focal Zoom','Lomo-ish','Holga-ish','HDR-ish','Cinemascope','Orton-ish','Infrared','Heat Map','Cross Process'];
const effectTools = ['Sharpen','Sepia','B&W','Warmify','Film Grain','Tint','Saturation','Soft Focus','Glow','Filtered B&W','Focal B&W','Graduated Tint', ...PICASA39_EFFECTS];
// LAP-100: three pages of up to twelve, in the classic three effect-tab groups, so nothing scrolls.
const effectPages = [effectTools.slice(0, 12),
  // LAP-107: HDR-ish is retired from the pages (Automask replaces it); its renderer stays so
  // photos that already have an HDR-ish step render as before.
  ['Infrared','Lomo-ish','Holga-ish','Cinemascope','Orton-ish','Heat Map','Cross Process'],
  ['Vignette','Pixelate','Focal Zoom']];
const effectPage = ref(0);
// Amount where each 3.9 effect opens: Picasa's own default for its main slider (100 = no Fade).
const PICASA39_DEFAULT_AMOUNT: Record<string, number> = {'Vignette':70,'Pixelate':37,'Focal Zoom':50,'Lomo-ish':50,'Holga-ish':70,
  'HDR-ish':33,'Cinemascope':100,'Orton-ish':50,'Infrared':100,'Heat Map':100,'Cross Process':100};
const tuningTools = [{key:'fillLight',label:'Fill Light',min:0},{key:'highlights',label:'Highlights',min:-100},
  {key:'shadows',label:'Shadows',min:0},{key:'temperature',label:'Color Temperature',min:-100}];
const emptyTools = () => ({fillLight:0,highlights:0,shadows:0,temperature:0,neutral:null,effects:[],retouch:[],text:[],lastStep:''});
const picasaTools = ref<any>(emptyTools());
const imageSrc = ref('');
const curveDraft = ref<Curves | null>(null);
const curvePreviewSrc = ref('');
const curveNeedsPreview = computed(() => Object.values({...identityCurves(), ...(curveDraft.value || picasaTools.value.curves)} as Curves)
  .some(points => points.some(([x,y]) => x!==y)));
// While an effect's Amount panel is open, its unsaved result (LAP-087); curves draw over it.
const effectPreviewSrc = ref('');
// Mask mode (LAP-098): the effect applies only where a painted or gradient mask says.
const maskMode = ref(false);
const curveSourceSrc = computed(() => (maskMode.value ? '' : effectPreviewSrc.value) || imageSrc.value);
const displayImageSrc = computed(() => curvePreviewSrc.value || curveSourceSrc.value);
function commitCurves(curves: Curves, channel: CurveChannel) {
  picasaTools.value = {...picasaTools.value, curves, lastStep:`Curves: ${channel==='rgb' ? 'RGB' : channel[0].toUpperCase()+channel.slice(1)}`};
}
let curveBaseSource = '', curveBase: Promise<ImageData> | null = null, curvePaintId = 0;
watch([curveSourceSrc, () => JSON.stringify(curveDraft.value || picasaTools.value.curves)], async () => {
  const id=++curvePaintId, source=curveSourceSrc.value;
  const curves:Curves={...identityCurves(), ...(curveDraft.value || picasaTools.value.curves)};
  const identity=Object.values(curves).every(points=>points.every(([x,y])=>x===y));
  if (source!==curveBaseSource) {
    curveBaseSource=source;curveBase=null;curvePreviewSrc.value='';
    // Keep imageReady true so curves controls remain active and interactive
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
const textBold = ref(false);
const textItalic = ref(false);
const textOutline = ref(false);
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
// The nearest panel under the pointer that has content to scroll; never one holding the photo.
function scrollablePanelAt(target: Node) {
  for (let el = target instanceof Element ? target : target.parentElement; el && el !== document.body; el = el.parentElement) {
    if (el.scrollHeight > el.clientHeight + 1 && /auto|scroll/.test(getComputedStyle(el).overflowY)
      && !(containerRef.value && el.contains(containerRef.value))) return el;
  }
  return null;
}
function handleEditorWheel(event: WheelEvent) {
  const target=event.target as Node | null;
  const browseAnywhere=props.embedded && comparisonMode.value === 'single';
  if (!target || (!browseAnywhere && !editorRoot.value?.contains(target))) return;
  const overPhoto=containerRef.value?.contains(target);
  const overFilmstrip=target instanceof Element && target.closest('[data-editor-filmstrip]');
  // LAP-100 (owner exception): over a side panel that can scroll, the wheel scrolls that panel.
  if (!event.ctrlKey && !overPhoto && !overFilmstrip && scrollablePanelAt(target)) {
    event.stopImmediatePropagation();
    return;
  }
  // In the Library's single-photo view, ordinary wheel movement browses photos.
  // Ctrl+wheel over the photo retains explicit zoom control.
  if (props.embedded && (browseAnywhere || overPhoto || overFilmstrip) && !event.ctrlKey) {
    if (!event.deltaY) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    void navigatePhoto(event.deltaY > 0 ? 1 : -1);
    return;
  }
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

const straightenAngleInput = ref('0.0');
const isStraightenInputFocused = ref(false);
const straightenInputRef = ref<HTMLInputElement | null>(null);

const isPreviewingCrop = ref(false);
const isCropLandscape = ref(true);
const cropBoxDrawn = ref(false);
const activeSuggestionIndex = ref(-1);
const selectedAspectPresetId = ref('manual');
const customCropW = ref(4);
const customCropH = ref(3);

const cropSuggestions = [
  { label: 'Left Subject', objectPosition: '15% center' },
  { label: 'Center Portrait', objectPosition: '50% center' },
  { label: 'Wide Crop', objectPosition: '85% center' },
];

interface AspectPreset {
  id: string;
  label: string;
  w?: number;
  h?: number;
  isSquare?: boolean;
}

const picasaAspectPresets: AspectPreset[] = [
  { id: 'manual', label: 'Manual' },
  { id: 'current', label: 'Current Aspect Ratio' },
  { id: '1:1', label: 'Square (1 x 1)', isSquare: true },
  { id: '4x6', label: '10 x 15', w: 6, h: 4 },
  { id: '5x7', label: '13 x 18', w: 7, h: 5 },
  { id: '8x10', label: '20 x 25', w: 10, h: 8 },
  { id: '8.5x11', label: '8.5 x 11', w: 11, h: 8.5 },
  { id: '4:3', label: '4:3', w: 4, h: 3 },
  { id: '16:9', label: '16:9', w: 16, h: 9 },
  { id: '16:10', label: '16:10', w: 16, h: 10 },
  { id: 'custom', label: 'Custom…' },
];

function getActiveCropRatio(): number | undefined {
  if (selectedAspectPresetId.value === 'manual') {
    return undefined;
  }
  if (selectedAspectPresetId.value === 'current') {
    const effectiveW = rotate.value % 180 === 0 ? imageWidth.value : imageHeight.value;
    const effectiveH = rotate.value % 180 === 0 ? imageHeight.value : imageWidth.value;
    if (effectiveH <= 0) return 1;
    const maxDim = Math.max(effectiveW, effectiveH);
    const minDim = Math.min(effectiveW, effectiveH);
    return isCropLandscape.value ? maxDim / minDim : minDim / maxDim;
  }
  if (selectedAspectPresetId.value === '1:1') {
    return 1.0;
  }
  if (selectedAspectPresetId.value === 'custom') {
    const w = Number(customCropW.value) || 1;
    const h = Number(customCropH.value) || 1;
    const maxDim = Math.max(w, h);
    const minDim = Math.min(w, h);
    return isCropLandscape.value ? maxDim / minDim : minDim / maxDim;
  }
  const preset = picasaAspectPresets.find(p => p.id === selectedAspectPresetId.value);
  if (preset && preset.w && preset.h) {
    return isCropLandscape.value ? preset.w / preset.h : preset.h / preset.w;
  }
  return undefined;
}

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
  try {
    const doc = JSON.parse(fileInfo.value?.edits || '{}');
    if (doc.recipe && !Object.prototype.hasOwnProperty.call(doc.recipe, 'picasa') && doc.recipe.tools !== undefined) {
      doc.recipe.picasa = doc.recipe.tools;
    }
    return doc;
  } catch { return {}; }
});
const IAUTO_PRESET_LABELS: Record<number, string> = {
  1: 'Balanced',
  2: 'Natural',
  3: 'Landscape',
  4: 'Portrait',
  5: 'Vivid',
};
const activeIAutoPresetIndex = computed(() => {
  const p = Number(savedEditState.value?.auto_enhance_preset || 1);
  return p >= 1 && p <= 5 ? p : 1;
});
const activeIAutoPresetLabel = computed(() => IAUTO_PRESET_LABELS[activeIAutoPresetIndex.value] || 'Balanced');
// I'm Feeling Lucky's cycle: Original (unfiltered), then styles 1-5, then back to Original.
// The buttons show it in click order from the first style: 1 2 3 4 5 Orig (LAP-105).
const LUCKY_CYCLE = [
  ...[1, 2, 3, 4, 5].map(i => ({ index: i, short: String(i), title: `Style ${i}/5: ${IAUTO_PRESET_LABELS[i]}` })),
  { index: 0, short: 'Orig', title: 'Unfiltered (original)' },
];
const activeLuckyStep = computed(() => savedEditState.value?.auto_enhance ? activeIAutoPresetIndex.value : 0);
const luckyCycleLabel = computed(() => activeLuckyStep.value
  ? `${activeLuckyStep.value}/5 ${activeIAutoPresetLabel.value}`
  : 'Original');
const ROTATE_BUTTONS = [
  { degree: -90, label: 'Rotate left', shortcut: 'meta.rotateCounterclockwise', icon: IconRotateLeft },
  { degree: 90, label: 'Rotate right', shortcut: 'meta.rotate', icon: IconRotateRight },
];
// What a comparison pane shows: 'Unfiltered', or its I'm Feeling Lucky style and effects.
function filterPillLabel(edits: any, liveEffects?: any[]) {
  const doc = imageEditState(edits);
  const parts: string[] = [];
  if (doc.auto_enhance) {
    const p = Number(doc.auto_enhance_preset || 1);
    const preset = p >= 1 && p <= 5 ? p : 1;
    parts.push(`Lucky ${preset}/5: ${IAUTO_PRESET_LABELS[preset]}`);
  }
  const effects = liveEffects ?? doc.recipe?.picasa?.effects ?? [];
  for (const e of effects) if (e?.name) parts.push(String(e.name));
  return parts.length ? parts.join(' + ') : 'Unfiltered';
}
const selectedPaneFilterPill = computed(() => {
  const label = filterPillLabel(fileInfo.value?.edits, picasaTools.value?.effects);
  return comparisonMode.value === 'original' ? `Edited · ${label}` : label;
});
const otherPaneFilterPill = computed(() => comparisonMode.value === 'original'
  ? 'Original · Unfiltered'
  : filterPillLabel(otherComparisonFile.value?.edits));

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
  window.addEventListener('pointerdown', startRangeDrag, true);
  window.addEventListener('pointerup', endRangeDrag, true);
  window.addEventListener('pointercancel', endRangeDrag, true);
  uiStore.pushInputHandler('EditImage');
  activeEditorTab.value = config.imageEditor.tab === 'adjust' ? 'adjust' : 'edit';

  const query = router.currentRoute.value.query;
  const fileId = Number(props.initialFile?.id || query.fileId || 0);
  if (props.embedded && props.initialFile?.file_path) {
    applyFileInfo(props.initialFile);
  } else if (fileId > 0) {
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
  window.removeEventListener('pointerdown', startRangeDrag, true);
  window.removeEventListener('pointerup', endRangeDrag, true);
  window.removeEventListener('pointercancel', endRangeDrag, true);
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
  const loadingId = initEditImageLoadingId.value;
  await nextTick();
  if (editorUnmounted || loadingId !== initEditImageLoadingId.value) return;
  if (imageReady.value && curvePreviewSrc.value && imageRef.value?.currentSrc===curvePreviewSrc.value) return;
  // An effect's live preview is the same picture at the same size: nothing to set up again.
  if (imageReady.value && effectPreviewSrc.value && imageRef.value?.currentSrc===effectPreviewSrc.value) return;

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

  // Decode before showing, and drop the held picture a frame later, so the swap has no blank frame.
  try { await imageRef.value?.decode?.(); } catch { /* shown as soon as it loads */ }
  if (editorUnmounted || loadingId !== initEditImageLoadingId.value) return;
  enableTransition.value = true;
  imageReady.value = true;
  isProcessing.value = false;
  // Give the photo a paint before canvas readback and histogram calculations.
  requestAnimationFrame(() => requestAnimationFrame(() => {
    if (editorUnmounted || loadingId !== initEditImageLoadingId.value || !imageReady.value) return;
    heldImage.value = null;
    heldElement = null;
    histogramReady.value = true;
  }));
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
  picasaTools.value = { ...emptyTools(), ...JSON.parse(JSON.stringify(saved?.picasa || {})) };
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
  if (!histogramReady.value) {
    histogramReady.value = true;
    await nextTick();
  }
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
  cropBoxDrawn.value = false;
  activeSuggestionIndex.value = -1;
  crop.value = { left: 0, top: 0, width: 0, height: 0 };
  cropBox.value = { left: 0, top: 0, width: 0, height: 0 };
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
  cropBoxDrawn.value = false;
  activeSuggestionIndex.value = -1;
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
  cropBoxDrawn.value = false;
  activeSuggestionIndex.value = -1;
  crop.value = { left: 0, top: 0, width: 0, height: 0 };
  cropBox.value = { left: 0, top: 0, width: 0, height: 0 };
  cropAngle.value = 0;
  isPreviewingCrop.value = false;
  activeTool.value = '';
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
  if (activeTool.value === 'Crop' && !cropBoxDrawn.value) {
    initCropBox();
    cropBoxDrawn.value = true;
  }
  picasaTools.value.lastStep = activeTool.value === 'Straighten' ? 'Straighten' : 'Crop';
  cropApplied.value = true;
  cropBoxFixed.value = false;
  isPreviewingCrop.value = false;
  fitCropBoxToContainer();

  cropStatus.value = 0;
  activeTool.value = '';
};

const toggleCropPreview = () => {
  isPreviewingCrop.value = !isPreviewingCrop.value;
};

const resetCropBox = () => {
  cropBoxDrawn.value = false;
  activeSuggestionIndex.value = -1;
  cropBox.value = { left: 0, top: 0, width: 0, height: 0 };
  crop.value = { left: 0, top: 0, width: 0, height: 0 };
  updateCropFromCropBox();
};

const onChangeCropPreset = () => {
  const effectiveW = rotate.value % 180 === 0 ? imageWidth.value : imageHeight.value;
  const effectiveH = rotate.value % 180 === 0 ? imageHeight.value : imageWidth.value;
  if (selectedAspectPresetId.value === 'current') {
    isCropLandscape.value = effectiveW >= effectiveH;
  }
  activeSuggestionIndex.value = -1;
  initCropBox();
  cropBoxDrawn.value = true;
};

const toggleCropOrientation = () => {
  isCropLandscape.value = !isCropLandscape.value;
  if (!cropBoxDrawn.value) return;
  if (selectedAspectPresetId.value === 'manual') {
    swapCropBoxDimensions();
  } else {
    initCropBox();
  }
};

const applyCropSuggestion = (index: number) => {
  containerRect.value = containerRef.value?.getBoundingClientRect() || null;
  imageRect.value = imageRef.value?.getBoundingClientRect() || null;
  if (!imageRect.value || !containerRect.value) return;

  const imgLeft = imageRect.value.left - containerRect.value.left;
  const imgTop = imageRect.value.top - containerRect.value.top;
  const imgW = imageRect.value.width;
  const imgH = imageRect.value.height;

  let width = imgW;
  let height = imgH;
  let left = imgLeft;
  let top = imgTop;

  const aspectRatio = getActiveCropRatio();

  if (index === 0) {
    height = imgH * 0.75;
    width = aspectRatio ? height * aspectRatio : imgW * 0.55;
    if (width > imgW * 0.85) {
      width = imgW * 0.85;
      if (aspectRatio) height = width / aspectRatio;
    }
    left = imgLeft + imgW * 0.05;
    top = imgTop + (imgH - height) / 2;
  } else if (index === 1) {
    height = imgH * 0.8;
    width = aspectRatio ? height * aspectRatio : imgW * 0.45;
    if (width > imgW * 0.7) {
      width = imgW * 0.7;
      if (aspectRatio) height = width / aspectRatio;
    }
    left = imgLeft + (imgW - width) / 2;
    top = imgTop + (imgH - height) / 2;
  } else {
    width = imgW * 0.92;
    height = aspectRatio ? width / aspectRatio : imgH * 0.92;
    if (height > imgH * 0.92) {
      height = imgH * 0.92;
      if (aspectRatio) width = height * aspectRatio;
    }
    left = imgLeft + (imgW - width) / 2;
    top = imgTop + (imgH - height) / 2;
  }

  cropBox.value = { left, top, width, height };
  cropBoxDrawn.value = true;
  activeSuggestionIndex.value = index;
  updateCropFromCropBox();
};

const swapCropBoxDimensions = () => {
  if (cropBox.value.width <= 0 || cropBox.value.height <= 0) return;
  const initialRect = imageRef.value?.getBoundingClientRect() || null;
  const cRect = containerRef.value?.getBoundingClientRect() || null;
  if (!initialRect || !cRect) return;

  const cx = cropBox.value.left + cropBox.value.width / 2;
  const cy = cropBox.value.top + cropBox.value.height / 2;
  const newW = cropBox.value.height;
  const newH = cropBox.value.width;

  const imgLeft = initialRect.left - cRect.left;
  const imgTop = initialRect.top - cRect.top;
  const imgRight = imgLeft + initialRect.width;
  const imgBottom = imgTop + initialRect.height;

  let finalW = newW;
  let finalH = newH;
  if (finalW > initialRect.width) {
    const s = initialRect.width / finalW;
    finalW *= s;
    finalH *= s;
  }
  if (finalH > initialRect.height) {
    const s = initialRect.height / finalH;
    finalW *= s;
    finalH *= s;
  }

  let finalLeft = cx - finalW / 2;
  let finalTop = cy - finalH / 2;
  finalLeft = Math.max(imgLeft, Math.min(imgRight - finalW, finalLeft));
  finalTop = Math.max(imgTop, Math.min(imgBottom - finalH, finalTop));

  cropBox.value = {
    left: finalLeft,
    top: finalTop,
    width: finalW,
    height: finalH,
  };
  updateCropFromCropBox();
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

  // Selected crop aspect ratio (null when "manual" / free).
  const aspectRatio = getActiveCropRatio();

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
  const clamped = Math.min(20, Math.max(-20, Number.isFinite(v) ? v : 0));
  cropAngle.value = Math.round(clamped * 10) / 10;
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

watch(cropAngle, (newVal) => {
  if (!isStraightenInputFocused.value) {
    straightenAngleInput.value = newVal.toFixed(1);
  }
}, { immediate: true });

const commitStraightenInput = () => {
  const raw = straightenAngleInput.value.trim();
  if (!raw) {
    straightenAngleInput.value = cropAngle.value.toFixed(1);
    return;
  }
  const parsed = parseFloat(raw);
  if (isNaN(parsed)) {
    straightenAngleInput.value = cropAngle.value.toFixed(1);
    return;
  }
  const clamped = Math.min(20, Math.max(-20, parsed));
  const rounded = Math.round(clamped * 10) / 10;
  applyCropAngle(rounded);
  straightenAngleInput.value = rounded.toFixed(1);
};

const onStraightenInputFocus = () => {
  isStraightenInputFocused.value = true;
};

const onStraightenInputBlur = () => {
  isStraightenInputFocused.value = false;
  commitStraightenInput();
};

const onStraightenInputKeyDown = (event: KeyboardEvent) => {
  event.stopPropagation();
  if (event.key === 'Enter') {
    event.preventDefault();
    commitStraightenInput();
    straightenInputRef.value?.blur();
  } else if (event.key === 'Escape') {
    event.preventDefault();
    straightenAngleInput.value = cropAngle.value.toFixed(1);
    straightenInputRef.value?.blur();
  } else if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
    event.preventDefault();
    const delta = event.key === 'ArrowDown' ? -0.1 : 0.1;
    const step = event.shiftKey ? delta * 10 : delta;
    const current = parseFloat(straightenAngleInput.value);
    const base = Number.isFinite(current) ? current : cropAngle.value;
    const clamped = Math.min(20, Math.max(-20, base + step));
    const rounded = Math.round(clamped * 10) / 10;
    applyCropAngle(rounded);
    straightenAngleInput.value = rounded.toFixed(1);
  }
};

const initCropBox = () => {
  containerRect.value = containerRef.value?.getBoundingClientRect() || null;
  imageRect.value = imageRef.value?.getBoundingClientRect() || null;
  if (!imageRect.value || !containerRect.value) return;

  const insetFactor = activeTool.value === 'Crop' ? 0.8 : 1.0;
  const maxW = imageRect.value.width * insetFactor;
  const maxH = imageRect.value.height * insetFactor;
  const imageLeft = imageRect.value.left - containerRect.value.left;
  const imageTop = imageRect.value.top - containerRect.value.top;

  const aspectRatio = getActiveCropRatio();
  if (aspectRatio != null && aspectRatio > 0) {
    let newWidth;
    let newHeight;
    if (maxW / maxH > aspectRatio) {
      newHeight = maxH;
      newWidth = newHeight * aspectRatio;
    } else {
      newWidth = maxW;
      newHeight = newWidth / aspectRatio;
    }

    cropBox.value = {
      left: imageLeft + (imageRect.value.width - newWidth) / 2,
      top: imageTop + (imageRect.value.height - newHeight) / 2,
      width: newWidth,
      height: newHeight,
    };
  } else {
    cropBox.value = {
      left: imageLeft + (imageRect.value.width - maxW) / 2,
      top: imageTop + (imageRect.value.height - maxH) / 2,
      width: maxW,
      height: maxH,
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
  picasaTools.value.lastStep = 'Rotate';
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

const startNewCropDrag = (event: MouseEvent) => {
  containerRect.value = containerRef.value?.getBoundingClientRect() || null;
  imageRect.value = imageRef.value?.getBoundingClientRect() || null;
  if (!imageRect.value || !containerRect.value) return;

  isDragging.value = true;
  cropBoxDrawn.value = true;
  activeSuggestionIndex.value = -1;
  dragHandle.value = 'bottom-right';
  document.body.style.cursor = 'crosshair';
  document.body.style.userSelect = 'none';

  const startX = Math.max(imageRect.value.left, Math.min(imageRect.value.right, event.clientX));
  const startY = Math.max(imageRect.value.top, Math.min(imageRect.value.bottom, event.clientY));

  const startContainerX = startX - containerRect.value.left;
  const startContainerY = startY - containerRect.value.top;

  cropBox.value = {
    left: startContainerX,
    top: startContainerY,
    width: 1,
    height: 1,
  };

  const initialBox = { ...cropBox.value };
  const initialRect = imageRect.value;
  const cRect = containerRect.value;

  const geo = {
    cx: initialRect.left - cRect.left + initialRect.width / 2,
    cy: initialRect.top - cRect.top + initialRect.height / 2,
    cos: Math.cos((displayRotate.value * Math.PI) / 180),
    sin: Math.sin((displayRotate.value * Math.PI) / 180),
    hw: (imageWidth.value * scale.value) / 2,
    hh: (imageHeight.value * scale.value) / 2,
  };

  const doDrag = (e: MouseEvent) => {
    if (!isDragging.value || !containerRect.value || !imageRect.value) return;
    const curX = Math.max(initialRect.left, Math.min(initialRect.right, e.clientX)) - containerRect.value.left;
    const curY = Math.max(initialRect.top, Math.min(initialRect.bottom, e.clientY)) - containerRect.value.top;

    const imgLeft = initialRect.left - cRect.left;
    const imgTop = initialRect.top - cRect.top;
    const imgRight = imgLeft + initialRect.width;
    const imgBottom = imgTop + initialRect.height;

    const availW = curX >= startContainerX ? imgRight - startContainerX : startContainerX - imgLeft;
    const availH = curY >= startContainerY ? imgBottom - startContainerY : startContainerY - imgTop;

    let w = Math.min(availW, Math.max(10, Math.abs(curX - startContainerX)));
    let h = Math.min(availH, Math.max(10, Math.abs(curY - startContainerY)));

    const aspectRatio = getActiveCropRatio();
    if (aspectRatio != null && aspectRatio > 0) {
      // Choose the largest box with ratio `aspectRatio` driven by the user's drag
      // that also fits within `(availW, availH)` from the anchor point.
      let targetW = w;
      let targetH = targetW / aspectRatio;
      if (targetH < h) {
        targetH = h;
        targetW = targetH * aspectRatio;
      }
      if (targetW > availW) {
        targetW = availW;
        targetH = targetW / aspectRatio;
      }
      if (targetH > availH) {
        targetH = availH;
        targetW = targetH * aspectRatio;
      }
      w = Math.max(10, targetW);
      h = Math.max(10, targetH);
    }

    const left = curX < startContainerX ? Math.max(imgLeft, startContainerX - w) : startContainerX;
    const top = curY < startContainerY ? Math.max(imgTop, startContainerY - h) : startContainerY;

    const proposed = { left, top, width: w, height: h };
    const t = maxResizeT(
      initialBox, proposed,
      geo.cx, geo.cy, geo.cos, geo.sin, geo.hw, geo.hh, 10,
    );
    cropBox.value = {
      left: initialBox.left + t * (proposed.left - initialBox.left),
      top: initialBox.top + t * (proposed.top - initialBox.top),
      width: Math.max(10, initialBox.width + t * (proposed.width - initialBox.width)),
      height: Math.max(10, initialBox.height + t * (proposed.height - initialBox.height)),
    };

    updateCropFromCropBox({ imageRect: initialRect, containerRect: cRect });
  };

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
    if (lastEvent) {
      doDrag(lastEvent);
      lastEvent = null;
    }
    document.body.style.cursor = '';
    document.body.style.userSelect = '';
    isDragging.value = false;
    window.removeEventListener('mousemove', onMouseMove);
    window.removeEventListener('mouseup', stopDrag);

    if (cropBox.value.width < 15 || cropBox.value.height < 15) {
      cropBoxDrawn.value = false;
      cropBox.value = { left: 0, top: 0, width: 0, height: 0 };
      crop.value = { left: 0, top: 0, width: 0, height: 0 };
      updateCropFromCropBox();
    } else {
      cropBoxDrawn.value = true;
    }
  };

  window.addEventListener('mousemove', onMouseMove);
  window.addEventListener('mouseup', stopDrag);
};

const onContainerMouseDown = (event: MouseEvent) => {
  if (activeTool.value !== 'Crop' || cropStatus.value !== 1 || isPreviewingCrop.value) return;
  if (event.button !== 0) return;
  const target = event.target as HTMLElement;
  if (target && target.closest('.crop-box-active')) return;
  event.preventDefault();
  startNewCropDrag(event);
};

const startDrag = (handle: string, event: MouseEvent) => {
  if (handle === 'move') {
    const target = event.currentTarget as HTMLElement;
    if (target) {
      const rect = target.getBoundingClientRect();
      const clickInside =
        event.clientX >= rect.left &&
        event.clientX <= rect.right &&
        event.clientY >= rect.top &&
        event.clientY <= rect.bottom;
      if (!clickInside && activeTool.value === 'Crop') {
        startNewCropDrag(event);
        return;
      }
    }
  }

  isDragging.value = true;
  activeSuggestionIndex.value = -1;
  dragHandle.value = handle;
  dragStartX.value = event.clientX;
  dragStartY.value = event.clientY;

  let cursor = 'move';
  if (handle === 'top-left' || handle === 'bottom-right') cursor = 'nwse-resize';
  else if (handle === 'top-right' || handle === 'bottom-left') cursor = 'nesw-resize';
  else if (handle === 'top' || handle === 'bottom') cursor = 'ns-resize';
  else if (handle === 'left' || handle === 'right') cursor = 'ew-resize';
  document.body.style.cursor = cursor;
  document.body.style.userSelect = 'none';

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

      const aspectRatio = getActiveCropRatio();
      if (aspectRatio != null && aspectRatio > 0) {
        if (dragHandle.value === 'top' || dragHandle.value === 'bottom') {
          proposedBox.width = proposedBox.height * aspectRatio;
          proposedBox.left = initialCropBoxData.left + (initialCropBoxData.width - proposedBox.width) / 2;
        } else if (dragHandle.value === 'left' || dragHandle.value === 'right') {
          proposedBox.height = proposedBox.width / aspectRatio;
          proposedBox.top = initialCropBoxData.top + (initialCropBoxData.height - proposedBox.height) / 2;
        } else {
          // Corner handles:
          const ratioW = Math.abs(proposedBox.width - initialCropBoxData.width);
          const ratioH = Math.abs(proposedBox.height - initialCropBoxData.height);
          if (ratioW >= ratioH) {
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
    document.body.style.cursor = '';
    document.body.style.userSelect = '';
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
  if (exportDialog.value?.open) return;
  if (!uiStore.isInputActive('EditImage')) return;
  const historyAction = editHistoryAction(event);
  if (historyAction) {
    event.preventDefault();
    event.stopPropagation();
    queueHistory(historyAction);
    return;
  }
  if (handleAutomaskKey(event) || handleMaskKey(event)) {
    event.preventDefault();
    event.stopPropagation();
    return;
  }
  const inField = isEditableTarget(event.target) || (event.target instanceof Element && !!event.target.closest('[role="slider"], .angle-slider'));
  // Esc in a field or slider, or with a tool open: leave the field and cancel the tool, history untouched.
  // Only an Esc with nothing open leaves the editor.
  if (event.key === 'Escape' && !showDiffPreview.value && (inField || cropStatus.value === 1 || activeTool.value)) {
    event.preventDefault();
    event.stopPropagation();
    if (inField && document.activeElement instanceof HTMLElement) document.activeElement.blur();
    if (cropStatus.value === 1 || activeTool.value) cancelActiveTool();
    return;
  }
  if (inField) return;
  if (cropStatus.value === 1 && (event.key === 'ArrowLeft' || event.key === 'ArrowRight' || event.key === 'ArrowUp' || event.key === 'ArrowDown')) {
    if (activeTool.value === 'Straighten') {
      event.preventDefault();
      event.stopPropagation();
      const delta = (event.key === 'ArrowLeft' || event.key === 'ArrowDown') ? -0.1 : 0.1;
      const step = event.shiftKey ? delta * 10 : delta;
      applyCropAngle(Number(Math.min(20, Math.max(-20, cropAngle.value + step)).toFixed(1)));
      return;
    }
  }
  if (props.embedded && cropStatus.value !== 1 && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) {
    event.preventDefault();
    event.stopImmediatePropagation();
    navigatePhoto(event.key === 'ArrowLeft' ? -1 : 1);
    return;
  }
  if (handleEditorShortcut(event)) {
    event.preventDefault();
    event.stopPropagation();
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

// Cancel the open tool or effect without a history step. A crop or straighten draft is dropped and
// the controls come back from the saved recipe, as its Cancel button does.
function cancelActiveTool() {
  maskMode.value = false;
  if (cropStatus.value === 1) {
    clickCancelCrop();
    void restoreEditorControls();
  }
  activeTool.value = '';
}

// Ctrl+1 / 2 / 3 switch the edit tabs, R / Shift+R rotate (LAP-101). Like the tab and rotate
// buttons, they wait while a crop, tool or mask is open.
const EDITOR_TAB_SHORTCUTS = [
  { actionId: 'editor.tabBasic', tab: 'basic' },
  { actionId: 'editor.tabTuning', tab: 'tuning' },
  { actionId: 'editor.tabEffects', tab: 'effects' },
] as const;
const editorTabShortcut = (tab: string) => getShortcutLabel(EDITOR_TAB_SHORTCUTS.find(item => item.tab === tab)!.actionId);
function handleEditorShortcut(event: KeyboardEvent): boolean {
  const tab = EDITOR_TAB_SHORTCUTS.find(item => matchesShortcut(item.actionId, event))?.tab;
  const degree = matchesShortcut('meta.rotate', event) ? 90 : matchesShortcut('meta.rotateCounterclockwise', event) ? -90 : 0;
  if (!tab && !degree) return false;
  if (cropStatus.value === 1 || isProcessing.value || activeTool.value || maskMode.value) return true;
  if (tab) editorTab.value = tab;
  else if (!cropApplied.value && imageReady.value) clickRotate(degree);
  return true;
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
    picasa: JSON.parse(JSON.stringify(picasaTools.value)),
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

// Revert to Original (LAP-RE-03): back to the photo as it was - its .picasaoriginals copy when a
// save wrote over the file, and no edits. Enabled when there is such a copy or there are edits.
const hasOriginalCopy = ref(false);
const canRevertOriginal = computed(() => hasOriginalCopy.value || !!savedEditState.value.recipe || !!savedEditState.value.auto_enhance);
async function refreshOriginalCopy() {
  const file = fileInfo.value;
  if (!file?.id || !file.file_path) { hasOriginalCopy.value = false; return; }
  try { hasOriginalCopy.value = await invoke<boolean>('has_picasa_original', { filePath: file.file_path, fileId: null }); }
  catch { hasOriginalCopy.value = false; }
}
watch(() => [fileInfo.value?.id, fileInfo.value?.file_path, fileInfo.value?.modified_at], () => { void refreshOriginalCopy(); }, { immediate: true });

async function revertToOriginal() {
  if (isProcessing.value || cropStatus.value === 1 || !fileInfo.value?.id) return;
  const fileId = Number(fileInfo.value.id);
  const filePath = fileInfo.value.file_path;
  if (hasOriginalCopy.value && !await askDialog('Replace this photo with its original from the .picasaoriginals folder? Changes saved into the file itself are lost; LapCasa\'s edits are cleared and can still be undone.',
    { title: 'Revert to Original', kind: 'warning' })) return;
  isProcessing.value = true;
  try {
    await historyNavigation;
    await flushRecipeSteps();
    const changed = await invoke<boolean>('revert_to_picasa_original', { filePath, fileId });
    const file: any = await getFileInfo(fileId);
    if (file && Number(fileInfo.value?.id) === fileId) fileInfo.value = { ...fileInfo.value, ...file };
    uiStore.clearActiveAdjustments();
    if (changed) {
      uiStore.updateFileVersion(filePath);
      sendToParent({ type: 'recipe-step', recipeSaved: true, sourceFileId: fileId, saveAsNew: false, filePath });
    }
  } catch (error) {
    historyError.value = String(error);
  } finally { isProcessing.value = false; }
  await restoreEditorControls();
  await refreshOriginalCopy();
}

function historyStepName(step:{step?:string}) {
  return step.step === 'Undo All Edits' ? 'All Edits' : step.step || 'Edit';
}

function toolPreview(name:string) {
  return ({'Sepia':'sepia(1)','B&W':'grayscale(1)','Warmify':'sepia(.3) saturate(1.3)',
    'Tint':'sepia(.5) hue-rotate(150deg)','Saturation':'saturate(2)','Soft Focus':'blur(1px)',
    'Glow':'brightness(1.2)','Filtered B&W':'grayscale(1) contrast(1.2)','Focal B&W':'grayscale(.6)',
    'Graduated Tint':'sepia(.4)','Sharpen':'contrast(1.2)','Vignette':'brightness(.85)','Pixelate':'blur(.5px) contrast(1.1)',
    'Focal Zoom':'blur(.7px)','Lomo-ish':'saturate(1.2) contrast(1.35)','Holga-ish':'grayscale(1) contrast(1.25)',
    'HDR-ish':'contrast(1.3) saturate(1.2)','Cinemascope':'saturate(.75) contrast(1.1)','Orton-ish':'contrast(1.3) brightness(1.05)',
    'Infrared':'grayscale(1) brightness(1.3)','Heat Map':'grayscale(1) sepia(1) hue-rotate(180deg) saturate(4)',
    'Cross Process':'contrast(1.15) saturate(1.35) hue-rotate(-12deg)'} as Record<string,string>)[name] || '';
}

async function selectPicasaTool(tool:string) {
  if (tool==='Crop' || tool==='Straighten') {
    activeTool.value = tool;
    isPreviewingCrop.value = false;
    // Every crop starts free (Manual); a ratio picked last time is not carried over (LAP-102).
    selectedAspectPresetId.value = 'manual';
    if (tool === 'Crop') {
      const effectiveW = rotate.value % 180 === 0 ? imageWidth.value : imageHeight.value;
      const effectiveH = rotate.value % 180 === 0 ? imageHeight.value : imageWidth.value;
      isCropLandscape.value = effectiveW >= effectiveH;
      const hadAppliedCrop = cropApplied.value && crop.value.width > 0 && crop.value.height > 0;
      cropStatus.value = 1;
      cropApplied.value = false;
      cropBoxFixed.value = false;
      activeSuggestionIndex.value = -1;
      if (hadAppliedCrop) {
        fitImageToContainer();
        cropBoxDrawn.value = true;
      } else {
        cropBoxDrawn.value = false;
        cropAngle.value = 0;
        cropBox.value = { left: 0, top: 0, width: 0, height: 0 };
        crop.value = { left: 0, top: 0, width: 0, height: 0 };
      }
    } else if (tool === 'Straighten') {
      cropStatus.value = 1;
      cropBoxDrawn.value = true;
      initCropBox();
    }
  } else if (tool==="I'm Feeling Lucky") {
    await flushRecipeSteps();
    fileInfo.value.edits = await invoke<string>('toggle_auto_enhance',{fileId:Number(fileInfo.value.id),enable:true});
    refreshBasePreview();
    sendToParent({type:'recipe-step',recipeSaved:true,sourceFileId:Number(fileInfo.value.id),filePath:fileInfo.value.file_path});
  } else if (tool==='Redeye') {
    redEye.value=true;picasaTools.value.lastStep='Redeye';
  } else if (tool==='Auto Contrast' || tool==='Auto Color') {
    await flushRecipeSteps();
    const result = await invoke<{failed:any[]}>('batch_edit_images', {fileIds:[Number(fileInfo.value.id)],
      op:tool==='Auto Contrast' ? 'auto_contrast' : 'auto_color'});
    if (result.failed.length) { historyError.value=result.failed[0].error; return; }
    const updated = await getFileInfo(Number(fileInfo.value.id));
    fileInfo.value.edits = updated.edits;
    await restoreEditorControls();
    sendToParent({type:'recipe-step',recipeSaved:true,sourceFileId:Number(fileInfo.value.id),filePath:fileInfo.value.file_path});
  } else if (['Sepia','B&W','Warmify','Film Grain'].includes(tool)) {
    picasaTools.value.effects.push({name:tool,amount:0.5});picasaTools.value.lastStep=tool;
  } else {
    activeTool.value=tool;effectAmount.value=tool==='Saturation'?75:(PICASA39_DEFAULT_AMOUNT[tool] ?? 50);
    retouchTarget.value=null;retouchSource.value=null;
  }
}

// LAP-100: Orig or style 1-5 straight from its button, as one undoable step.
async function selectLuckyStep(step:number) {
  if (step===activeLuckyStep.value) return;
  await flushRecipeSteps();
  fileInfo.value.edits = await invoke<string>('set_lucky_style',{fileId:Number(fileInfo.value.id),style:step});
  refreshBasePreview();
  sendToParent({type:'recipe-step',recipeSaved:true,sourceFileId:Number(fileInfo.value.id),filePath:fileInfo.value.file_path});
}

function applyPicasaEffect() {
  effectApplied = true;   // keep the live result up until the saved step's preview replaces it
  picasaTools.value.effects.push({name:activeTool.value,amount:effectAmount.value/100,color:effectColor.value,cx:toolPoint.value.x,cy:toolPoint.value.y});
  picasaTools.value.lastStep=activeTool.value;activeTool.value='';
}

// Masked effects (LAP-098). An effect listed here offers Mask (M) in its panel; add a name to give
// another effect the same painted / gradient mask. The renderer applies any step's mask.
const MASK_EFFECTS = ['Automask'];   // LAP-107: the masks belong to Automask
// LAP-103: the photo offers its regions. Subject / Sky / People / Background and Click to select
// come from the content selection model (select_* commands); Brush and Gradient are LAP-098's.
const MASK_REGIONS = [
  { key: 'subject', label: 'Subject', title: 'The main subject: the person (from their face) or what is in the middle' },
  { key: 'sky', label: 'Sky', title: 'The sky' },
  { key: 'people', label: 'People', title: 'Everyone whose face the library found' },
  { key: 'background', label: 'Background', title: 'Everything but the subject' },
] as const;
type MaskRegion = '' | typeof MASK_REGIONS[number]['key'];
const maskTool = ref<'paint' | 'gradient' | 'select'>('select');
const maskRegion = ref<MaskRegion>('');
const selectPoints = ref<SelectPoint[]>([]);
const selectStatus = ref<{ available: boolean; reason: string; cuda?: boolean } | null>(null);
const selectBusy = ref(false);
const selectPreparing = ref(false);
let preparedFileId = 0;
let preparingFileId = 0;
let preparePromise: Promise<void> | null = null;
let unlistenGpu: (() => void) | null = null;
onMounted(async () => {
  const off = await listen('gpu-ready', () => {
    preparedFileId = 0;
    selectStatus.value = null;
    void prepareSelect().catch(() => {});
  });
  if (editorUnmounted) off(); else unlistenGpu = off;
});
onUnmounted(() => { unlistenGpu?.(); });
const selectError = ref('');
const selectDisabledReason = computed(() => selectStatus.value && !selectStatus.value.available ? (selectStatus.value.reason || 'Content selection is not available') : '');
let selectRequest = 0;
const maskBrush = ref(0.05);      // brush radius, fraction of the photo's short side
const maskFeather = ref(0.01);    // edge softening (Gaussian sigma), fraction of the short side
const maskInverted = ref(false);
const maskOverlay = ref(false);
const effectMaskRef = ref<InstanceType<typeof EffectMask> | null>(null);
const maskLayerStyle = computed((): CSSProperties => ({ ...imageStyle.value, transition: 'none', willChange: 'auto' }));

function enterMaskMode(tool: 'paint' | 'gradient' | 'select' = 'select') {
  if (!MASK_EFFECTS.includes(activeTool.value) || cropStatus.value === 1 || !imageReady.value) return;
  maskTool.value = tool; maskInverted.value = false; maskOverlay.value = false;
  maskRegion.value = ''; selectPoints.value = []; selectError.value = '';
  maskMode.value = true;
  void prepareSelect().catch(() => {});
}
function leaveMaskMode() { maskMode.value = false; }
// Brush, Gradient or Click to select as the current tool; opens mask mode if it is not open.
function startMaskTool(tool: 'paint' | 'gradient' | 'select') {
  if (!maskMode.value) { enterMaskMode(tool); return; }
  if (tool === 'select' && selectDisabledReason.value) return;
  maskTool.value = tool;
  if (tool === 'select') { maskRegion.value = ''; }
  if (tool === 'gradient') { maskRegion.value = ''; selectPoints.value = []; }
  selectError.value = '';
}
function toggleMaskGradient() { startMaskTool(maskTool.value === 'gradient' ? 'select' : 'gradient'); }
watch(activeTool, tool => {
  if (!MASK_EFFECTS.includes(tool)) maskMode.value = false;
  else if (!selectStatus.value) void invoke<{ available: boolean; reason: string; cuda?: boolean }>('select_status').then(s => { selectStatus.value = s; }).catch(e => { selectStatus.value = { available: false, reason: String(e) }; });
});
watch(() => fileInfo.value?.id, () => { maskMode.value = false; preparedFileId = 0; ++selectRequest; selectBusy.value = false; });
watch([imageReady, () => fileInfo.value?.id], ([ready, id]) => {
  if (ready && id) void prepareSelect().catch(() => {});
}, { immediate: true });

// Start as soon as the editor photo is ready; clicks await the same in-flight preparation.
async function prepareSelect() {
  const fileId = Number(fileInfo.value?.id);
  if (!fileId || selectDisabledReason.value) return;
  if (preparedFileId === fileId) return;
  if (preparePromise && preparingFileId === fileId) return preparePromise;
  preparingFileId = fileId;
  selectPreparing.value = true;
  const promise = invoke('select_prepare', { fileId }).then(() => {
    if (Number(fileInfo.value?.id) === fileId) preparedFileId = fileId;
  }).catch(e => {
    if (Number(fileInfo.value?.id) === fileId) selectError.value = String(e);
    throw e;
  }).finally(() => {
    if (preparePromise === promise) { preparePromise = null; selectPreparing.value = false; }
  });
  preparePromise = promise;
  return promise;
}
// A click in Click to select mode: a new selection, or Shift / Alt to add to or take from it.
function onSelectClick(p: { x: number; y: number; add: boolean; subtract: boolean }) {
  if (maskTool.value !== 'select' || isProcessing.value) return;
  const point: SelectPoint = { x: p.x, y: p.y, label: p.subtract && selectPoints.value.length ? 0 : 1 };
  selectPoints.value = (p.add || p.subtract) && selectPoints.value.length ? [...selectPoints.value, point] : [point];
  maskRegion.value = '';
  void runSelection('select_points', { points: selectPoints.value });
}
// Subject / Sky / People / Background: one click, the model's region; Background is Subject inverted.
function applyRegion(kind: MaskRegion) {
  if (!kind || selectDisabledReason.value) return;
  if (!maskMode.value) enterMaskMode('select');
  maskTool.value = 'select'; maskRegion.value = kind; selectPoints.value = [];
  maskInverted.value = kind === 'background';
  void runSelection('select_region', { kind: kind === 'background' ? 'subject' : kind });
}
async function runSelection(command: 'select_points' | 'select_region', args: Record<string, unknown>) {
  const fileId = Number(fileInfo.value?.id);
  if (!fileId) return;
  const request = ++selectRequest;
  selectBusy.value = true; selectError.value = '';
  try {
    await prepareSelect();
    if (request !== selectRequest || Number(fileInfo.value?.id) !== fileId) return;
    const r = await invoke<{ png: string; iou: number; ms: number }>(command, { fileId, ...args });
    if (request !== selectRequest || Number(fileInfo.value?.id) !== fileId || !maskMode.value) return;
    await effectMaskRef.value?.setSelection(r.png);
  } catch (e) {
    if (request === selectRequest) selectError.value = String(e);
  } finally {
    if (request === selectRequest) selectBusy.value = false;
  }
}

// Where a pointer is on the photo, 0..1 of its width and height (as pickEditorPoint, unclamped).
function clientToImage(clientX: number, clientY: number) {
  const rect = imageRef.value!.getBoundingClientRect();
  const dx = (clientX - rect.left - rect.width / 2) / scale.value;
  const dy = (clientY - rect.top - rect.height / 2) / scale.value;
  const a = displayRotate.value * Math.PI / 180;
  const x = (dx * Math.cos(a) + dy * Math.sin(a)) * (isFlippedX.value ? -1 : 1) + imageWidth.value / 2;
  const y = (-dx * Math.sin(a) + dy * Math.cos(a)) * (isFlippedY.value ? -1 : 1) + imageHeight.value / 2;
  return { x: x / imageWidth.value, y: y / imageHeight.value };
}

// M opens mask mode for an effect that has it; in mask mode the keys below drive the mask and
// nothing else (no photo navigation). Returns true when the key was used.
function handleMaskKey(event: KeyboardEvent): boolean {
  if (event.ctrlKey || event.metaKey || isProcessing.value) return false;
  const target = event.target;
  const slider = target instanceof HTMLInputElement && target.type === 'range';
  if (isEditableTarget(target) && !slider) return false;
  const key = event.key.toLowerCase();
  if (!maskMode.value) {
    if (!event.altKey && MASK_EFFECTS.includes(activeTool.value)) {
      if (key === 'm' || key === 'b') { enterMaskMode('paint'); return maskMode.value; }
      if (key === 's' && !selectDisabledReason.value) { enterMaskMode('select'); return maskMode.value; }
      if (key === 'g') { enterMaskMode('gradient'); return maskMode.value; }
    }
    return false;
  }
  if (event.code === 'BracketLeft' || event.code === 'BracketRight') {
    const up = event.code === 'BracketRight';
    if (event.shiftKey) maskFeather.value = Math.min(0.1, up ? Math.max(0.002, maskFeather.value * 1.25) : (maskFeather.value < 0.0025 ? 0 : maskFeather.value / 1.25));
    else maskBrush.value = Math.min(0.5, Math.max(0.005, up ? maskBrush.value * 1.2 : maskBrush.value / 1.2));
    return true;
  }
  switch (key) {
    case 'i': maskInverted.value = !maskInverted.value; return true;
    case 'x': maskOverlay.value = !maskOverlay.value; return true;
    case 'g': toggleMaskGradient(); return true;
    case 'b': startMaskTool('paint'); return true;
    case 's': startMaskTool('select'); return true;
    case 'm': leaveMaskMode(); return true;
    case 'backspace': case 'delete': maskRegion.value = ''; selectPoints.value = []; effectMaskRef.value?.clear(); return true;
    case 'enter': void (activeTool.value === 'Automask' ? commitLight() : commitMask()); return true;
    // Esc: no step, the photo back as it was.
    case 'escape': maskMode.value = false; activeTool.value = ''; return true;
    case 'arrowleft': case 'arrowright': case 'arrowup': case 'arrowdown': return !slider;
    case ' ': return true;
    default: return false;
  }
}

async function commitMask() {
  const layer = effectMaskRef.value;
  if (!maskMode.value || !layer || isProcessing.value || !fileInfo.value?.id) return;
  if (selectBusy.value) return;   // the selection is still being made: Enter then would commit the wrong mask
  // Nothing painted: the effect applies to the whole photo, exactly as without a mask.
  if (!maskInverted.value && layer.isEmpty()) { maskMode.value = false; applyPicasaEffect(); return; }
  const fileId = Number(fileInfo.value.id);
  const tool = activeTool.value;
  isProcessing.value = true;
  try {
    const mask = await invoke<string>('save_effect_mask', { fileId, png: layer.exportPng(), feather: maskFeather.value,
      inverted: maskInverted.value, info: layer.info() });
    const step = { name: tool, amount: effectAmount.value / 100, color: effectColor.value, cx: toolPoint.value.x, cy: toolPoint.value.y, mask };
    const tools = JSON.parse(JSON.stringify(picasaTools.value));
    tools.effects = [...(tools.effects || []), step];
    ++effectDraftId;
    const preview = await invoke<string>('render_edit_draft', { fileId, picasa: tools });
    if (Number(fileInfo.value?.id) !== fileId || activeTool.value !== tool) return;
    // The masked result stays up until the saved step's preview replaces it.
    effectPreviewSrc.value = preview;
    effectApplied = true;
    maskMode.value = false;
    picasaTools.value.effects.push(step);
    picasaTools.value.lastStep = tool; activeTool.value = '';
  } catch (error) {
    historyError.value = String(error);
  } finally { isProcessing.value = false; }
}

// LAP-107: Automask - brightness by region (local curves). Each entry is a region (whole photo or a
// saved mask) and a tone curve: Shadows / Highlights / Whites / Blacks bend it both ways, points
// drawn in its curve apply on top. Kept in the recipe's local list as settings (t_edit_local.rs
// renders them), so each can be reopened, switched off or removed later; they stack in order.
type LightAdj = { region: string; mask: string | null; enabled?: boolean; detail?: boolean; shadows: number; highlights: number; whites: number; blacks: number; curve: CurvePoint[] };
type LightSlider = 'shadows' | 'highlights' | 'whites' | 'blacks';
const LIGHT_SLIDERS: { key: LightSlider; label: string; title: string }[] = [
  { key: 'shadows', label: 'Shadows', title: 'Right lifts the dark tones, left deepens them' },
  { key: 'highlights', label: 'Highlights', title: 'Left holds back the bright tones, right brightens them' },
  { key: 'whites', label: 'Whites', title: 'Moves the white end' },
  { key: 'blacks', label: 'Blacks', title: 'Moves the black end' },
];
const LIGHT_REGIONS: Record<string, string> = { whole: 'Whole photo', subject: 'Subject', sky: 'Sky', people: 'People', background: 'Background',
  select: 'Selection', brush: 'Brushed area', gradient: 'Gradient', dark: 'Dark areas', bright: 'Bright areas' };
// Number keys pick where it applies, in the row's own order.
const AUTOMASK_KEYS = [
  { actionId: 'editor.automaskWhole', region: 'whole' },
  { actionId: 'editor.automaskSubject', region: 'subject' },
  { actionId: 'editor.automaskSky', region: 'sky' },
  { actionId: 'editor.automaskPeople', region: 'people' },
  { actionId: 'editor.automaskBackground', region: 'background' },
] as const;
const regionKey = (region: string) => { const k = AUTOMASK_KEYS.find(item => item.region === region); return k ? getShortcutLabel(k.actionId) : ''; };
const newLight = (): LightAdj => ({ region: 'whole', mask: null, enabled: true, detail: true, shadows: 0, highlights: 0, whites: 0, blacks: 0, curve: [[0, 0], [1, 1]] });
const lightDraft = ref<LightAdj>(newLight());
const lightEditing = ref(-1);
const lightCurvePreview = ref<CurvePoint[] | null>(null);
const lightStatus = ref('');
const balancing = ref(false);
const lightList = computed<LightAdj[]>(() => Array.isArray(picasaTools.value.local) ? picasaTools.value.local : []);
const lightKeepsMask = computed(() => activeTool.value === 'Automask' && !maskMode.value && !!lightDraft.value.mask);
const lightCurveModel = computed(() => ({ ...identityCurves(), rgb: lightDraft.value.curve }));
const lightCurveBent = computed(() => lightDraft.value.curve.some(([x, y]) => x !== y));
const clonePlain = <T,>(v: T): T => JSON.parse(JSON.stringify(v));
function lightLabel(a: LightAdj) { return LIGHT_REGIONS[a.mask ? a.region : 'whole'] || 'Area'; }
function lightSummary(a: LightAdj) {
  const parts = LIGHT_SLIDERS.filter(sl => a[sl.key]).map(sl => `${sl.label} ${a[sl.key] > 0 ? '+' : ''}${a[sl.key]}`);
  if (a.curve?.some(([x, y]) => x !== y)) parts.push('curve');
  return parts.join(', ') || 'no change';
}
// The curve that applies (sliders' bend, then the points), drawn in blue under the editable points.
const lightResult = computed(() => {
  const lut = curveLut(lightCurvePreview.value || lightDraft.value.curve);
  const a = lightDraft.value;
  let last = 0;
  return Array.from({ length: 129 }, (_, i) => {
    // Detail mode: a negative Blacks is a black point (t_edit_local BLACK_POINT_MAX 0.25), not a bend.
    const bp = a.detail && a.blacks < 0 ? -a.blacks / 100 * 0.25 : 0;
    const bent = automaskBend(i / 128, a.shadows / 100, a.highlights / 100, a.whites / 100, bp ? 0 : a.blacks / 100);
    const y = Math.max(last, Math.max(0, Math.min(1, bp ? (bent - bp) / (1 - bp) : bent)));
    last = y;
    const f = y * 255, i0 = Math.floor(f), i1 = Math.min(255, i0 + 1);
    return (lut[i0] * (1 - (f - i0)) + lut[i1] * (f - i0)) / 255;
  });
});
function onLightCurve(curves: Curves) { lightDraft.value = { ...lightDraft.value, curve: curves.rgb }; lightCurvePreview.value = null; }
// Open Automask: a new one starts by picking where it applies (the photo answers to the pointer at
// once); an existing one opens with its settings and keeps its area unless another is picked.
function openLight(index: number) {
  lightEditing.value = index >= 0 && index < lightList.value.length ? index : -1;
  lightDraft.value = lightEditing.value >= 0 ? { ...newLight(), ...clonePlain(lightList.value[index]) } : newLight();
  lightCurvePreview.value = null;
  lightStatus.value = '';
  maskMode.value = false;
  activeTool.value = 'Automask';
  if (lightEditing.value < 0) void nextTick(() => enterMaskMode('select'));
}
function chooseWholePhoto() {
  leaveMaskMode();
  if (activeTool.value === 'Automask') lightDraft.value = { ...lightDraft.value, region: 'whole', mask: null };
}
function setLightList(local: LightAdj[], step: string) {
  effectApplied = false;
  picasaTools.value = { ...picasaTools.value, local, lastStep: step };
}
function toggleLight(index: number) {
  const local = clonePlain(lightList.value);
  const on = local[index].enabled === false;
  local[index].enabled = on;
  setLightList(local, `Automask ${index + 1} ${on ? 'on' : 'off'}`);
}
function removeLight(index: number) {
  const local = clonePlain(lightList.value);
  const [gone] = local.splice(index, 1);
  setLightList(local, `Remove Automask: ${lightLabel(gone)}`);
}
// The list as it would be with the draft in place.
function lightListWithDraft(entry: LightAdj) {
  const local = clonePlain(lightList.value);
  if (lightEditing.value >= 0 && lightEditing.value < local.length) local[lightEditing.value] = entry; else local.push(entry);
  return local;
}
// Live result while the sliders and curve move (an unsaved render, no history step). In mask mode the
// draft is rendered for the whole photo and the mask layer shows it only where the selection is.
let lightDraftTimer: ReturnType<typeof setTimeout> | null = null, lightDraftId = 0;
watch([activeTool, () => JSON.stringify(lightDraft.value), () => JSON.stringify(lightCurvePreview.value), maskMode], () => {
  if (activeTool.value !== 'Automask' || !fileInfo.value?.id) return;
  if (lightDraftTimer) clearTimeout(lightDraftTimer);
  const id = ++lightDraftId;
  lightDraftTimer = setTimeout(async () => {
    const fileId = Number(fileInfo.value.id);
    const entry: LightAdj = { ...clonePlain(lightDraft.value), curve: clonePlain(lightCurvePreview.value || lightDraft.value.curve), enabled: true };
    if (maskMode.value) { entry.mask = null; entry.region = 'whole'; }
    const tools = clonePlain(picasaTools.value);
    tools.local = lightListWithDraft(entry);
    try {
      const src = await invoke<string>('render_edit_draft', { fileId, picasa: tools });
      if (id === lightDraftId && Number(fileInfo.value?.id) === fileId && activeTool.value === 'Automask') effectPreviewSrc.value = src;
    } catch (error) { console.error('Automask preview failed:', error); }
  }, 120);
});
// Apply (or Enter): the selection (if any) is saved as a mask, the adjustment goes into the list
// (replacing the one being changed) as one history step, and the list shows it.
async function commitLight() {
  if (activeTool.value !== 'Automask' || isProcessing.value || !fileInfo.value?.id || selectBusy.value) return;
  const fileId = Number(fileInfo.value.id);
  const entry: LightAdj = { ...clonePlain(lightDraft.value), enabled: true };
  isProcessing.value = true;
  try {
    const layer = effectMaskRef.value;
    if (maskMode.value) {
      if (layer && (maskInverted.value || !layer.isEmpty())) {
        entry.mask = await invoke<string>('save_effect_mask', { fileId, png: layer.exportPng(), feather: maskFeather.value,
          inverted: maskInverted.value, info: layer.info() });
        entry.region = maskRegion.value || (maskTool.value === 'paint' ? 'brush' : maskTool.value);
      } else { entry.mask = null; }
    }
    if (!entry.mask) entry.region = 'whole';
    const local = lightListWithDraft(entry);
    const tools = clonePlain(picasaTools.value);
    tools.local = local;
    ++lightDraftId;
    // The result stays up until the saved step's preview replaces it.
    const preview = await invoke<string>('render_edit_draft', { fileId, picasa: tools });
    if (Number(fileInfo.value?.id) !== fileId || activeTool.value !== 'Automask') return;
    effectPreviewSrc.value = preview;
    effectApplied = true;
    maskMode.value = false;
    picasaTools.value = { ...picasaTools.value, local, lastStep: `Automask: ${lightLabel(entry)}` };
    activeTool.value = '';
    editorTab.value = 'tuning';
  } catch (error) {
    historyError.value = String(error);
  } finally { isProcessing.value = false; }
}

// Pointing at the photo before anything is picked: the model's region under the pointer lights up;
// a click picks it (Click to select).
const hoverMaskSrc = ref('');
let hoverRequest = 0;
async function onSelectHover(p: { x: number; y: number }) {
  if (activeTool.value !== 'Automask' || maskTool.value !== 'select' || maskRegion.value || selectPoints.value.length
      || selectBusy.value || isProcessing.value || preparedFileId !== Number(fileInfo.value?.id)) return;
  const id = ++hoverRequest, fileId = Number(fileInfo.value.id);
  try {
    const r = await invoke<{ png: string }>('select_points', { fileId, points: [{ x: p.x, y: p.y, label: 1 }] });
    if (id === hoverRequest && !maskRegion.value && !selectPoints.value.length && maskMode.value) hoverMaskSrc.value = r.png;
  } catch { /* the hover is a hint only */ }
}
function onSelectLeave() { ++hoverRequest; hoverMaskSrc.value = ''; }
watch([maskRegion, () => selectPoints.value.length, maskMode, activeTool], () => { ++hoverRequest; hoverMaskSrc.value = ''; });

// Auto for the area being picked: the sliders by Auto Balance's rules from that area of the photo
// without any Automask (t_edit_balance), shown on the sliders to fine-tune.
const lightAutoDisabled = computed(() => lightKeepsMask.value || selectBusy.value);
async function autoLight() {
  if (activeTool.value !== 'Automask' || !fileInfo.value?.id || lightAutoDisabled.value) return;
  try {
    const layer = effectMaskRef.value;
    const png = maskMode.value && layer && (maskInverted.value || !layer.isEmpty()) ? layer.exportPng() : null;
    const values = await invoke<Pick<LightAdj, LightSlider>>('automask_auto', { fileId: Number(fileInfo.value.id), picasa: clonePlain(picasaTools.value),
      png, inverted: !!png && maskInverted.value, region: maskMode.value ? (maskRegion.value || 'select') : 'whole' });
    lightDraft.value = { ...lightDraft.value, ...values, detail: true };
  } catch (error) { historyError.value = String(error); }
}
// Auto Balance (round 3, t_edit_balance): the photo decides what needs masking - haze, sky, bright and
// dark areas, people (whole faces only) - and each becomes an Automask entry, as one history step.
// When the darkness of a scene is its mood, the shadows are left alone. Same photo, same result.
async function autoBalance() {
  if (!fileInfo.value?.id || isProcessing.value || !imageReady.value || cropStatus.value === 1 || activeTool.value) return;
  const fileId = Number(fileInfo.value.id);
  isProcessing.value = true; balancing.value = true; lightStatus.value = '';
  try {
    const r = await invoke<{ entries: LightAdj[]; mood: boolean }>('automask_balance', { fileId, picasa: clonePlain(picasaTools.value) });
    if (Number(fileInfo.value?.id) !== fileId) return;
    if (!r.entries.length) {
      lightStatus.value = r.mood ? 'Auto Balance: the darkness here looks intended, so it is left as it is.' : 'Auto Balance: nothing in this photo needs balancing.';
      return;
    }
    if (r.mood) lightStatus.value = 'Auto Balance: the darkness here looks intended, so the shadows are left alone.';
    setLightList([...clonePlain(lightList.value), ...r.entries], 'Auto Balance');
    editorTab.value = 'tuning';
  } catch (error) {
    historyError.value = String(error);
  } finally { isProcessing.value = false; balancing.value = false; }
}

// Automask keys (registry: editor.automask*): M opens it; inside it 1-5 pick where it applies,
// S click to select, X shows the mask, Delete removes the open Automask (or clears a selection),
// Enter applies. Esc cancels as for every tool.
function handleAutomaskKey(event: KeyboardEvent): boolean {
  if (event.ctrlKey || event.metaKey || event.altKey || isProcessing.value || cropStatus.value === 1) return false;
  const target = event.target;
  // A slider or a list checkbox with focus still takes these keys; text fields do not.
  const control = target instanceof HTMLInputElement && (target.type === 'range' || target.type === 'checkbox');
  if (isEditableTarget(target) && !control) return false;
  if (activeTool.value !== 'Automask') {
    if (!activeTool.value && imageReady.value && matchesShortcut('editor.automask', event)) { openLight(-1); return true; }
    return false;
  }
  const region = AUTOMASK_KEYS.find(item => matchesShortcut(item.actionId, event));
  if (region) {
    if (region.region === 'whole') chooseWholePhoto(); else applyRegion(region.region);
    return true;
  }
  if (matchesShortcut('editor.automaskSelect', event)) { startMaskTool('select'); return true; }
  if (matchesShortcut('editor.automaskShowMask', event)) { if (maskMode.value) maskOverlay.value = !maskOverlay.value; return true; }
  if (matchesShortcut('editor.automaskRemove', event)) {
    if (lightEditing.value >= 0) {
      const index = lightEditing.value;
      maskMode.value = false; activeTool.value = '';
      removeLight(index);
      editorTab.value = 'tuning';
    } else if (maskMode.value) { maskRegion.value = ''; selectPoints.value = []; effectMaskRef.value?.clear(); }
    return true;
  }
  if (matchesShortcut('editor.automask', event)) return true;
  if (event.key === 'Enter') { void commitLight(); return true; }
  return false;
}

// A history step made by a masked effect (the badge on Undo / Redo).
function stepMasked(state: any) {
  if (typeof state?.step === 'string' && state.step.startsWith('Automask: ') && state.step !== 'Automask: Whole photo') return true;
  const effects = state?.recipe?.picasa?.effects;
  const last = Array.isArray(effects) ? effects.at(-1) : null;
  return !!(last?.mask && state?.step === last.name);
}

// LAP-087: effects with an Amount slider preview live on the edited picture while the slider,
// colour or centre changes (an unsaved render, no history step); the Original side of A|A and the
// other side of A|B are never touched.
const AMOUNT_EFFECTS = ['Sharpen','Tint','Saturation','Soft Focus','Glow','Filtered B&W','Focal B&W','Graduated Tint', ...PICASA39_EFFECTS];
let effectApplied = false, effectDraftTimer: ReturnType<typeof setTimeout> | null = null, effectDraftId = 0;
watch([activeTool, effectAmount, effectColor, () => JSON.stringify(toolPoint.value)], () => {
  if (effectDraftTimer) clearTimeout(effectDraftTimer);
  const id = ++effectDraftId;
  if (!AMOUNT_EFFECTS.includes(activeTool.value) || !fileInfo.value?.id) {
    if (!effectApplied && !DRAFT_TOOLS.includes(activeTool.value) && activeTool.value !== 'Automask') effectPreviewSrc.value = '';
    return;
  }
  effectDraftTimer = setTimeout(async () => {
    const fileId = Number(fileInfo.value.id);
    const tools = JSON.parse(JSON.stringify(picasaTools.value));
    tools.effects = [...(tools.effects || []), {name:activeTool.value,amount:effectAmount.value/100,color:effectColor.value,cx:toolPoint.value.x,cy:toolPoint.value.y}];
    try {
      const src = await invoke<string>('render_edit_draft', { fileId, picasa: tools });
      if (id === effectDraftId && Number(fileInfo.value?.id) === fileId && AMOUNT_EFFECTS.includes(activeTool.value)) effectPreviewSrc.value = src;
    } catch (error) { console.error('Effect preview failed:', error); }
  }, 120);
});
watch(imageSrc, () => { effectApplied = false; if (!AMOUNT_EFFECTS.includes(activeTool.value) && !DRAFT_TOOLS.includes(activeTool.value) && activeTool.value !== 'Automask') effectPreviewSrc.value = ''; });

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
  } else if (activeTool.value==='Temperature Picker') {
    void pickNeutralTemperature(point);
  } else if (activeTool.value==='Neutral Color Picker') {
    const canvas=document.createElement('canvas');canvas.width=image.naturalWidth;canvas.height=image.naturalHeight;
    const context=canvas.getContext('2d')!;context.drawImage(image,0,0);
    const pixel=context.getImageData(Math.min(canvas.width-1,Math.floor(point.x*canvas.width)),Math.min(canvas.height-1,Math.floor(point.y*canvas.height)),1,1).data;
    const gray=(pixel[0]+pixel[1]+pixel[2])/3;
    picasaTools.value.neutral=[pixel[0],pixel[1],pixel[2]].map(v=>gray/Math.max(1,v));
    picasaTools.value.lastStep='Neutral Color Picker';activeTool.value='';
  }
}

function retouchLayer() {
  return {sx:retouchSource.value!.x,sy:retouchSource.value!.y,tx:retouchTarget.value!.x,ty:retouchTarget.value!.y,radius:brushSize.value/100};
}

// Tuning eyedropper (Picasa's whitept, LAP-071): the photo is rendered as it stands at the
// temperature step (the edits before it, temperature 0, no effects or curves after it), the clicked
// spot (5 x 5 average) is read, and Color Temperature is set to (B - R) / 60 - the temperature step
// adds 30 x t to red and takes it from blue, so that spot ends with red equal to blue. The result
// previews at once, until the saved step's preview replaces it.
async function pickNeutralTemperature(point:{x:number;y:number}) {
  const fileId = Number(fileInfo.value?.id);
  if (!fileId) return;
  const stage = JSON.parse(JSON.stringify(picasaTools.value));
  stage.temperature = 0; stage.effects = []; delete stage.curves;
  try {
    const src = await invoke<string>('render_edit_draft', { fileId, picasa: stage });
    const img = new Image(); img.src = src; await img.decode();
    const canvas = document.createElement('canvas'); canvas.width = img.naturalWidth; canvas.height = img.naturalHeight;
    const context = canvas.getContext('2d')!; context.drawImage(img, 0, 0);
    const x0 = Math.max(0, Math.min(canvas.width - 5, Math.round(point.x * (canvas.width - 1)) - 2));
    const y0 = Math.max(0, Math.min(canvas.height - 5, Math.round(point.y * (canvas.height - 1)) - 2));
    const data = context.getImageData(x0, y0, Math.min(5, canvas.width), Math.min(5, canvas.height)).data;
    let red = 0, blue = 0;
    for (let i = 0; i < data.length; i += 4) { red += data[i]; blue += data[i + 2]; }
    const n = data.length / 4;
    const slider = Math.max(-100, Math.min(100, Math.round((blue / n - red / n) * 100 / 60)));
    if (activeTool.value !== 'Temperature Picker' || Number(fileInfo.value?.id) !== fileId) return;
    picasaTools.value.temperature = slider; picasaTools.value.lastStep = 'Color Temperature';
    activeTool.value = '';
    effectApplied = true;
    const preview = await invoke<string>('render_edit_draft', { fileId, picasa: JSON.parse(JSON.stringify(picasaTools.value)) });
    if (Number(fileInfo.value?.id) === fileId && !activeTool.value) effectPreviewSrc.value = preview;
  } catch (error) { console.error('Neutral temperature pick failed:', error); }
}

function applyRetouch() {
  if (!retouchTarget.value || !retouchSource.value) return;
  effectApplied = true;   // keep the live result up until the saved step's preview replaces it
  picasaTools.value.retouch.push(retouchLayer());
  picasaTools.value.lastStep='Retouch';activeTool.value='';
}

// The Text tool's layer as the recipe stores it: the text drawn into a PNG (font, size, colour,
// bold / italic, an outline in black or white, whichever stands out from the colour) placed at
// toolPoint. Shared by the live preview and Apply, so Apply saves exactly what was previewed.
function buildTextLayer() {
  const canvas=document.createElement('canvas');const context=canvas.getContext('2d')!;
  const size=Math.max(12,Math.min(96,textSize.value));
  const font=`${textItalic.value?'italic ':''}${textBold.value?'bold ':''}${size}px ${toolFont.value}`;
  const line=textOutline.value?Math.max(2,Math.round(size/12)):0;
  const pad=2+line;
  context.font=font;
  canvas.width=Math.min(2048,Math.ceil(context.measureText(toolText.value).width)+pad*2);canvas.height=size*2+line*2;
  context.font=font;context.textBaseline='top';context.lineJoin='round';
  if (line) {
    const hex=effectColor.value.replace('#','');
    const rgb=[0,2,4].map(i=>parseInt(hex.slice(i,i+2),16)||0);
    context.strokeStyle=(rgb[0]*299+rgb[1]*587+rgb[2]*114)/1000>128?'#000000':'#ffffff';
    context.lineWidth=line*2;context.strokeText(toolText.value,pad,pad);
  }
  context.fillStyle=effectColor.value;context.fillText(toolText.value,pad,pad);
  return {png:canvas.toDataURL('image/png').split(',')[1],x:toolPoint.value.x,y:toolPoint.value.y,
    width:canvas.width/imageWidth.value,height:canvas.height/imageHeight.value};
}

function applyText() {
  if (!toolText.value) return;
  effectApplied = true;   // keep the live result up until the saved step's preview replaces it
  picasaTools.value.text.push(buildTextLayer());
  picasaTools.value.lastStep='Text';activeTool.value='';
}

// Build 37: Text and Retouch preview live on the photo too (an unsaved render, no history step):
// the text as it is typed, styled and placed; the healed blemish once both areas are picked.
const DRAFT_TOOLS = ['Text', 'Retouch'];
let toolDraftTimer: ReturnType<typeof setTimeout> | null = null, toolDraftId = 0;
watch([activeTool, toolText, toolFont, textSize, effectColor, textBold, textItalic, textOutline, brushSize,
  () => JSON.stringify(toolPoint.value), () => JSON.stringify(retouchTarget.value), () => JSON.stringify(retouchSource.value)], () => {
  if (!DRAFT_TOOLS.includes(activeTool.value) || !fileInfo.value?.id) return;
  if (toolDraftTimer) clearTimeout(toolDraftTimer);
  const id = ++toolDraftId;
  const tool = activeTool.value;
  const ready = tool === 'Text' ? !!toolText.value : !!(retouchTarget.value && retouchSource.value);
  if (!ready) { effectPreviewSrc.value = ''; return; }
  toolDraftTimer = setTimeout(async () => {
    const fileId = Number(fileInfo.value.id);
    const tools = JSON.parse(JSON.stringify(picasaTools.value));
    if (tool === 'Text') tools.text = [...(tools.text || []), buildTextLayer()];
    else tools.retouch = [...(tools.retouch || []), retouchLayer()];
    try {
      const src = await invoke<string>('render_edit_draft', { fileId, picasa: tools });
      if (id === toolDraftId && Number(fileInfo.value?.id) === fileId && activeTool.value === tool) effectPreviewSrc.value = src;
    } catch (error) { console.error('Tool preview failed:', error); }
  }, 120);
});

// A moving tone slider (Fill Light, LAP-106; and Highlights, Shadows, Colour Temperature, LAP-108)
// renders the actual tone curve before the gesture is saved, the newest value winning.
const TONE_KEYS = ['fillLight', 'highlights', 'shadows', 'temperature'];
const toneUnsaved = () => TONE_KEYS.some(key =>
  Number(picasaTools.value[key] || 0) !== Number(savedEditState.value.recipe?.picasa?.[key] || 0));
let fillDraftTimer: ReturnType<typeof setTimeout> | null = null;
const fillDraft = latestDraft(async () => {
  const fileId = Number(fileInfo.value?.id);
  if (!fileId || !imageReady.value || restoringHistory || activeTool.value || editorUnmounted) return null;
  if (!toneUnsaved()) return null;
  const tools = JSON.parse(JSON.stringify(picasaTools.value));
  delete tools.curves; // the editor's curve layer follows this uncurved base
  const signature = baseSignature;
  const src = await invoke<string>('render_edit_draft', { fileId, picasa: tools, previewSize: 768 });
  return { src, signature, fileId };
}, ({ src, signature, fileId }) => {
  if (signature === baseSignature && !editorUnmounted && !restoringHistory
      && Number(fileInfo.value?.id) === fileId && !activeTool.value && toneUnsaved()) {
    effectPreviewSrc.value = src;
  }
}, error => { historyError.value = String(error); });
onUnmounted(() => { fillDraft.dispose(); if (fillDraftTimer) clearTimeout(fillDraftTimer); });
watch(() => TONE_KEYS.map(key => picasaTools.value[key]).join(), () => {
  fillDraft.invalidate();
  if (!activeTool.value && !toneUnsaved()) effectPreviewSrc.value = '';
  if (fillDraftTimer) clearTimeout(fillDraftTimer);
  fillDraftTimer = setTimeout(() => fillDraft.request(), 30);
});

// Where the Retouch areas sit on screen (container pixels): the inverse of pickEditorPoint.
const retouchRings = computed(() => {
  const rings: { kind: string; x: number; y: number; r: number }[] = [];
  const image = imageRef.value, box = containerRef.value;
  void scale.value; void displayRotate.value; void effectPreviewSrc.value;
  if (!image || !box || !imageWidth.value || !imageHeight.value) return rings;
  const ir = image.getBoundingClientRect(), cr = box.getBoundingClientRect();
  const cx = ir.left + ir.width / 2 - cr.left, cy = ir.top + ir.height / 2 - cr.top;
  const a = displayRotate.value * Math.PI / 180;
  const r = Math.max(4, brushSize.value / 100 * Math.min(imageWidth.value, imageHeight.value) * scale.value);
  for (const [kind, point] of [['target', retouchTarget.value], ['source', retouchSource.value]] as const) {
    if (!point) continue;
    const px = (point.x * imageWidth.value - imageWidth.value / 2) * (isFlippedX.value ? -1 : 1);
    const py = (point.y * imageHeight.value - imageHeight.value / 2) * (isFlippedY.value ? -1 : 1);
    rings.push({ kind, x: cx + (px * Math.cos(a) - py * Math.sin(a)) * scale.value, y: cy + (px * Math.sin(a) + py * Math.cos(a)) * scale.value, r });
  }
  return rings;
});

// LAP-108: one slider gesture is one history step (sliderSteps): a drag is written shortly after
// release, a double-click reset joins its first click, and a new gesture writes the last one first.
const sliderGesture = sliderSteps({
  clock: { now: () => performance.now(), set: (fn, ms) => setTimeout(fn, ms), clear: handle => clearTimeout(handle as ReturnType<typeof setTimeout>) },
  hasPending: () => pendingRecipe !== null,
  flush: () => { void flushRecipeSteps().catch(err => { console.error('Failed to flush recipe step:', err); }); },
});
// A slider is being moved: pressed (its first input comes before it takes focus) or focused.
function sliderMoving() {
  const active = document.activeElement;
  return sliderGesture.moving() || (active instanceof HTMLInputElement && active.type === 'range');
}
function startRangeDrag(event: PointerEvent) {
  if (!(event.target instanceof HTMLInputElement && event.target.type === 'range')) return;
  if (recipeTimer) {
    if (sliderGesture.isIdle()) {
      void flushRecipeSteps().catch(err => { console.error('Failed to flush recipe step:', err); });
    } else {
      clearTimeout(recipeTimer);
      recipeTimer = null;
    }
  }
  sliderGesture.press(event.target, event.clientX, event.clientY);
}
function endRangeDrag() { sliderGesture.release(); }
// A slider's value is written when it is released (LAP-106); a pointer release is left to the gesture.
function onSliderChange() { sliderGesture.change(); }
function resetFillLight() {
  sliderGesture.reset();
  picasaTools.value.fillLight = 0;
  picasaTools.value.lastStep = 'Fill Light';
}

let baseSignature='';
function recipeBaseSignature(doc:any) {
  const {curves, lastStep, ...tools}=doc.recipe?.picasa || {};
  return JSON.stringify([doc.auto_enhance,doc.auto_enhance_preset || 1,doc.recipe?.redEye,tools]);
}
function refreshBasePreview() {
  const doc=savedEditState.value;
  const signature=recipeBaseSignature(doc);
  if (signature===baseSignature) return;
  fillDraft.invalidate(); // a saved full-size base supersedes any in-flight slider draft
  baseSignature=signature;
  const src=getPreviewUrl(fileInfo.value.id,fileInfo.value.file_path,false,Number(fileInfo.value.modified_at||0),config.settings.rawThumbnailSource);
  imageSrc.value=`${src}${src.includes('?')?'&':'?'}original=1&baseEdit=1&rk=${doc.render_key||''}`;
}

watch([rotate, isFlippedX, isFlippedY, cropAngle, cropStatus, brightness, contrast, saturation, hue, blur, selectedFilter,
  () => crop.value.left, () => crop.value.top, () => crop.value.width, () => crop.value.height,
  () => resizeOutput.value.width, () => resizeOutput.value.height, redEye, () => JSON.stringify(picasaTools.value)], () => {
  if (restoringHistory || !imageReady.value || !fileInfo.value || cropStatus.value === 1) return;
  const params = setEditParams();
  if (sliderMoving()) {
    sliderGesture.valueChanged(JSON.stringify(params));
    pendingRecipe = params;
    if (recipeTimer) clearTimeout(recipeTimer);
    // A drag is one step, written after release (sliderGesture).
    recipeTimer = sliderGesture.moving() ? null : setTimeout(() => { void flushRecipeSteps().catch(err => { console.error('Failed to flush recipe step:', err); }); }, 200);
  } else {
    if (pendingRecipe) void flushRecipeSteps().catch(err => { console.error('Failed to flush recipe step:', err); });
    persistRecipeStep(params);
  }
}, { flush: 'post' });

</script>

<style scoped>
.crop-box-active {
  position: absolute;
  border: 1px solid rgba(255, 255, 255, 0.95);
  box-shadow: 0 0 0 9999px rgba(255, 255, 255, 0.55);
  box-sizing: border-box;
  will-change: transform;
  transition: none !important;
}

.crop-box-preview {
  position: absolute;
  border: 1px solid rgba(255, 255, 255, 0.2);
  box-shadow: 0 0 0 9999px rgba(0, 0, 0, 0.85);
  box-sizing: border-box;
  transition: none !important;
}

/* Straighten grid over the photo: a light line with a dark edge so it shows on sky and shadow alike. */
.straighten-grid {
  position: absolute;
  inset: 0;
  pointer-events: none;
  background-image:
    linear-gradient(to right, rgba(255, 255, 255, 0.6) 1px, rgba(0, 0, 0, 0.35) 1px, rgba(0, 0, 0, 0.35) 2px, transparent 2px),
    linear-gradient(to bottom, rgba(255, 255, 255, 0.6) 1px, rgba(0, 0, 0, 0.35) 1px, rgba(0, 0, 0, 0.35) 2px, transparent 2px);
  background-size: 36px 36px;
  background-position: center;
}

.crop-box-done {
  position: absolute;
  box-shadow: 0 0 0 9999px var(--color-base-200);
  box-sizing: border-box;
  will-change: transform;
  transition: none !important;
}

.no-transition {
  transition: none !important;
}

.drag-handle {
  position: absolute;
  background: transparent;
  border: none;
  box-sizing: border-box;
  transition: none !important;
}

.top-left {
  top: -8px;
  left: -8px;
  width: 16px;
  height: 16px;
  cursor: nwse-resize;
  z-index: 20;
}

.top {
  top: -6px;
  left: 8px;
  right: 8px;
  height: 12px;
  cursor: ns-resize;
  z-index: 15;
}

.top-right {
  top: -8px;
  right: -8px;
  width: 16px;
  height: 16px;
  cursor: nesw-resize;
  z-index: 20;
}

.left {
  left: -6px;
  top: 8px;
  bottom: 8px;
  width: 12px;
  cursor: ew-resize;
  z-index: 15;
}

.right {
  right: -6px;
  top: 8px;
  bottom: 8px;
  width: 12px;
  cursor: ew-resize;
  z-index: 15;
}

.bottom-left {
  bottom: -8px;
  left: -8px;
  width: 16px;
  height: 16px;
  cursor: nesw-resize;
  z-index: 20;
}

.bottom {
  bottom: -6px;
  left: 8px;
  right: 8px;
  height: 12px;
  cursor: ns-resize;
  z-index: 15;
}

.bottom-right {
  bottom: -8px;
  right: -8px;
  width: 16px;
  height: 16px;
  cursor: nwse-resize;
  z-index: 20;
}

.crop-panel-container {
  padding: 4px 4px;
  font-family: 'Segoe UI', Tahoma, Arial, sans-serif;
  box-sizing: border-box;
}

.crop-panel-header {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}

.crop-header-icon {
  width: 36px;
  height: 25px;
  border: 1px solid #b5b5b5;
  border-radius: 2px;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12);
  object-fit: cover;
  flex-shrink: 0;
}

.crop-header-title {
  font-size: 15px;
  font-weight: normal;
  color: #383838;
}

.crop-instruction-text {
  font-size: 11px;
  font-weight: 500;
  line-height: 1.35;
  color: #555555;
  margin: 0 0 10px 0;
}

.crop-dropdown-wrap {
  margin-bottom: 10px;
}

.crop-dimension-select {
  width: 100%;
  height: 24px;
  background-color: #ffffff;
  border: 1px solid #7f9db9;
  border-radius: 2px;
  font-size: 12px;
  font-weight: 600;
  color: #222222;
  padding: 1px 6px;
  outline: none;
  box-sizing: border-box;
  cursor: pointer;
}

.crop-thumbnails-row {
  display: flex;
  gap: 6px;
  margin-bottom: 12px;
}

.crop-thumbnail-card {
  flex: 1;
  height: 70px;
  border: 1px solid #b0b0b0;
  border-radius: 2px;
  background: #ffffff;
  padding: 2px;
  box-sizing: border-box;
  cursor: pointer;
  outline: none;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08);
  transition: none !important;
}

.crop-thumbnail-card:hover {
  border-color: #707070;
}

.crop-thumbnail-card-active {
  border: 2px solid #3b82f6;
  padding: 1px;
}

.crop-thumbnail-inner {
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: #eaeaea;
}

.crop-thumbnail-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  pointer-events: none;
  user-select: none;
}

.crop-btn-row {
  display: flex;
  gap: 8px;
  margin-bottom: 6px;
}

.crop-btn-row-center {
  display: flex;
  justify-content: center;
  margin-bottom: 6px;
}

.crop-btn-reset {
  width: 96px;
}

.crop-btn-icon {
  width: 15px;
  height: 15px;
  margin-left: 6px;
  flex-shrink: 0;
}

.picasa-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-family: 'Segoe UI', Tahoma, Arial, sans-serif;
  font-size: 12px;
  font-weight: 500;
  color: #222222;
  height: 25px;
  padding: 0 10px;
  background: linear-gradient(180deg, #fefefe 0%, #ececec 48%, #e1e1e1 52%, #e5e5e5 100%);
  border: 1px solid #9c9c9c;
  border-radius: 3px;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.8), 0 1px 1px rgba(0, 0, 0, 0.08);
  cursor: pointer;
  user-select: none;
  outline: none;
  transition: none !important;
}

.picasa-btn:hover:not(:disabled) {
  background: linear-gradient(180deg, #ffffff 0%, #f4f4f4 48%, #e8e8e8 52%, #eeeeee 100%);
  border-color: #707070;
}

.picasa-btn:active:not(:disabled) {
  background: linear-gradient(180deg, #d4d4d4 0%, #e2e2e2 100%);
  box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.2);
}

.picasa-btn:disabled {
  background: #f0f0f0;
  border-color: #d4d4d4;
  color: #a8a8a8;
  box-shadow: none;
  cursor: default;
}
</style>
