<template>
  <component
    :is="isStandalone ? 'div' : ModalDialog"
    v-bind="isStandalone ? { class: 'w-full h-full flex flex-col flex-1 min-h-0 bg-base-300 overflow-hidden' } : {
      title: 'Folder Manager',
      width: 800,
      height: 560,
      positionKey: 'folder-manager',
      onCancel: clickCancel
    }"
  >
    <div class="flex flex-col flex-1 min-h-0 select-none text-xs text-base-content/85 h-full">
      <div class="flex flex-1 min-h-0 gap-2">
        <!-- Left: LapCasa folders + the whole computer (drives -> folders, lazy) -->
        <div class="w-[21rem] shrink-0 flex flex-col border border-base-content/20 bg-base-100">
          <div class="h-6 px-1.5 flex items-center justify-between border-b border-base-content/15 bg-base-200/60 shrink-0 gap-1">
            <span class="font-semibold truncate">Folders</span>
            <div class="flex items-center gap-1 shrink-0">
              <button type="button" class="px-1.5 h-5 border border-base-content/20 hover:bg-base-content/10"
                :disabled="isLoading || isProcessing" title="Pick a folder with the system dialog and show it here"
                @click="clickBrowse">Browse...</button>
              <TButton :icon="IconRefresh" :buttonSize="'small'" :tooltip="'Reload'"
                :disabled="isLoading || isProcessing" @click="loadData" />
            </div>
          </div>
          <div ref="treeEl" class="flex-1 min-h-0 overflow-auto py-0.5">
            <div v-if="isLoading" class="flex items-center justify-center h-24 gap-2 text-base-content/50">
              <span class="loading loading-spinner loading-xs"></span>
              <span>Loading folders...</span>
            </div>
            <template v-else>
              <div class="px-1.5 h-5 flex items-center font-semibold text-base-content/60 bg-base-200/40">In LapCasa</div>
              <div v-if="lapNodes.length === 0" class="px-3 py-1 text-base-content/50">
                No folders yet. Tick a folder under Computer to add it.
              </div>
              <div v-for="node in visibleLapNodes" :key="'l:' + node.path"
                :data-path="'l:' + node.path"
                class="flex items-center h-5 pr-1 cursor-default whitespace-nowrap"
                :class="rowClass(node, 'l')"
                :style="{ paddingLeft: `calc(2px + ${node.depth * 0.5}ch)` }"
                :title="node.path"
                @click="selectNode(node, 'l')" @dblclick="toggleExpand(node)">
                <button type="button" class="w-3.5 h-3.5 shrink-0 flex items-center justify-center text-base-content/50"
                  :class="{ invisible: !node.hasChildren }" @click.stop="toggleExpand(node)">
                  <span v-if="node.loading" class="loading loading-spinner loading-xs scale-75"></span>
                  <IconRight v-else class="w-3 h-3 transition-transform" :class="{ 'rotate-90': node.expanded }" />
                </button>
                <!-- Tick = in LapCasa. Untick takes it out (files on disk are never touched). -->
                <input type="checkbox" class="checkbox checkbox-xs rounded-none shrink-0 mx-1"
                  :checked="isIncluded(node.path)"
                  :indeterminate.prop="!isIncluded(node.path) && containsRoots(node.path)"
                  :disabled="isProcessing"
                  :title="isIncluded(node.path) ? 'In LapCasa - untick to take it out' : 'Tick to add to LapCasa'"
                  @click.stop @change="toggleIncluded(node, $event)" />
                <span class="truncate flex-1" :class="nameClass(node.path)">{{ node.name }}</span>
                <span v-if="isScanning(node.path)" class="loading loading-spinner loading-xs scale-75 text-primary mr-1" title="Scanning"></span>
                <span v-if="status(node.path) === 'once'" class="ml-1 shrink-0 text-[10px] text-info"
                  :class="stateOpacity(node.path)" :title="stateTitle(node.path)">once</span>
              </div>

              <div class="px-1.5 h-5 mt-1 flex items-center font-semibold text-base-content/60 bg-base-200/40">Computer</div>
              <div v-for="node in visibleDiskNodes" :key="'d:' + node.path"
                :data-path="'d:' + node.path"
                class="flex items-center h-5 pr-1 cursor-default whitespace-nowrap"
                :class="rowClass(node, 'd')"
                :style="{ paddingLeft: `calc(2px + ${node.depth * 0.5}ch)` }"
                :title="node.path"
                @click="selectNode(node, 'd')" @dblclick="toggleExpand(node)">
                <button type="button" class="w-3.5 h-3.5 shrink-0 flex items-center justify-center text-base-content/50"
                  :class="{ invisible: !node.hasChildren }" @click.stop="toggleExpand(node)">
                  <span v-if="node.loading" class="loading loading-spinner loading-xs scale-75"></span>
                  <IconRight v-else class="w-3 h-3 transition-transform" :class="{ 'rotate-90': node.expanded }" />
                </button>
                <!-- Tick = in LapCasa. Untick takes it out (files on disk are never touched). -->
                <input type="checkbox" class="checkbox checkbox-xs rounded-none shrink-0 mx-1"
                  :checked="isIncluded(node.path)"
                  :indeterminate.prop="!isIncluded(node.path) && containsRoots(node.path)"
                  :disabled="isProcessing"
                  :title="isIncluded(node.path) ? 'In LapCasa - untick to take it out' : 'Tick to add to LapCasa'"
                  @click.stop @change="toggleIncluded(node, $event)" />
                <span class="truncate flex-1" :class="nameClass(node.path)">{{ node.name }}</span>
                <span v-if="status(node.path) === 'none' && containsRoots(node.path)"
                  class="w-1.5 h-1.5 rounded-full bg-success/70 mr-1 shrink-0" title="Contains folders in LapCasa"></span>
                <span v-if="status(node.path) === 'once'" class="ml-1 shrink-0 text-[10px] text-info"
                  :class="stateOpacity(node.path)" :title="stateTitle(node.path)">once</span>
              </div>
            </template>
          </div>
        </div>

        <!-- Right: options for the selected folder -->
        <div class="flex-1 min-w-0 flex flex-col border border-base-content/20 bg-base-100 p-3 overflow-y-auto">
          <div v-if="!selected" class="flex-1 flex flex-col items-center justify-center text-base-content/55 text-center px-6 gap-2">
            <div>Pick a folder on the left.</div>
            <div><b>Tick</b> a folder to add it to LapCasa, <b>untick</b> it to take it out. Subfolders follow their parent unless you tick or untick them separately. Files on disk are never deleted.</div>
          </div>

          <div v-else class="flex flex-col gap-3">
            <div class="min-w-0">
              <div class="font-semibold text-sm truncate">{{ selected.name }}</div>
              <div class="text-base-content/55 break-all">{{ selected.path }}</div>
              <div class="mt-1 text-base-content/70">{{ selectedSummary }}</div>
            </div>

            <div :key="radioKey" class="flex flex-col gap-1.5">
              <div class="font-semibold">Folder options</div>
              <label class="flex items-start gap-2 cursor-pointer">
                <input type="radio" name="folderMode" class="radio radio-xs mt-0.5"
                  :checked="selectedStatus === 'always'" :disabled="isProcessing" @change="chooseMode('always')" />
                <IconUnhide class="w-3.5 h-3.5 mt-0.5 text-success shrink-0" />
                <span>
                  <span class="font-medium">Scan Always</span>
                  <span class="block text-base-content/55">Include this folder and keep it in sync with the disk.</span>
                </span>
              </label>
              <label class="flex items-start gap-2 cursor-pointer">
                <input type="radio" name="folderMode" class="radio radio-xs mt-0.5"
                  :checked="selectedStatus === 'once'" :disabled="isProcessing" @change="chooseMode('once')" />
                <IconOk class="w-3.5 h-3.5 mt-0.5 text-info shrink-0" />
                <span>
                  <span class="font-medium">Scan Once</span>
                  <span class="block text-base-content/55">Include it and scan it now, but don't watch it for changes.</span>
                </span>
              </label>
              <label class="flex items-start gap-2 cursor-pointer">
                <input type="radio" name="folderMode" class="radio radio-xs mt-0.5"
                  :checked="selectedStatus === 'exclude' || selectedStatus === 'none'" :disabled="isProcessing"
                  @change="chooseMode('exclude')" />
                <IconClose class="w-3.5 h-3.5 mt-0.5 text-error shrink-0" />
                <span>
                  <span class="font-medium">Remove from LapCasa</span>
                  <span class="block text-base-content/55">Not shown and not scanned. Files on disk are not deleted.</span>
                </span>
              </label>
              <div v-if="selectedNote" class="text-base-content/55">{{ selectedNote }}</div>
            </div>

            <div v-if="selectedStatus !== 'none' && selectedStatus !== 'exclude' && !isPendingRoot(selected.path)"
              class="flex flex-col gap-1.5 border-t border-base-content/10 pt-2">
              <div class="font-semibold mt-1">Face Detection</div>
              <label class="flex items-center gap-2 cursor-pointer">
                <input type="radio" :name="'faceMode_' + selected.path" class="radio radio-xs"
                  :checked="!selectedFlags.faces"
                  :disabled="isProcessing || !selectedFlagsLoaded"
                  @change="stageFlag('faces', false)" />
                <span>Face Detection On</span>
              </label>
              <label class="flex items-center gap-2 cursor-pointer">
                <input type="radio" :name="'faceMode_' + selected.path" class="radio radio-xs"
                  :checked="selectedFlags.faces"
                  :disabled="isProcessing || !selectedFlagsLoaded"
                  @change="stageFlag('faces', true)" />
                <span>Face Detection Off</span>
              </label>
              
              <div class="font-semibold mt-2">Search</div>
              <label class="flex items-center gap-2 cursor-pointer">
                <input type="checkbox" class="checkbox checkbox-xs" :checked="!selectedFlags.search"
                  :disabled="isProcessing || !selectedFlagsLoaded"
                  @change="stageFlag('search', !($event.target as HTMLInputElement).checked)" />
                <span>Include this folder in search</span>
              </label>
            </div>
          </div>
        </div>
      </div>

      <!-- Footer -->
      <div class="flex justify-between items-center pt-2 mt-2 border-t border-base-content/10 shrink-0 gap-2">
        <div class="text-base-content/50 truncate">
          <span v-if="isProcessing" class="flex items-center gap-1.5 text-primary">
            <span class="loading loading-spinner loading-xs"></span>
            {{ progressText || 'Applying changes...' }}
          </span>
          <span v-else-if="pendingCount > 0">{{ pendingCount }} change(s) will apply when you click OK.</span>
          <span v-else>Changes apply when you click OK. Files on disk are never deleted.</span>
        </div>
        <div class="flex items-center gap-2 shrink-0">
          <button type="button" class="t-button-default px-4 py-1 text-xs" :disabled="isProcessing" @click="clickCancel">Cancel</button>
          <button type="button" class="t-button-primary px-5 py-1 text-xs" :disabled="isProcessing" @click="clickOk">OK</button>
        </div>
      </div>
    </div>

    <MessageBox
      v-if="showRemoveConfirm"
      title="Remove from LapCasa"
      :message="removeConfirmMessage"
      OkText="Remove"
      cancelText="Cancel"
      :warningOk="true"
      @ok="confirmRemove"
      @cancel="cancelRemove"
    />
  </component>
</template>

<script setup lang="ts">
// Folder Manager (also File > Add Folder).
// The tree shows the folders already in LapCasa and the whole computer.
// - A folder outside LapCasa: Scan Always / Scan Once adds it as a new album
//   root (albums nested inside it are merged into it) and scans it right away.
// - A folder inside an album root: Scan Always / Scan Once / Remove set a rule
//   that its subfolders inherit; Remove hides it and stops scanning it.
// - An album root: Remove takes the whole album out of LapCasa's catalog.
// Edits are staged and applied on OK. Nothing here deletes or moves files on
// disk, and nothing here ever removes a library.
import { ref, computed, nextTick, onMounted } from 'vue';
import { emit as tauriEmit } from '@tauri-apps/api/event';
import ModalDialog from '@/components/ModalDialog.vue';
import MessageBox from '@/components/MessageBox.vue';
import TButton from '@/components/TButton.vue';
import { useToast } from '@/common/toast';
import { libConfig } from '@/common/config';
import { isWin, openFolderDialog } from '@/common/utils';
import {
  getAllAlbums,
  listFolderTree,
  listDrives,
  addAlbumRoot,
  removeAlbum,
  getFolderRules,
  setFolderRule,
  getFolderFacesExcluded,
  getFolderSearchExcluded,
  setFolderFacesExcluded,
  setFolderSearchExcluded,
} from '@/common/api';
import { IconClose, IconFolder, IconOk, IconRefresh, IconRight, IconUnhide } from '@/common/icons';

type Mode = 'always' | 'once' | 'exclude';
type Status = Mode | 'none';
type Section = 'l' | 'd';

interface Rule { path: string; key: string; mode: Mode }
interface Root { id: number; path: string; name: string; key: string }
interface TreeNode {
  path: string;
  name: string;
  depth: number;
  hasChildren: boolean;
  expanded: boolean;
  loading: boolean;
  loaded: boolean;
  children: TreeNode[];
}
type Op =
  | { kind: 'add'; path: string; mode: 'always' | 'once' }
  | { kind: 'removeRoot'; path: string }
  | { kind: 'rule'; path: string; mode: Mode };
interface FolderFlags { faces: boolean; search: boolean }

const props = defineProps({
  initialPath: { type: String, default: '' },
  isStandalone: { type: Boolean, default: false },
});
const emit = defineEmits(['cancel', 'ok', 'updated']);
const toast = useToast();

const isLoading = ref(false);
const isProcessing = ref(false);
const progressText = ref('');
const treeEl = ref<HTMLElement | null>(null);
const roots = ref<Root[]>([]);       // album roots incl. staged adds / removals
const rules = ref<Rule[]>([]);       // stored rules incl. staged edits
const lapNodes = ref<TreeNode[]>([]);
const diskNodes = ref<TreeNode[]>([]);
const selected = ref<TreeNode | null>(null);
const selectedSection = ref<Section>('l');
const ops = ref<Op[]>([]);
const flagCache = ref<Record<string, FolderFlags>>({});
const flagEdits = ref<Record<string, { faces?: boolean; search?: boolean }>>({});
const showRemoveConfirm = ref(false);
const radioKey = ref(0);

// --- path keys and rule resolution (mirrors src-tauri/src/t_folder_rules.rs) ---
const normKey = (path: string) => {
  let key = path.replace(/\\/g, '/');
  if (isWin) key = key.replace(/[A-Z]/g, c => c.toLowerCase());
  return key.replace(/\/+$/, '');
};
const within = (ancestor: string, key: string) =>
  key === ancestor || (key.length > ancestor.length && key.startsWith(ancestor) && key[ancestor.length] === '/');
const nearest = (key: string, list: Rule[]) => {
  let best: Rule | null = null;
  for (const rule of list) {
    if (within(rule.key, key) && (!best || rule.key.length > best.key.length)) best = rule;
  }
  return best;
};
const rootOf = (path: string): Root | null => {
  const key = normKey(path);
  let best: Root | null = null;
  for (const root of roots.value) {
    if (within(root.key, key) && (!best || root.key.length > best.key.length)) best = root;
  }
  return best;
};
const isRoot = (path: string) => roots.value.some(root => root.key === normKey(path));
const isPendingRoot = (path: string) => roots.value.some(root => root.key === normKey(path) && root.id <= 0);
const containsRoots = (path: string) => {
  const key = normKey(path);
  return roots.value.some(root => root.key !== key && within(key, root.key));
};
const explicitMode = (path: string): Mode | null =>
  rules.value.find(rule => rule.key === normKey(path))?.mode || null;
const inheritedRule = (path: string) => {
  const key = normKey(path);
  return nearest(key, rules.value.filter(rule => rule.key !== key));
};
const status = (path: string): Status => {
  if (!rootOf(path)) return 'none';
  return nearest(normKey(path), rules.value)?.mode || 'always';
};

// Same semantics as set_rule_on: descendants follow, redundant rules dropped.
const applyRuleLocal = (path: string, mode: Mode | null) => {
  const key = normKey(path);
  const inherited = inheritedRule(path)?.mode || 'always';
  const next = rules.value.filter(rule => !within(key, rule.key));
  if (mode && mode !== inherited) next.push({ path, key, mode });
  rules.value = next;
};

// --- display helpers ---
const modeLabel = (mode: Status) =>
  mode === 'always' ? 'Scan Always' : mode === 'once' ? 'Scan Once'
    : mode === 'exclude' ? 'Removed from LapCasa' : 'Not in LapCasa';
const stateTitle = (path: string) => {
  const st = status(path);
  if (st === 'none') return containsRoots(path) ? 'Not in LapCasa (contains LapCasa folders)' : 'Not in LapCasa';
  return `${modeLabel(st)}${explicitMode(path) || isRoot(path) ? '' : ' (inherited)'}`;
};
const stateOpacity = (path: string) => (explicitMode(path) || isRoot(path) ? 'opacity-100' : 'opacity-35');
const folderIconClass = (path: string) => {
  const st = status(path);
  return st === 'none' || st === 'exclude' ? 'text-base-content/35' : 'text-warning/90';
};
const nameClass = (path: string) => {
  const st = status(path);
  if (st === 'exclude') return 'text-base-content/40 line-through decoration-base-content/30';
  if (st === 'none') return 'text-base-content/70';
  return isRoot(path) ? 'font-semibold' : '';
};
const rowClass = (node: TreeNode, section: Section) =>
  selected.value?.path === node.path && selectedSection.value === section ? 'bg-primary/25' : 'hover:bg-base-content/5';

const isScanning = (path: string) => {
  const root = roots.value.find(r => r.key === normKey(path));
  if (!root || root.id <= 0) return false;
  return Number((libConfig.index.albumQueue as any[])[0] || 0) === root.id && Number(libConfig.index.status) === 1;
};

// --- tree ---
const makeNode = (path: string, name: string, depth: number, hasChildren: boolean): TreeNode => ({
  path, name, depth, hasChildren, expanded: false, loading: false, loaded: false, children: [],
});
const flatten = (nodes: TreeNode[]) => {
  const out: TreeNode[] = [];
  const walk = (list: TreeNode[]) => {
    for (const node of list) {
      out.push(node);
      if (node.expanded) walk(node.children);
    }
  };
  walk(nodes);
  return out;
};
const visibleLapNodes = computed(() => flatten(lapNodes.value));
const visibleDiskNodes = computed(() => flatten(diskNodes.value));

const loadChildren = async (node: TreeNode) => {
  if (node.loaded || node.loading) return;
  node.loading = true;
  try {
    const entries = (await listFolderTree(node.path)) || [];
    node.children = entries.map((entry: any) => makeNode(entry.path, entry.name, node.depth + 1, !!entry.has_subfolders));
    node.hasChildren = node.children.length > 0;
    node.loaded = true;
  } catch (error) {
    console.error('Failed to list subfolders:', error);
    node.hasChildren = false;
    node.loaded = true;
  } finally {
    node.loading = false;
  }
};

const toggleExpand = async (node: TreeNode) => {
  if (!node.hasChildren) return;
  if (!node.expanded) await loadChildren(node);
  node.expanded = !node.expanded && node.children.length > 0;
};

const rootName = (path: string) => path.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || path;
const rebuildLapNodes = () => {
  const previous = new Map(lapNodes.value.map(node => [node.path, node]));
  lapNodes.value = roots.value.map(root => previous.get(root.path) || makeNode(root.path, root.name, 0, true));
};

// Expand the tree down to `path` and select it.
const revealPath = async (path: string, section: Section) => {
  const wanted = normKey(path);
  let level = section === 'l' ? lapNodes.value : diskNodes.value;
  let found: TreeNode | null = null;
  for (;;) {
    const next = level.find(node => within(normKey(node.path), wanted));
    if (!next) break;
    found = next;
    if (normKey(next.path) === wanted) break;
    await loadChildren(next);
    next.expanded = next.children.length > 0;
    level = next.children;
  }
  if (!found) return false;
  await selectNode(found, section);
  await nextTick();
  treeEl.value?.querySelector(`[data-path="${CSS.escape(section + ':' + found.path)}"]`)?.scrollIntoView({ block: 'nearest' });
  return true;
};

const loadData = async () => {
  isLoading.value = true;
  try {
    const [albums, stored, drives] = await Promise.all([getAllAlbums(), getFolderRules(), listDrives()]);
    roots.value = (albums || []).map((album: any) => ({
      id: Number(album.id), path: album.path, key: normKey(album.path),
      name: album.name || rootName(album.path),
    }));
    for (const album of albums || []) {
      flagCache.value[album.path] = { faces: !!album.faces_excluded, search: !!album.is_excluded_from_search };
    }
    rules.value = (stored || []).map((rule: any) => ({ path: rule.path, key: normKey(rule.path), mode: rule.mode }));
    const staged = ops.value;
    ops.value = [];
    for (const op of staged) stageLocal(op); // keep staged edits across reloads
    ops.value = staged;
    lapNodes.value = [];
    rebuildLapNodes();
    diskNodes.value = (drives || []).map((drive: string) => makeNode(drive, drive.replace(/\\$/, ''), 0, true));
    selected.value = null;
    if (props.initialPath) {
      const section: Section = rootOf(props.initialPath) ? 'l' : 'd';
      await revealPath(props.initialPath, section);
    }
  } catch (error) {
    console.error('Failed to load folder manager data:', error);
    toast.error('Failed to load folders');
  } finally {
    isLoading.value = false;
  }
};

const clickBrowse = async () => {
  const picked = await openFolderDialog('Select a folder');
  if (!picked || Array.isArray(picked)) return;
  const path = String(picked);
  if (!(await revealPath(path, rootOf(path) ? 'l' : 'd'))) toast.error(`Cannot show ${path}`);
};

// --- selection and staging ---
const selectedStatus = computed<Status>(() => (selected.value ? status(selected.value.path) : 'none'));
const selectedSummary = computed(() => {
  if (!selected.value) return '';
  const path = selected.value.path;
  const root = rootOf(path);
  if (!root) return 'Not in LapCasa.';
  if (isRoot(path)) return isPendingRoot(path) ? 'Will be added to LapCasa.' : 'Folder in LapCasa.';
  return `Inside ${root.path}.`;
});
const selectedNote = computed(() => {
  if (!selected.value) return '';
  const path = selected.value.path;
  if (selectedStatus.value === 'none') {
    const nested = roots.value.filter(root => root.key !== normKey(path) && within(normKey(path), root.key));
    return nested.length > 0
      ? `Choosing Scan Always or Scan Once adds this folder; ${nested.map(r => r.path).join(', ')} will be merged into it.`
      : 'Choose Scan Always or Scan Once to add this folder to LapCasa and scan it now.';
  }
  if (isRoot(path)) return '';
  if (explicitMode(path)) return 'Set on this folder; its subfolders follow it unless set separately.';
  const from = inheritedRule(path);
  return from ? `Inherited from ${from.path}` : `Inherited from ${rootOf(path)?.path}`;
});

const selectedFlagsLoaded = computed(() => !!selected.value && !!flagCache.value[selected.value.path]);
const selectedFlags = computed<FolderFlags>(() => {
  const path = selected.value?.path || '';
  const base = flagCache.value[path] || { faces: false, search: false };
  const edit = flagEdits.value[path];
  return { faces: edit?.faces ?? base.faces, search: edit?.search ?? base.search };
});

const selectNode = async (node: TreeNode, section: Section) => {
  selected.value = node;
  selectedSection.value = section;
  if (rootOf(node.path) && !isPendingRoot(node.path) && !flagCache.value[node.path]) {
    const [faces, search] = await Promise.all([getFolderFacesExcluded(node.path), getFolderSearchExcluded(node.path)]);
    flagCache.value[node.path] = { faces: !!faces, search: !!search };
  }
};

const stageLocal = (op: Op) => {
  if (op.kind === 'add') {
    const key = normKey(op.path);
    roots.value = roots.value.filter(root => !within(key, root.key)); // nested roots merge in
    roots.value.push({ id: 0, path: op.path, key, name: rootName(op.path) });
    rules.value = rules.value.filter(rule => rule.key !== key);
    if (op.mode === 'once') rules.value.push({ path: op.path, key, mode: 'once' });
  } else if (op.kind === 'removeRoot') {
    const key = normKey(op.path);
    roots.value = roots.value.filter(root => root.key !== key);
    rules.value = rules.value.filter(rule => !within(key, rule.key));
  } else {
    applyRuleLocal(op.path, op.mode);
  }
  ops.value.push(op);
  rebuildLapNodes();
};

const isIncluded = (path: string) => {
  const st = status(path);
  return st === 'always' || st === 'once';
};

// Tick box on a tree row: tick adds / re-includes, untick takes the folder out.
// Unticking a whole top-level folder asks first (it drops LapCasa's data for it);
// a subfolder is just hidden, which a tick undoes, so it needs no prompt.
const toggleIncluded = async (node: TreeNode, event: Event) => {
  const box = event.target as HTMLInputElement;
  await selectNode(node, rootOf(node.path) ? 'l' : 'd');
  const current = status(node.path);
  if (current === 'always' || current === 'once') {
    box.checked = true; // stays ticked until the change is staged / confirmed
    if (isRoot(node.path) && !isPendingRoot(node.path)) {
      showRemoveConfirm.value = true;
      return;
    }
    confirmRemove();
    return;
  }
  chooseMode('always');
};

const chooseMode = (mode: Mode) => {
  const node = selected.value;
  if (!node) return;
  const current = status(node.path);
  if (mode === current || (mode === 'exclude' && current === 'none')) return;
  if (current === 'none') {
    stageLocal({ kind: 'add', path: node.path, mode: mode as 'always' | 'once' });
    return;
  }
  if (mode === 'exclude') {
    showRemoveConfirm.value = true;
    return;
  }
  stageLocal({ kind: 'rule', path: node.path, mode });
};

const removeConfirmMessage = computed(() => {
  const node = selected.value;
  if (node && isRoot(node.path)) {
    return `Remove "${node.name}" from LapCasa?\n\n`
      + 'Its photos and LapCasa\'s information about them (people, tags, ratings kept in LapCasa) leave the catalog. '
      + 'Files on your disk are NOT deleted, and you can add the folder again at any time.';
  }
  return `Remove "${node?.name}" and its subfolders from LapCasa?\n\n`
    + 'Its photos will be hidden and the folder will no longer be scanned. '
    + 'Files on your disk are NOT deleted, and you can set the folder back to Scan Always at any time.';
});

const confirmRemove = () => {
  showRemoveConfirm.value = false;
  const node = selected.value;
  if (!node) return;
  if (isPendingRoot(node.path)) {
    // Undo a not-yet-applied add.
    const key = normKey(node.path);
    ops.value = ops.value.filter(op => !(op.kind === 'add' && normKey(op.path) === key));
    loadData();
    return;
  }
  stageLocal(isRoot(node.path) ? { kind: 'removeRoot', path: node.path } : { kind: 'rule', path: node.path, mode: 'exclude' });
};

const cancelRemove = () => {
  showRemoveConfirm.value = false;
  radioKey.value++; // re-render radios so the previous choice shows again
};

const stageFlag = (flag: 'faces' | 'search', excluded: boolean) => {
  const node = selected.value;
  if (!node) return;
  const edit = flagEdits.value[node.path] || {};
  edit[flag] = excluded;
  flagEdits.value[node.path] = edit;
};

const pendingCount = computed(() => ops.value.length + Object.keys(flagEdits.value).length);

// --- apply ---
const clickOk = async () => {
  if (pendingCount.value === 0) {
    emit('ok');
    return;
  }
  isProcessing.value = true;
  const scanIds = new Set<number>();
  const removedIds = new Set<number>();
  const addedIds: number[] = [];
  let live: any[] = (await getAllAlbums()) || [];
  const albumIdFor = (path: string) => {
    const key = normKey(path);
    let best: any = null;
    for (const album of live) {
      const albumKey = normKey(album.path);
      if (within(albumKey, key) && (!best || albumKey.length > normKey(best.path).length)) best = album;
    }
    return best ? Number(best.id) : 0;
  };
  try {
    // Applied in the order chosen: a parent's choice resets its subfolders,
    // later edits on subfolders override it.
    for (const op of ops.value) {
      if (op.kind === 'add') {
        progressText.value = `Adding ${op.path}...`;
        const result: any = await addAlbumRoot(op.path, op.mode);
        const id = Number(result?.album?.id || 0);
        if (id > 0) { addedIds.push(id); scanIds.add(id); }
        for (const merged of result?.merged_album_ids || []) { removedIds.add(Number(merged)); scanIds.delete(Number(merged)); }
        live = (await getAllAlbums()) || [];
      } else if (op.kind === 'removeRoot') {
        progressText.value = `Removing ${op.path}...`;
        const id = albumIdFor(op.path);
        if (id > 0 && normKey(live.find(a => Number(a.id) === id)?.path || '') === normKey(op.path)) {
          if ((await removeAlbum(id)) === null) throw new Error(`Could not remove ${op.path}`);
          removedIds.add(id);
          scanIds.delete(id);
        }
        await setFolderRule(op.path, null); // drop rules left under the old root
        live = (await getAllAlbums()) || [];
      } else {
        await setFolderRule(op.path, op.mode);
        const id = albumIdFor(op.path);
        if (op.mode !== 'exclude' && id > 0) scanIds.add(id);
      }
    }
    for (const [path, edit] of Object.entries(flagEdits.value)) {
      const id = albumIdFor(path);
      if (id <= 0) continue;
      if (edit.faces !== undefined) await setFolderFacesExcluded(id, path, edit.faces);
      if (edit.search !== undefined) await setFolderSearchExcluded(id, path, edit.search);
    }

    ops.value = [];
    flagEdits.value = {};
    // The main window (Home.vue) owns the scan queue; this dialog may also run in Settings.
    // Explicit scans run even when background processing is off.
    await tauriEmit('folder-manager-scan-albums', { albumIds: [...scanIds], removedAlbumIds: [...removedIds] });
    if (addedIds.length > 0 || removedIds.size > 0) {
      await tauriEmit('album-roots-changed', { selectAlbumId: addedIds[0] || 0 });
    }
    await tauriEmit('albums-refreshed', { albums: live, refreshFolders: true });
    await tauriEmit('library-total-refreshed');
    await tauriEmit('refresh-content');
    toast.success(addedIds.length > 0 ? 'Folder added - scanning now' : 'Folder Manager changes applied');
    emit('updated');
    emit('ok');
  } catch (error: any) {
    console.error('Failed to apply folder manager changes:', error);
    toast.error(String(error?.message || error || 'Failed to apply changes'));
    // Whatever did apply must still reach the album list and the scan queue.
    await tauriEmit('folder-manager-scan-albums', { albumIds: [...scanIds], removedAlbumIds: [...removedIds] });
    await tauriEmit('album-roots-changed', { selectAlbumId: addedIds[0] || 0 });
    ops.value = [];
    flagEdits.value = {};
    await loadData();
  } finally {
    isProcessing.value = false;
    progressText.value = '';
  }
};

const clickCancel = () => {
  if (isProcessing.value) return;
  emit('cancel');
};

onMounted(() => {
  loadData();
});
</script>
