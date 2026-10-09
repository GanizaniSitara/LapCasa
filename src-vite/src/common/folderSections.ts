// Folders view: one flat, always-open list of every folder that directly
// holds pictures (no tree), and a grid that scrolls through all of them in the same
// order, a section per folder. The sidebar (AlbumList) shows this list; Content.vue
// loads the grid with QueryParams.foldersAll and reads folder dates / ids from here.
import { reactive } from 'vue';
import { config, libConfig } from '@/common/config';
import { getFolderSections } from '@/common/api';

export type FolderSection = {
  folderId: number;
  albumId: number;
  path: string;
  name: string;
  count: number;
  size: number;
  dateMin: number | null;
  dateMax: number | null;
  isFavorite: boolean;
};

// Folders in disk order: each top-level folder, then its subfolders, as on disk
// (backend folder_sort 4). Chosen over a by-date order.
export const FOLDER_VIEW_SORT = 4;

export const folderSections = reactive({
  list: [] as FolderSection[],
  loaded: false,
  loading: false,
});

let byPath = new Map<string, FolderSection>();
let loadedKey = '';
let inflight: Promise<FolderSection[]> | null = null;
let inflightKey = '';

/// Full QueryParams for the view (the backend needs every field).
export function folderViewQueryParams(overrides: Record<string, any> = {}) {
  return {
    searchFileName: '',
    searchFileType: Number(config.search.fileType || 0),
    sortType: Number(config.search.sortType || 0),
    sortOrder: Number(config.search.sortOrder || 0),
    randomSeed: 0,
    searchAllSubfolders: '',
    searchFolder: '',
    startDate: 0,
    endDate: 0,
    calendarSort: Number(config.settings.calendarSort || 0),
    folderSort: FOLDER_VIEW_SORT,
    categorySort: Number(config.settings.categorySort || 0),
    make: '',
    model: '',
    lensMake: '',
    lensModel: '',
    locationAdmin1: '',
    locationName: '',
    isFavorite: false,
    rating: -1,
    cullingFlag: -1,
    tagId: 0,
    tagGroupId: 0,
    personId: 0,
    foldersAll: true,
    ...overrides,
  };
}

function currentKey() {
  return `${libConfig._libraryId}|${Number(config.search.fileType || 0)}`;
}

/// Load the list (once per library / file-type filter unless forced).
export function loadFolderSections(force = false): Promise<FolderSection[]> {
  const key = currentKey();
  if (!force && folderSections.loaded && loadedKey === key) return Promise.resolve(folderSections.list);
  if (inflight && inflightKey === key && !force) return inflight;
  folderSections.loading = true;
  inflightKey = key;
  const request = (getFolderSections(folderViewQueryParams()) as Promise<FolderSection[] | null>)
    .then((list) => {
      if (request !== inflight) return folderSections.list;
      const sections = Array.isArray(list) ? list : [];
      byPath = new Map(sections.map(section => [section.path, section]));
      folderSections.list = sections;
      folderSections.loaded = Array.isArray(list);
      loadedKey = key;
      return sections;
    })
    .finally(() => {
      if (request === inflight) {
        inflight = null;
        folderSections.loading = false;
      }
    });
  inflight = request;
  return request;
}

export function folderSectionOf(path: string | null | undefined) {
  void folderSections.list; // reactive dependency: callers re-run when the list reloads
  return path ? byPath.get(path) : undefined;
}
