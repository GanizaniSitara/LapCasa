// Keep likely next-folder thumbnails decoded under their original URLs. Copying
// hundreds of responses into blob URLs blocks the UI thread during navigation.
import { shallowRef } from 'vue';

// Content owns the query and its row/image caches. Warming through that same
// path keeps filtering and sorting identical to the screen that will be shown.
export const hoveredFolder = shallowRef('');

export function prefetchOnHover(path: string) {
  hoveredFolder.value = path;
}

export function cancelHoverPrefetch() {
  hoveredFolder.value = '';
}
