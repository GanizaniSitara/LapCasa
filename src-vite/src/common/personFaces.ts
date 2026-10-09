// Person view: face crops, "?" suggestions with tick / x, learning
// after each tick, undo of an accidental x, and the person's ignored faces.
// Content.vue loads the people shown; Thumbnail.vue crops and shows the controls.
// Confirmed faces regroup; ignored faces briefly dim with Undo, then leave review.
//
// State is kept per person (people: personId -> state), so the continuous People
// view can show many people at once. A tile there carries `face_person_id`, the
// section's person; elsewhere tiles use personFaces.personId, the current person.
import { reactive } from 'vue';
import { useConfigStore } from '@/stores/configStore';
import {
  getPersonFaceBoxes, confirmFaceSuggestions, rejectFaceSuggestions,
  undoRejectFaceSuggestions, confirmIgnoredFaces, getPersonIgnoredFaces, refreshPersonSuggestions,
  excludeFacesFromPerson,
  restoreGloballyIgnoredFaces,
} from '@/common/api';

type Box = { x: number; y: number; width: number; height: number; coordinate_space?: 'raw' | 'display' };

type PersonState = {
  boxes: Map<number, Box>;
  // Pending "?" suggestions: fileId -> suggested faceId.
  suggestions: Map<number, number>;
  // Faces ignored with "x" for this person: fileId -> faceId.
  ignored: Map<number, number>;
  // Files ignored this session: their tiles stay, dimmed, with Undo.
  sessionIgnored: Set<number>;
  // Confirmed photos (for the section header and the sidebar count).
  confirmed: number;
  // Confirmed faces: fileId -> faceId (for right-click "Not this person").
  confirmedFaces: Map<number, number>;
  // Files taken off the person with "Not this person" this session: Undo re-confirms them.
  excluded: Set<number>;
};

const EMPTY_STATE: PersonState = Object.freeze({
  boxes: new Map(),
  suggestions: new Map(),
  ignored: new Map(),
  sessionIgnored: new Set(),
  confirmed: 0,
  confirmedFaces: new Map(),
  excluded: new Set(),
}) as PersonState;

export const personFaces = reactive({
  // The person the toolbar acts on: the single-person view's person, or the section
  // at the top of the continuous People view.
  personId: 0,
  people: new Map<number, PersonState>(),
  // When the session (a person view or the People view) was opened (unix s): the list
  // keeps faces ignored since then.
  sessionSince: 0,
  // Suggestions button: narrow the person view to pending suggestions only.
  // (Normally they follow the confirmed photos, each with a tick / x under it.)
  reviewMode: false,
  // Ignored button: list the person's ignored faces, each with a tick to restore.
  ignoredMode: false,
  // The last "x", for the toolbar Undo.
  lastIgnored: null as null | { personId: number; fileIds: number[] },
  // The grid reserves a tick / x row under every tile once any suggestion is shown;
  // it stays for the session so rows never change height mid-review.
  controlsRow: false,
  // The current person's state (read-only views used by the toolbar).
  get boxes(): Map<number, Box> { return stateOf(this.personId).boxes; },
  get suggestions(): Map<number, number> { return stateOf(this.personId).suggestions; },
  get ignored(): Map<number, number> { return stateOf(this.personId).ignored; },
  get sessionIgnored(): Set<number> { return stateOf(this.personId).sessionIgnored; },
});

export function stateOf(personId: number | null | undefined): PersonState {
  return (personId && personFaces.people.get(personId)) || EMPTY_STATE;
}

// Same rule as the People sidebar's "Named People" section.
export function isNamedPersonName(name: string | null | undefined): boolean {
  return !!name && !!name.trim() && !name.trim().startsWith('Person ');
}

function resolvePerson(personId?: number | null): number {
  return Number(personId ?? personFaces.personId ?? 0);
}

const loadTokens = new Map<number, number>();
const loading = new Map<number, Promise<void>>();

function parseBox(json: string): Box | undefined {
  try {
    const b = JSON.parse(json);
    return b.width > 0 && b.height > 0 ? b : undefined;
  } catch {
    return undefined;
  }
}

async function loadPersonState(personId: number) {
  const token = (loadTokens.get(personId) || 0) + 1;
  loadTokens.set(personId, token);
  const [rows, ignoredRows] = await Promise.all([
    (personId === -1 ? Promise.resolve([]) : getPersonFaceBoxes(personId)) as Promise<[number, string, number, boolean][] | null>,
    getPersonIgnoredFaces(personId) as Promise<[number, number, string][] | null>,
  ]);
  if (loadTokens.get(personId) !== token) return;
  const boxes = new Map<number, Box>();
  const suggestions = new Map<number, number>();
  const ignored = new Map<number, number>();
  let confirmed = 0;
  const confirmedFaces = new Map<number, number>();
  for (const [fileId, json, faceId, suggested] of rows || []) {
    if (suggested) suggestions.set(fileId, faceId);
    else {
      confirmed++;
      confirmedFaces.set(fileId, faceId);
    }
    const b = parseBox(json);
    if (b) boxes.set(fileId, b);
  }
  for (const [fileId, faceId, json] of ignoredRows || []) {
    ignored.set(fileId, faceId);
    const b = parseBox(json);
    if (b && !boxes.has(fileId)) boxes.set(fileId, b);
  }
  const previous = personFaces.people.get(personId);
  const sessionIgnored = previous?.sessionIgnored ?? new Set<number>();
  const excluded = previous?.excluded ?? new Set<number>();
  personFaces.people.set(personId, { boxes, suggestions, ignored, sessionIgnored, confirmed, confirmedFaces, excluded });
  if (suggestions.size > 0 || confirmedFaces.size > 0) personFaces.controlsRow = true;
}

// A new session: the People view (or a person view) was opened from elsewhere.
export function startPeopleSession() {
  for (const timer of ignoreExpiryTimers.values()) clearTimeout(timer);
  ignoreExpiryTimers.clear();
  personFaces.sessionSince = Math.floor(Date.now() / 1000);
  personFaces.reviewMode = false;
  personFaces.ignoredMode = false;
  personFaces.lastIgnored = null;
  personFaces.controlsRow = false;
  for (const state of personFaces.people.values()) {
    state.sessionIgnored = new Set();
    state.excluded = new Set();
  }
}

// Load a section's person once, for the continuous People view.
export function ensurePersonFaces(personId: number) {
  if (!personId || personId <= 0 || personFaces.people.has(personId) || loading.has(personId)) return;
  const promise = loadPersonState(personId).finally(() => loading.delete(personId));
  loading.set(personId, promise);
}

// Make personId the current person. keepSession: moving within the People view
// (scrolling or a sidebar click), so modes and dimmed tiles stay as they are.
export async function loadPersonFaces(personId: number | null, keepSession = false) {
  const id = Number(personId || 0);
  const switching = id !== personFaces.personId;
  const opening = id > 0 && switching && !keepSession;
  if (switching && !keepSession) {
    startPeopleSession();
    personFaces.people = new Map();
  }
  personFaces.personId = id > 0 || id === -1 ? id : 0;
  if (id === -1) {
    await loadPersonState(id);
    personFaces.controlsRow = true;
    return;
  }
  if (id <= 0) return;
  if (keepSession && personFaces.people.has(id)) return;
  await loadPersonState(id);
  if (personFaces.personId === id && stateOf(id).ignored.size === 0) personFaces.ignoredMode = false;
  // Keep learning in the background: the person's confirmed faces may now
  // match faces that were not suggested before.
  if (opening) void learnInBackground(id);
}

async function learnInBackground(personId: number) {
  const files: number[] = ((await refreshPersonSuggestions(personId)) as number[] | null) || [];
  if (files.length > 0 && personFaces.people.has(personId)) {
    await loadPersonState(personId);
    // The person view appends these at the end of that person's photos; nothing else moves.
    announce(personId, { appended: files.map(Number) });
  }
}

export function suggestedFaceId(fileId: number, personId?: number | null): number | undefined {
  return stateOf(resolvePerson(personId)).suggestions.get(fileId);
}

export function ignoredFaceId(fileId: number, personId?: number | null): number | undefined {
  return personFaces.ignoredMode ? stateOf(resolvePerson(personId)).ignored.get(fileId) : undefined;
}

export function isSessionIgnored(fileId: number, personId?: number | null): boolean {
  return !personFaces.ignoredMode && stateOf(resolvePerson(personId)).sessionIgnored.has(fileId);
}

export function hasFaceBox(fileId: number, personId?: number | null): boolean {
  return stateOf(resolvePerson(personId)).boxes.has(fileId);
}

// Tell the People sidebar (badge, count) and the person view what changed.
// appended = file ids that gained a suggestion (the view adds them at the end);
// reload = a bulk change (Confirm all, Remove, restore from the ignored list).
function announce(personId: number, change: { appended?: number[]; reload?: boolean; restack?: boolean } = {}) {
  const state = stateOf(personId);
  window.dispatchEvent(new CustomEvent('lapcasa-face-suggestions-changed', {
    detail: {
      personId,
      appended: change.appended ?? [],
      reload: !!change.reload,
      restack: !!change.restack,
      suggestions: state.suggestions.size,
      count: state.confirmed,
    },
  }));
}

const ignoreExpiryTimers = new Map<string, ReturnType<typeof setTimeout>>();
function expireIgnoredFaces(personId: number, fileIds: number[]) {
  for (const fileId of fileIds) {
    const key = `${personId}:${fileId}`;
    const previous = ignoreExpiryTimers.get(key);
    if (previous) clearTimeout(previous);
    ignoreExpiryTimers.set(key, setTimeout(() => {
      ignoreExpiryTimers.delete(key);
      const state = personFaces.people.get(personId);
      if (!state?.sessionIgnored.has(fileId)) return; // already undone or a new session
      state.sessionIgnored = new Set([...state.sessionIgnored].filter(id => id !== fileId));
      state.excluded = new Set([...state.excluded].filter(id => id !== fileId));
      const last = personFaces.lastIgnored;
      if (last?.personId === personId) {
        const remaining = last.fileIds.filter(id => id !== fileId);
        personFaces.lastIgnored = remaining.length ? { personId, fileIds: remaining } : null;
      }
      announce(personId, { restack: true });
    }, 5_000));
  }
}

// Apply a confirm / ignore to the local state at once, so the tiles react immediately.
function applyLocally(personId: number, confirm: boolean, fileIds: number[]) {
  const state = personFaces.people.get(personId);
  if (!state) return;
  const next = new Map(state.suggestions);
  const ignored = new Map(state.ignored);
  const sessionIgnored = new Set(state.sessionIgnored);
  const confirmedFaces = new Map(state.confirmedFaces);
  let confirmed = state.confirmed;
  for (const fileId of fileIds) {
    if (!next.has(fileId)) continue;
    if (confirm) {
      confirmed++;
      confirmedFaces.set(fileId, next.get(fileId)!);
    } else {
      ignored.set(fileId, next.get(fileId)!);
      sessionIgnored.add(fileId);
    }
    next.delete(fileId);
  }
  state.confirmed = confirmed;
  state.confirmedFaces = confirmedFaces;
  state.suggestions = next;
  state.ignored = ignored;
  state.sessionIgnored = sessionIgnored;
  announce(personId);
}

// Learning runs shortly after the last tick (suggestions keep coming as you confirm),
// so a burst of ticks costs one re-match per person, not one per tick.
const learnTimers = new Map<number, ReturnType<typeof setTimeout>>();
function scheduleLearning(personId: number) {
  const pending = learnTimers.get(personId);
  if (pending) clearTimeout(pending);
  learnTimers.set(personId, setTimeout(() => {
    learnTimers.delete(personId);
    void learnInBackground(personId);
  }, 1200));
}

// Confirm (assign to the person) or ignore suggested faces;
// fileIds = null applies to every pending suggestion of that person.
export async function resolveSuggestions(confirm: boolean, fileIds: number[] | null = null, forPerson?: number | null) {
  const personId = resolvePerson(forPerson);
  const state = personFaces.people.get(personId);
  if (!personId || !state) return;
  const allFiles = fileIds === null;
  const files = (fileIds ?? [...state.suggestions.keys()]).filter(id => state.suggestions.has(id));
  const faceIds = files.map(id => state.suggestions.get(id)!);
  if (faceIds.length === 0) return;
  const previousSessionIgnored = state.sessionIgnored;
  const previousLastIgnored = personFaces.lastIgnored;
  if (!confirm) personFaces.lastIgnored = { personId, fileIds: files };
  // Optimistic: the clicked tiles update before the database answers.
  if (!allFiles) applyLocally(personId, confirm, files);
  const done: any = confirm
    ? await confirmFaceSuggestions(personId, allFiles ? null : faceIds, useConfigStore().settings.face?.confirmCascadeThreshold ?? 0.80)
    : await rejectFaceSuggestions(personId, allFiles ? null : faceIds);
  if (done === null) {
    state.sessionIgnored = previousSessionIgnored;
    personFaces.lastIgnored = previousLastIgnored;
    await loadPersonState(personId); // the write failed: show the real state again
    announce(personId, { reload: true });
    return false;
  }
  if (!personFaces.people.has(personId)) return;
  if (allFiles) {
    applyLocally(personId, confirm, files);
  } else if (confirm && Array.isArray(done?.files)) {
    // Cascade: near-identical suggestions were confirmed with it; they lose their "?".
    const current = stateOf(personId).suggestions;
    const extra = done.files.map(Number).filter((id: number) => current.has(id));
    if (extra.length) applyLocally(personId, true, extra);
  }
  if (confirm) {
    announce(personId, { restack: true });
    scheduleLearning(personId);
  } else {
    expireIgnoredFaces(personId, files);
  }
  return true;
}

// Right-click "Not this person" on confirmed photos: the face leaves the person
// and is never suggested for them again; the tile stays in place, dimmed, with Undo.
export function isConfirmedFor(fileId: number, forPerson?: number | null): boolean {
  return stateOf(resolvePerson(forPerson)).confirmedFaces.has(fileId);
}

export async function excludeFromPerson(fileIds: number[], forPerson?: number | null) {
  const personId = resolvePerson(forPerson);
  const state = personFaces.people.get(personId);
  if (!personId || !state) return;
  const files = fileIds.filter(id => state.confirmedFaces.has(id));
  if (files.length === 0) return;
  const faceIds = files.map(id => state.confirmedFaces.get(id)!);
  const previousSessionIgnored = state.sessionIgnored;
  const previousExcluded = state.excluded;
  const previousLastIgnored = personFaces.lastIgnored;
  const confirmedFaces = new Map(state.confirmedFaces);
  const ignored = new Map(state.ignored);
  const sessionIgnored = new Set(state.sessionIgnored);
  const excluded = new Set(state.excluded);
  files.forEach((id, i) => {
    confirmedFaces.delete(id);
    ignored.set(id, faceIds[i]);
    sessionIgnored.add(id);
    excluded.add(id);
  });
  state.confirmedFaces = confirmedFaces;
  state.ignored = ignored;
  state.sessionIgnored = sessionIgnored;
  state.excluded = excluded;
  state.confirmed = Math.max(0, state.confirmed - files.length);
  personFaces.lastIgnored = { personId, fileIds: files };
  announce(personId);
  const done = await excludeFacesFromPerson(personId, faceIds);
  if (done === null) {
    state.sessionIgnored = previousSessionIgnored;
    state.excluded = previousExcluded;
    personFaces.lastIgnored = previousLastIgnored;
    await loadPersonState(personId); // the write failed: show the real state again
    announce(personId, { reload: true });
    return false;
  }
  expireIgnoredFaces(personId, files);
  return true;
}

// One selection can contain both suggested and already confirmed faces.
export async function excludePersonFiles(fileIds: number[], personId: number) {
  const state = personFaces.people.get(personId);
  if (!state) return;
  const files = [...new Set(fileIds)];
  const suggested = files.filter(id => state.suggestions.has(id));
  const confirmed = files.filter(id => state.confirmedFaces.has(id));
  const ignored: number[] = [];
  if (suggested.length && await resolveSuggestions(false, suggested, personId)) ignored.push(...suggested);
  if (confirmed.length && await excludeFromPerson(confirmed, personId)) ignored.push(...confirmed);
  if (ignored.length) personFaces.lastIgnored = { personId, fileIds: ignored };
}

// Undo an "x" on these files (the tile's own Undo, or the toolbar Undo for the last x).
// A face taken off the person with "Not this person" goes back to confirmed instead.
export async function undoIgnore(fileIds: number[], forPerson?: number | null) {
  const personId = resolvePerson(forPerson);
  const state = personFaces.people.get(personId);
  if (!personId || !state) return;
  const reconfirm = fileIds.filter(id => state.excluded.has(id) && state.ignored.has(id));
  if (reconfirm.length) {
    const faceIds = reconfirm.map(id => state.ignored.get(id)!);
    const confirmedFaces = new Map(state.confirmedFaces);
    const ignored = new Map(state.ignored);
    const sessionIgnored = new Set(state.sessionIgnored);
    const excluded = new Set(state.excluded);
    reconfirm.forEach((id, i) => {
      confirmedFaces.set(id, faceIds[i]);
      ignored.delete(id);
      sessionIgnored.delete(id);
      excluded.delete(id);
    });
    state.confirmedFaces = confirmedFaces;
    state.ignored = ignored;
    state.sessionIgnored = sessionIgnored;
    state.excluded = excluded;
    state.confirmed += reconfirm.length;
    if (personFaces.lastIgnored?.personId === personId) personFaces.lastIgnored = null;
    announce(personId);
    await confirmIgnoredFaces(personId, faceIds);
  }
  const files = fileIds.filter(id => state.ignored.has(id) && !reconfirm.includes(id));
  if (files.length === 0) return;
  const faceIds = files.map(id => state.ignored.get(id)!);
  // Optimistic: back to a pending "?" in place.
  const suggestions = new Map(state.suggestions);
  const ignored = new Map(state.ignored);
  const sessionIgnored = new Set(state.sessionIgnored);
  files.forEach((id, i) => {
    suggestions.set(id, faceIds[i]);
    ignored.delete(id);
    sessionIgnored.delete(id);
  });
  state.suggestions = suggestions;
  state.ignored = ignored;
  state.sessionIgnored = sessionIgnored;
  const last = personFaces.lastIgnored;
  if (last && last.personId === personId && last.fileIds.every(id => !ignored.has(id))) {
    personFaces.lastIgnored = null;
  }
  announce(personId);
  await undoRejectFaceSuggestions(personId, faceIds);
}

export async function undoLastIgnore() {
  const last = personFaces.lastIgnored;
  if (!last) return;
  await undoIgnore(last.fileIds, last.personId);
  personFaces.lastIgnored = null;
}

// From the ignored list: it is this person after all.
export async function confirmIgnored(fileIds: number[], forPerson?: number | null) {
  const personId = resolvePerson(forPerson);
  const state = stateOf(personId);
  const faceIds = fileIds.map(id => state.ignored.get(id)).filter((id): id is number => id !== undefined);
  if (!personId || faceIds.length === 0) return;
  const done = await confirmIgnoredFaces(personId, faceIds);
  if (done === null) return;
  await loadPersonState(personId);
  announce(personId, { reload: true });
}

export async function restoreGlobalIgnore(fileIds: number[]) {
  const state = stateOf(-1);
  const faceIds = fileIds.map(id => state.ignored.get(id)).filter((id): id is number => id !== undefined);
  if (!faceIds.length || await restoreGloballyIgnoredFaces(faceIds) === null) return;
  await loadPersonState(-1);
  // Other cached person sections may now contain a restored face.
  for (const id of personFaces.people.keys()) if (id !== -1) personFaces.people.delete(id);
  window.dispatchEvent(new CustomEvent('lapcasa-global-ignores-changed'));
  announce(-1, { reload: true });
}

// Position the photo inside the clipped tile; this works across WebView versions.
// People faces are framed as portrait tiles (width:height 5:6), hair to neck.
export const FACE_TILE_ASPECT = 5 / 6;

export function faceCropStyle(
  fileId: number, width?: number, height?: number, aspect = FACE_TILE_ASPECT, personId?: number | null,
) {
  const b = stateOf(resolvePerson(personId)).boxes.get(fileId);
  if (!b || !width || !height) return undefined;
  let h = Math.max(b.height * 1.1, b.width * 1.1 / aspect);
  let w = h * aspect;
  const fit = Math.min(1, width / w, height / h);
  w *= fit;
  h *= fit;
  const cx = b.x + b.width / 2;
  const cy = b.y + b.height * 0.5;
  const left = Math.min(Math.max(cx - w / 2, 0), width - w);
  const top = Math.min(Math.max(cy - h / 2, 0), height - h);
  return {
    position: 'absolute' as const,
    width: `${(width / w * 100).toFixed(3)}%`,
    height: `${(height / h * 100).toFixed(3)}%`,
    left: `${(-left / w * 100).toFixed(3)}%`,
    top: `${(-top / h * 100).toFixed(3)}%`,
    maxWidth: 'none',
    maxHeight: 'none',
    objectFit: 'fill' as const,
    transform: 'none',
  };
}
