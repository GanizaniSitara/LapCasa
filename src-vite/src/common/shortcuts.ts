export type ShortcutPlatform = 'mac' | 'windows' | 'linux';

export type ShortcutContext =
  | 'global'
  | 'content'
  | 'media-viewer'
  | 'image-viewer'
  | 'settings'
  | 'image-editor'
  | 'album-list'
  | 'album-folder'
  | 'tag-dialog'
  | 'map';

export type ShortcutActionId =
  | 'app.sidebar.toggle'
  | 'app.scale.increase'
  | 'app.scale.decrease'
  | 'app.scale.reset'
  | 'app.preferences'
  | 'app.search'
  | 'file.openNewWindow'
  | 'file.openExternalApp'
  | 'file.editImage'
  | 'file.copy'
  | 'file.paste'
  | 'file.selectAll'
  | 'file.selectNone'
  | 'file.invertSelection'
  | 'file.rename'
  | 'file.moveTo'
  | 'file.moveToFolder'
  | 'file.exportToFolder'
  | 'file.trash'
  | 'file.reveal'
  | 'file.searchSimilar'
  | 'edit.undo'
  | 'edit.redo'
  | 'editor.apply'
  | 'editor.tabBasic'
  | 'editor.tabTuning'
  | 'editor.tabEffects'
  | 'editor.automask'
  | 'editor.automaskWhole'
  | 'editor.automaskSubject'
  | 'editor.automaskSky'
  | 'editor.automaskPeople'
  | 'editor.automaskBackground'
  | 'editor.automaskSelect'
  | 'editor.automaskShowMask'
  | 'editor.automaskRemove'
  | 'meta.favorite'
  | 'meta.rating.clear'
  | 'meta.rating.one'
  | 'meta.rating.two'
  | 'meta.rating.three'
  | 'meta.rating.four'
  | 'meta.rating.five'
  | 'meta.culling.pick'
  | 'meta.culling.reject'
  | 'meta.culling.unreviewed'
  | 'meta.tag'
  | 'meta.collection'
  | 'meta.comment'
  | 'meta.rotate'
  | 'meta.rotateCounterclockwise'
  | 'meta.info'
  | 'view.quickPreview'
  | 'view.close'
  | 'view.next'
  | 'view.previous'
  | 'view.first'
  | 'view.last'
  | 'view.pageUp'
  | 'view.pageDown'
  | 'view.zoomIn'
  | 'view.zoomOut'
  | 'view.zoomInDirectional'
  | 'view.zoomOutDirectional'
  | 'view.zoomFit'
  | 'view.cycleBackground'
  | 'view.backgroundTheme'
  | 'view.backgroundBlack'
  | 'view.backgroundDarkGray'
  | 'view.backgroundMediumGray'
  | 'view.backgroundLightGray'
  | 'view.backgroundWhite'
  | 'view.togglePane'
  | 'view.thumbnailsSmall'
  | 'view.thumbnailsNormal'
  | 'view.timeline'
  | 'slideshow.toggle';

export type ShortcutModifier = 'ctrl' | 'meta' | 'alt' | 'shift' | 'cmdOrCtrl';

type ShortcutLabel = string | Partial<Record<ShortcutPlatform, string>> & { default?: string };

export interface ShortcutBinding {
  key?: string;
  code?: string;
  modifiers?: readonly ShortcutModifier[];
  platforms?: readonly ShortcutPlatform[];
  allowShift?: boolean;
  label: ShortcutLabel;
}

export interface ShortcutDefinition {
  id: ShortcutActionId;
  contexts: readonly ShortcutContext[];
  defaultBindings: readonly ShortcutBinding[];
  readonly?: boolean;
}

export interface ShortcutEventLike {
  key: string;
  code?: string;
  altKey?: boolean;
  ctrlKey?: boolean;
  metaKey?: boolean;
  shiftKey?: boolean;
}

export const VIEW_BACKGROUND_SHORTCUTS = [
  { actionId: 'view.backgroundTheme', value: 0 },
  { actionId: 'view.backgroundBlack', value: 1 },
  { actionId: 'view.backgroundDarkGray', value: 2 },
  { actionId: 'view.backgroundMediumGray', value: 3 },
  { actionId: 'view.backgroundLightGray', value: 4 },
  { actionId: 'view.backgroundWhite', value: 5 },
] as const satisfies ReadonlyArray<{ actionId: ShortcutActionId; value: number }>;

export const DEFAULT_PLATFORM: ShortcutPlatform = (() => {
  if (typeof navigator === 'undefined') return 'windows';
  const platform = `${navigator.platform || ''} ${navigator.userAgent || ''}`;
  if (/mac/i.test(platform)) return 'mac';
  if (/linux|x11/i.test(platform)) return 'linux';
  return 'windows';
})();

export const SHORTCUTS: readonly ShortcutDefinition[] = [
  {
    id: 'app.sidebar.toggle',
    contexts: ['global'],
    defaultBindings: [
      { code: 'KeyB', modifiers: ['cmdOrCtrl'], allowShift: true, label: { mac: '⌘B', windows: 'Ctrl+B', linux: 'Ctrl+B' } },
    ],
  },
  {
    id: 'app.scale.increase',
    contexts: ['global'],
    defaultBindings: [
      { key: '=', modifiers: ['cmdOrCtrl'], label: { mac: '⌘=', windows: 'Ctrl+=', linux: 'Ctrl+=' } },
      { key: '+', modifiers: ['cmdOrCtrl'], allowShift: true, label: { mac: '⌘+', windows: 'Ctrl++', linux: 'Ctrl++' } },
      { code: 'NumpadAdd', modifiers: ['cmdOrCtrl'], label: { mac: '⌘+', windows: 'Ctrl++', linux: 'Ctrl++' } },
    ],
  },
  {
    id: 'app.scale.decrease',
    contexts: ['global'],
    defaultBindings: [
      { key: '-', modifiers: ['cmdOrCtrl'], label: { mac: '⌘-', windows: 'Ctrl+-', linux: 'Ctrl+-' } },
      { key: '_', modifiers: ['cmdOrCtrl', 'shift'], label: { mac: '⌘-', windows: 'Ctrl+-', linux: 'Ctrl+-' } },
      { code: 'NumpadSubtract', modifiers: ['cmdOrCtrl'], label: { mac: '⌘-', windows: 'Ctrl+-', linux: 'Ctrl+-' } },
    ],
  },
  {
    id: 'app.scale.reset',
    contexts: ['global'],
    defaultBindings: [
      { key: '0', modifiers: ['cmdOrCtrl'], label: { mac: '⌘0', windows: 'Ctrl+0', linux: 'Ctrl+0' } },
      { code: 'Numpad0', modifiers: ['cmdOrCtrl'], label: { mac: '⌘0', windows: 'Ctrl+0', linux: 'Ctrl+0' } },
    ],
  },
  {
    id: 'app.preferences',
    contexts: ['global'],
    defaultBindings: [
      { code: 'Comma', modifiers: ['cmdOrCtrl'], label: { mac: '⌘,', windows: 'Ctrl+,', linux: 'Ctrl+,' } },
    ],
  },
  {
    id: 'app.search',
    contexts: ['global'],
    defaultBindings: [
      { code: 'Slash', modifiers: ['cmdOrCtrl'], label: { mac: '⌘/', windows: 'Ctrl+/', linux: 'Ctrl+/' } },
    ],
  },
  {
    id: 'file.openNewWindow',
    contexts: ['content'],
    defaultBindings: [
      { key: 'Enter', modifiers: ['cmdOrCtrl'], allowShift: true, label: { mac: '⌘⏎', windows: 'Ctrl+Enter', linux: 'Ctrl+Enter' } },
    ],
  },
  {
    id: 'file.openExternalApp',
    contexts: ['content'],
    defaultBindings: [
      { code: 'KeyE', modifiers: ['cmdOrCtrl'], label: { mac: '⌘E', windows: 'Ctrl+E', linux: 'Ctrl+E' } },
    ],
  },
  {
    // Enter, E and Ctrl+3 open the editor, as double-click does (LAP-101). Space stays the quick preview.
    id: 'file.editImage',
    contexts: ['content'],
    defaultBindings: [
      { key: 'Enter', platforms: ['windows', 'linux'], label: { windows: 'Enter', linux: 'Enter' } },
      { code: 'KeyE', allowShift: true, label: 'E' },
      { code: 'Digit3', modifiers: ['cmdOrCtrl'], label: { mac: '⌘3', windows: 'Ctrl+3', linux: 'Ctrl+3' } },
    ],
  },
  {
    id: 'file.copy',
    contexts: ['content'],
    defaultBindings: [
      { code: 'KeyC', modifiers: ['cmdOrCtrl'], allowShift: true, label: { mac: '⌘C', windows: 'Ctrl+C', linux: 'Ctrl+C' } },
    ],
  },
  {
    id: 'file.paste',
    contexts: ['content', 'album-list', 'album-folder'],
    defaultBindings: [
      { code: 'KeyV', modifiers: ['cmdOrCtrl'], label: { mac: '⌘V', windows: 'Ctrl+V', linux: 'Ctrl+V' } },
    ],
  },
  {
    id: 'file.selectAll',
    contexts: ['content'],
    defaultBindings: [
      { code: 'KeyA', modifiers: ['cmdOrCtrl'], label: { mac: '⌘A', windows: 'Ctrl+A', linux: 'Ctrl+A' } },
    ],
  },
  {
    id: 'file.selectNone',
    contexts: ['content'],
    defaultBindings: [
      { code: 'KeyD', modifiers: ['cmdOrCtrl'], label: { mac: '⌘D', windows: 'Ctrl+D', linux: 'Ctrl+D' } },
    ],
  },
  {
    // Ctrl+I is Info (LAP-101).
    id: 'file.invertSelection',
    contexts: ['content'],
    defaultBindings: [
      { code: 'KeyI', modifiers: ['cmdOrCtrl', 'shift'], label: { mac: '⇧⌘I', windows: 'Ctrl+Shift+I', linux: 'Ctrl+Shift+I' } },
    ],
  },
  {
    id: 'file.rename',
    contexts: ['content'],
    defaultBindings: [
      { key: 'Enter', platforms: ['mac'], allowShift: true, label: { mac: '⏎' } },
      { key: 'F2', platforms: ['windows', 'linux'], allowShift: true, label: { windows: 'F2', linux: 'F2' } },
    ],
  },
  {
    id: 'file.moveTo',
    contexts: ['content'],
    defaultBindings: [{ code: 'KeyM', allowShift: true, label: 'M' }],
  },
  {
    id: 'file.moveToFolder',
    contexts: ['content'],
    defaultBindings: [{ code: 'KeyM', modifiers: ['cmdOrCtrl', 'shift'], label: { mac: '⇧⌘M', windows: 'Ctrl+Shift+M', linux: 'Ctrl+Shift+M' } }],
  },
  {
    // Picasa's File > Export Picture to Folder (LAP-049)
    id: 'file.exportToFolder',
    contexts: ['content'],
    defaultBindings: [{ code: 'KeyS', modifiers: ['cmdOrCtrl', 'shift'], label: { mac: '⇧⌘S', windows: 'Ctrl+Shift+S', linux: 'Ctrl+Shift+S' } }],
  },
  {
    id: 'file.trash',
    contexts: ['content'],
    defaultBindings: [
      { key: 'Backspace', modifiers: ['meta'], platforms: ['mac'], allowShift: true, label: { mac: '⌘⌫' } },
      { key: 'Delete', platforms: ['windows', 'linux'], allowShift: true, label: { windows: 'Del', linux: 'Del' } },
    ],
  },
  {
    id: 'file.reveal',
    contexts: ['content'],
    defaultBindings: [
      { code: 'KeyE', modifiers: ['cmdOrCtrl', 'shift'], label: { mac: '⌘⇧E', windows: 'Ctrl+Shift+E', linux: 'Ctrl+Shift+E' } },
      { code: 'KeyR', modifiers: ['cmdOrCtrl'], label: { mac: '⌘R', windows: 'Ctrl+R', linux: 'Ctrl+R' } },
    ],
  },
  {
    id: 'file.searchSimilar',
    contexts: ['content'],
    defaultBindings: [{ code: 'KeyS', allowShift: true, label: 'S' }],
  },
  {
    id: 'meta.favorite',
    contexts: ['content', 'media-viewer', 'image-viewer'],
    defaultBindings: [{ code: 'KeyF', allowShift: true, label: 'F' }],
  },
  {
    id: 'meta.rating.clear',
    contexts: ['content', 'media-viewer', 'image-viewer'],
    defaultBindings: [{ key: '0', label: '0' }],
  },
  {
    id: 'meta.rating.one',
    contexts: ['content', 'media-viewer', 'image-viewer'],
    defaultBindings: [{ key: '1', label: '1' }],
  },
  {
    id: 'meta.rating.two',
    contexts: ['content', 'media-viewer', 'image-viewer'],
    defaultBindings: [{ key: '2', label: '2' }],
  },
  {
    id: 'meta.rating.three',
    contexts: ['content', 'media-viewer', 'image-viewer'],
    defaultBindings: [{ key: '3', label: '3' }],
  },
  {
    id: 'meta.rating.four',
    contexts: ['content', 'media-viewer', 'image-viewer'],
    defaultBindings: [{ key: '4', label: '4' }],
  },
  {
    id: 'meta.rating.five',
    contexts: ['content', 'media-viewer', 'image-viewer'],
    defaultBindings: [{ key: '5', label: '5' }],
  },
  {
    id: 'meta.culling.pick',
    contexts: ['content', 'media-viewer', 'image-viewer'],
    defaultBindings: [{ code: 'KeyP', label: 'P' }],
  },
  {
    id: 'meta.culling.reject',
    contexts: ['content', 'media-viewer', 'image-viewer'],
    defaultBindings: [{ code: 'KeyX', allowShift: true, label: 'X' }],
  },
  {
    id: 'meta.culling.unreviewed',
    contexts: ['content', 'media-viewer', 'image-viewer'],
    defaultBindings: [{ code: 'KeyU', allowShift: true, label: 'U' }],
  },
  {
    id: 'meta.tag',
    contexts: ['content', 'media-viewer', 'image-viewer'],
    defaultBindings: [{ code: 'KeyT', allowShift: true, label: 'T' }],
  },
  {
    id: 'meta.collection',
    contexts: ['content', 'media-viewer', 'image-viewer'],
    defaultBindings: [{ code: 'KeyB', label: 'B' }],
  },
  {
    id: 'meta.comment',
    contexts: ['content', 'media-viewer', 'image-viewer'],
    defaultBindings: [{ code: 'KeyC', allowShift: true, label: 'C' }],
  },
  {
    id: 'meta.rotate',
    contexts: ['content', 'media-viewer', 'image-viewer', 'image-editor'],
    defaultBindings: [{ code: 'KeyR', label: 'R' }],
  },
  {
    id: 'meta.rotateCounterclockwise',
    contexts: ['content', 'media-viewer', 'image-viewer', 'image-editor'],
    defaultBindings: [{ code: 'KeyR', modifiers: ['shift'], label: { mac: '⇧R', windows: 'Shift+R', linux: 'Shift+R' } }],
  },
  {
    id: 'meta.info',
    contexts: ['content', 'media-viewer'],
    defaultBindings: [
      { code: 'KeyI', allowShift: true, label: 'I' },
      { code: 'KeyI', modifiers: ['cmdOrCtrl'], label: { mac: '⌘I', windows: 'Ctrl+I', linux: 'Ctrl+I' } },
    ],
  },
  {
    id: 'view.quickPreview',
    contexts: ['content'],
    defaultBindings: [
      { key: ' ', label: 'Space' },
      { key: 'Space', label: 'Space' },
    ],
  },
  {
    id: 'view.close',
    contexts: ['content', 'image-viewer', 'settings', 'image-editor'],
    defaultBindings: [{ key: 'Escape', label: 'Esc' }],
  },
  {
    id: 'view.next',
    contexts: ['content', 'image-viewer', 'album-list', 'album-folder', 'image-editor'],
    defaultBindings: [{ key: 'ArrowRight', label: '→' }],
  },
  {
    id: 'view.previous',
    contexts: ['content', 'image-viewer', 'album-list', 'album-folder', 'image-editor'],
    defaultBindings: [{ key: 'ArrowLeft', label: '←' }],
  },
  {
    id: 'view.first',
    contexts: ['content', 'image-viewer'],
    defaultBindings: [
      { key: 'ArrowUp', modifiers: ['meta'], platforms: ['mac'], label: { mac: '⌘↑' } },
      { key: 'Home', platforms: ['windows', 'linux'], label: { windows: 'Home', linux: 'Home' } },
    ],
  },
  {
    id: 'view.last',
    contexts: ['content', 'image-viewer'],
    defaultBindings: [
      { key: 'ArrowDown', modifiers: ['meta'], platforms: ['mac'], label: { mac: '⌘↓' } },
      { key: 'End', platforms: ['windows', 'linux'], label: { windows: 'End', linux: 'End' } },
    ],
  },
  {
    id: 'view.pageUp',
    contexts: ['content'],
    defaultBindings: [{ key: 'PageUp', label: { mac: 'Fn↑', windows: 'PgUp', linux: 'PgUp' } }],
  },
  {
    id: 'view.pageDown',
    contexts: ['content'],
    defaultBindings: [{ key: 'PageDown', label: { mac: 'Fn↓', windows: 'PgDn', linux: 'PgDn' } }],
  },
  {
    id: 'view.zoomIn',
    contexts: ['content', 'media-viewer', 'image-viewer', 'map'],
    defaultBindings: [
      { key: '=', label: '=' },
      { key: '+', allowShift: true, label: '+' },
    ],
  },
  {
    id: 'view.zoomOut',
    contexts: ['content', 'media-viewer', 'image-viewer', 'map'],
    defaultBindings: [{ key: '-', label: '-' }],
  },
  {
    id: 'view.zoomInDirectional',
    contexts: ['image-viewer'],
    defaultBindings: [{ key: 'ArrowUp', label: '↑' }],
  },
  {
    id: 'view.zoomOutDirectional',
    contexts: ['image-viewer'],
    defaultBindings: [{ key: 'ArrowDown', label: '↓' }],
  },
  {
    // In the Library, Space is the quick preview (which toggles fit once it is open).
    id: 'view.zoomFit',
    contexts: ['media-viewer', 'image-viewer'],
    defaultBindings: [
      { key: ' ', label: 'Space' },
      { key: 'Space', label: 'Space' },
    ],
  },
  {
    id: 'view.cycleBackground',
    contexts: ['content', 'media-viewer', 'image-viewer'],
    defaultBindings: [
      { code: 'KeyB', modifiers: ['shift'], label: { mac: '⇧B', windows: 'Shift+B', linux: 'Shift+B' } },
    ],
  },
  ...VIEW_BACKGROUND_SHORTCUTS.map(({ actionId, value }) => ({
    id: actionId,
    contexts: ['content', 'media-viewer', 'image-viewer'] as const,
    defaultBindings: [
      {
        code: `Digit${value + 1}`,
        modifiers: ['shift'] as const,
        label: { mac: `⇧${value + 1}`, windows: `Shift+${value + 1}`, linux: `Shift+${value + 1}` },
      },
    ],
  })),
  {
    id: 'view.togglePane',
    contexts: ['image-viewer'],
    defaultBindings: [{ key: 'Tab', label: 'Tab' }],
  },
  {
    id: 'slideshow.toggle',
    contexts: ['content', 'media-viewer', 'image-viewer'],
    defaultBindings: [
      {
        code: 'KeyP',
        modifiers: ['shift'],
        label: { mac: '⇧P', windows: 'Shift+P', linux: 'Shift+P' },
      },
      { code: 'Digit4', modifiers: ['cmdOrCtrl'], label: { mac: '⌘4', windows: 'Ctrl+4', linux: 'Ctrl+4' } },
    ],
  },
  {
    id: 'view.thumbnailsSmall',
    contexts: ['content'],
    defaultBindings: [{ code: 'Digit1', modifiers: ['cmdOrCtrl'], label: { mac: '⌘1', windows: 'Ctrl+1', linux: 'Ctrl+1' } }],
  },
  {
    id: 'view.thumbnailsNormal',
    contexts: ['content'],
    defaultBindings: [{ code: 'Digit2', modifiers: ['cmdOrCtrl'], label: { mac: '⌘2', windows: 'Ctrl+2', linux: 'Ctrl+2' } }],
  },
  {
    id: 'view.timeline',
    contexts: ['content'],
    defaultBindings: [{ code: 'Digit5', modifiers: ['cmdOrCtrl'], label: { mac: '⌘5', windows: 'Ctrl+5', linux: 'Ctrl+5' } }],
  },
  // Editor. Undo/redo (utils.editHistoryAction), Enter and Esc are handled in ImageEditor.vue;
  // they are listed here so every chord stays unique within its context.
  {
    id: 'edit.undo',
    contexts: ['content', 'image-editor'],
    defaultBindings: [{ code: 'KeyZ', modifiers: ['cmdOrCtrl'], label: { mac: '⌘Z', windows: 'Ctrl+Z', linux: 'Ctrl+Z' } }],
  },
  {
    id: 'edit.redo',
    contexts: ['content', 'image-editor'],
    defaultBindings: [
      { code: 'KeyY', modifiers: ['cmdOrCtrl'], label: { mac: '⌘Y', windows: 'Ctrl+Y', linux: 'Ctrl+Y' } },
      { code: 'KeyZ', modifiers: ['cmdOrCtrl', 'shift'], label: { mac: '⇧⌘Z', windows: 'Ctrl+Shift+Z', linux: 'Ctrl+Shift+Z' } },
    ],
  },
  {
    id: 'editor.apply',
    contexts: ['image-editor'],
    defaultBindings: [{ key: 'Enter', label: { mac: '⏎', windows: 'Enter', linux: 'Enter' } }],
  },
  {
    id: 'editor.tabBasic',
    contexts: ['image-editor'],
    defaultBindings: [{ code: 'Digit1', modifiers: ['cmdOrCtrl'], label: { mac: '⌘1', windows: 'Ctrl+1', linux: 'Ctrl+1' } }],
  },
  {
    id: 'editor.tabTuning',
    contexts: ['image-editor'],
    defaultBindings: [{ code: 'Digit2', modifiers: ['cmdOrCtrl'], label: { mac: '⌘2', windows: 'Ctrl+2', linux: 'Ctrl+2' } }],
  },
  {
    id: 'editor.tabEffects',
    contexts: ['image-editor'],
    defaultBindings: [{ code: 'Digit3', modifiers: ['cmdOrCtrl'], label: { mac: '⌘3', windows: 'Ctrl+3', linux: 'Ctrl+3' } }],
  },
  // Automask (LAP-107): M opens it; inside it the number keys pick where it applies.
  {
    id: 'editor.automask',
    contexts: ['image-editor'],
    defaultBindings: [{ code: 'KeyM', label: 'M' }],
  },
  {
    id: 'editor.automaskWhole',
    contexts: ['image-editor'],
    defaultBindings: [{ code: 'Digit1', label: '1' }],
  },
  {
    id: 'editor.automaskSubject',
    contexts: ['image-editor'],
    defaultBindings: [{ code: 'Digit2', label: '2' }],
  },
  {
    id: 'editor.automaskSky',
    contexts: ['image-editor'],
    defaultBindings: [{ code: 'Digit3', label: '3' }],
  },
  {
    id: 'editor.automaskPeople',
    contexts: ['image-editor'],
    defaultBindings: [{ code: 'Digit4', label: '4' }],
  },
  {
    id: 'editor.automaskBackground',
    contexts: ['image-editor'],
    defaultBindings: [{ code: 'Digit5', label: '5' }],
  },
  {
    id: 'editor.automaskSelect',
    contexts: ['image-editor'],
    defaultBindings: [{ code: 'KeyS', label: 'S' }],
  },
  {
    id: 'editor.automaskShowMask',
    contexts: ['image-editor'],
    defaultBindings: [{ code: 'KeyX', label: 'X' }],
  },
  {
    id: 'editor.automaskRemove',
    contexts: ['image-editor'],
    defaultBindings: [{ key: 'Delete', label: { mac: '⌦', windows: 'Delete', linux: 'Delete' } }],
  },
] as const;

const shortcutMap = new Map<ShortcutActionId, ShortcutDefinition>(
  SHORTCUTS.map((shortcut) => [shortcut.id, shortcut]),
);

export function getShortcut(actionId: ShortcutActionId): ShortcutDefinition {
  const shortcut = shortcutMap.get(actionId);
  if (!shortcut) {
    throw new Error(`Unknown shortcut action: ${actionId}`);
  }
  return shortcut;
}

export function getShortcutsByContext(context: ShortcutContext): readonly ShortcutDefinition[] {
  return SHORTCUTS.filter((shortcut) => shortcut.contexts.includes(context));
}

export function getShortcutLabels(
  actionId: ShortcutActionId,
  platform: ShortcutPlatform = DEFAULT_PLATFORM,
): string[] {
  return getShortcut(actionId).defaultBindings
    .filter((binding) => isBindingAvailableOnPlatform(binding, platform))
    .map((binding) => getBindingLabel(binding, platform));
}

export function getShortcutLabel(
  actionId: ShortcutActionId,
  platform: ShortcutPlatform = DEFAULT_PLATFORM,
): string {
  return getShortcutLabels(actionId, platform)[0] ?? '';
}

export function matchesShortcut(
  actionId: ShortcutActionId,
  event: ShortcutEventLike,
  platform: ShortcutPlatform = DEFAULT_PLATFORM,
): boolean {
  return getShortcut(actionId).defaultBindings.some((binding) => matchesBinding(binding, event, platform));
}

export function findShortcutMatches(
  context: ShortcutContext,
  event: ShortcutEventLike,
  platform: ShortcutPlatform = DEFAULT_PLATFORM,
): ShortcutActionId[] {
  return getShortcutsByContext(context)
    .filter((shortcut) => shortcut.defaultBindings.some((binding) => matchesBinding(binding, event, platform)))
    .map((shortcut) => shortcut.id);
}

export function getShortcutConflicts(
  context: ShortcutContext,
  platform: ShortcutPlatform = DEFAULT_PLATFORM,
): Array<{ signature: string; actions: ShortcutActionId[] }> {
  const actionsBySignature = new Map<string, ShortcutActionId[]>();
  const shortcuts = SHORTCUTS.filter((shortcut) =>
    shortcut.contexts.includes(context) || shortcut.contexts.includes('global'));

  for (const shortcut of shortcuts) {
    for (const binding of shortcut.defaultBindings) {
      if (!isBindingAvailableOnPlatform(binding, platform)) continue;
      for (const signature of getBindingSignatures(binding, platform)) {
        const actions = actionsBySignature.get(signature) ?? [];
        if (!actions.includes(shortcut.id)) actions.push(shortcut.id);
        actionsBySignature.set(signature, actions);
      }
    }
  }

  return Array.from(actionsBySignature.entries())
    .filter(([, actions]) => actions.length > 1)
    .map(([signature, actions]) => ({ signature, actions }));
}

export function getBindingLabel(
  binding: ShortcutBinding,
  platform: ShortcutPlatform = DEFAULT_PLATFORM,
): string {
  if (typeof binding.label === 'string') return binding.label;
  return binding.label[platform] ?? binding.label.default ?? '';
}

export function getBindingSignature(
  binding: ShortcutBinding,
  platform: ShortcutPlatform = DEFAULT_PLATFORM,
): string {
  return getBindingSignatures(binding, platform)[0];
}

function getBindingSignatures(
  binding: ShortcutBinding,
  platform: ShortcutPlatform = DEFAULT_PLATFORM,
): string[] {
  const modifiers = (binding.modifiers ?? []).map((modifier) =>
    modifier === 'cmdOrCtrl' ? (platform === 'mac' ? 'meta' : 'ctrl') : modifier,
  );
  modifiers.sort();
  const physicalKey = getPhysicalKey(binding);
  const baseSignature = [...modifiers, physicalKey].join('+');

  if (!binding.allowShift || modifiers.includes('shift')) {
    return [baseSignature];
  }

  const shiftedModifiers = [...modifiers, 'shift'];
  shiftedModifiers.sort();
  return [baseSignature, [...shiftedModifiers, physicalKey].join('+')];
}

function getPhysicalKey(binding: ShortcutBinding): string {
  if (binding.code) return binding.code;
  const key = binding.key ?? '';
  if (key === ' ' || key === 'Space') return 'Space';
  if (/^[0-9]$/.test(key)) return `Digit${key}`;
  if (/^[a-z]$/i.test(key)) return `Key${key.toUpperCase()}`;
  return key;
}

function matchesBinding(
  binding: ShortcutBinding,
  event: ShortcutEventLike,
  platform: ShortcutPlatform,
): boolean {
  if (!isBindingAvailableOnPlatform(binding, platform)) return false;
  if (binding.key && event.key !== binding.key) return false;
  if (binding.code && event.code !== binding.code) return false;

  const modifiers = new Set(binding.modifiers ?? []);
  const expectsCtrl = modifiers.has('ctrl') || (modifiers.has('cmdOrCtrl') && platform !== 'mac');
  const expectsMeta = modifiers.has('meta') || (modifiers.has('cmdOrCtrl') && platform === 'mac');

  return Boolean(event.ctrlKey) === expectsCtrl
    && Boolean(event.metaKey) === expectsMeta
    && Boolean(event.altKey) === modifiers.has('alt')
    && (binding.allowShift && !modifiers.has('shift')
      ? true
      : Boolean(event.shiftKey) === modifiers.has('shift'));
}

function isBindingAvailableOnPlatform(binding: ShortcutBinding, platform: ShortcutPlatform): boolean {
  return !binding.platforms || binding.platforms.includes(platform);
}
