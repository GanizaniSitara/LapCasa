import type { Geometry } from './layout';

export type SectionHeader = { index: number; height: number; rowHeight?: number };
export type IndexedGrid = {
  length: number;
  contentSize: number;
  boxAt(index: number): Geometry | undefined;
  range(start: number, end: number): { start: number; end: number };
  displayIndex(fileIndex: number): number;
  fileIndex(displayIndex: number): number;
};

// Headers are indexed once by the query. Resizing visits sections, never photos.
export function sectionGrid(
  length: number, headers: SectionHeader[], columns: number,
  width: number, itemWidth: number, itemHeight: number,
): IndexedGrid {
  const sections: Array<{ start: number; end: number; y: number; header: number; rowHeight: number; filesBefore: number }> = [];
  let y = 0;
  let filesBefore = 0;
  const add = (start: number, end: number, header: number, rowHeight = itemHeight) => {
    sections.push({ start, end, y, header, rowHeight, filesBefore });
    const count = end - start - (header > 0 ? 1 : 0);
    y += header + Math.ceil(count / columns) * rowHeight;
    filesBefore += count;
  };
  if (headers.length === 0) add(0, length, 0);
  else {
    if (headers[0].index > 0) add(0, headers[0].index, 0);
    headers.forEach((header, i) => add(header.index, headers[i + 1]?.index ?? length, header.height, header.rowHeight));
  }
  function sectionAt(index: number) {
    let low = 0, high = sections.length;
    while (low < high) {
      const mid = (low + high) >>> 1;
      if (sections[mid].end <= index) low = mid + 1;
      else high = mid;
    }
    return sections[low];
  }
  function boxAt(index: number): Geometry | undefined {
    if (index < 0 || index >= length) return;
    const section = sectionAt(index);
    if (!section) return;
    if (section.header > 0 && index === section.start) {
      return { x: 0, y: section.y, width, height: section.header };
    }
    const offset = index - section.start - (section.header > 0 ? 1 : 0);
    return { x: (offset % columns) * itemWidth,
      y: section.y + section.header + Math.floor(offset / columns) * section.rowHeight,
      width: itemWidth, height: section.rowHeight };
  }
  function bound(position: number, byEnd: boolean) {
    let low = 0, high = length;
    while (low < high) {
      const mid = (low + high) >>> 1;
      const box = boxAt(mid)!;
      const value = box.y + (byEnd ? box.height : 0);
      if (byEnd ? value < position : value <= position) low = mid + 1;
      else high = mid;
    }
    return low;
  }
  return {
    length, contentSize: y, boxAt,
    range: (start, end) => ({ start: bound(start, true), end: bound(end, false) }),
    displayIndex(fileIndex) {
      let low = 0, high = sections.length;
      while (low < high) {
        const mid = (low + high) >>> 1;
        const s = sections[mid];
        const count = s.end - s.start - (s.header > 0 ? 1 : 0);
        if (s.filesBefore + count <= fileIndex) low = mid + 1;
        else high = mid;
      }
      const s = sections[low];
      return s ? s.start + (s.header > 0 ? 1 : 0) + fileIndex - s.filesBefore : -1;
    },
    fileIndex(index) {
      const s = sectionAt(index);
      if (!s || index < 0 || index >= length || (s.header > 0 && index === s.start)) return -1;
      return s.filesBefore + index - s.start - (s.header > 0 ? 1 : 0);
    },
  };
}
