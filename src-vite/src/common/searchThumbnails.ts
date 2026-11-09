// Keep a bounded, lazy set of decoded search images so a result batch can paint
// together. A slow or broken neighbour cannot hold the batch indefinitely.
import { getLapUrl, imageEditState } from '@/common/utils';

const decoded = new Map<string, HTMLImageElement>();
const pendingImages = new Set<HTMLImageElement>();
let idleTimer: ReturnType<typeof setTimeout> | undefined;

export function decodedSearchThumbnail(source: string) {
  if (!source) return undefined;
  const key = new URL(source, document.baseURI).href;
  const image = decoded.get(key);
  if (image?.src !== key) {
    decoded.delete(key);
    return undefined;
  }
  if (!image.complete || image.naturalWidth <= 0) return undefined;
  rememberSearchThumbnail(key, image);
  return image;
}

export function rememberSearchThumbnail(source: string, image: HTMLImageElement) {
  if (!source || !image.complete || !image.naturalWidth) return;
  const key = new URL(source, document.baseURI).href;
  decoded.delete(key);
  decoded.set(key, image);
  while (decoded.size > 160) decoded.delete(decoded.keys().next().value!);
  if (idleTimer) clearTimeout(idleTimer);
  idleTimer = setTimeout(() => decoded.clear(), 600000);
}

export function cancelSearchThumbnailPreparation() {
  for (const image of pendingImages) image.src = '';
  pendingImages.clear();
}

export function searchThumbnailSource(file: any, size: number) {
  return imageEditState(file.edits).revision || !file.thumbnail
    ? (Number(file.id) > 0 ? getLapUrl(Number(file.id), false, size, Number(file.modified_at || 0), file.edits) : '')
    : String(file.thumbnail);
}

export async function prepareSearchThumbnails(files: any[], count: number, size: number) {
  const pending = files.slice(0, Math.min(128, count)).map(async file => {
    const source = searchThumbnailSource(file, size);
    if (!source || decodedSearchThumbnail(source)) return;
    const image = new Image();
    pendingImages.add(image);
    image.src = source;
    try {
      await image.decode();
      rememberSearchThumbnail(source, image);
    } catch { /* The existing tile retry handles unavailable images. */ }
    finally { pendingImages.delete(image); }
  });
  let timeout: ReturnType<typeof setTimeout> | undefined;
  await Promise.race([
    Promise.all(pending),
    new Promise<void>(resolve => { timeout = setTimeout(resolve, 96); }),
  ]);
  if (timeout) clearTimeout(timeout);
}
