// Search keeps relevance order, so date jumps use actual result positions.
export function searchTimeline(files: any[]) {
  const seen = new Set<number>();
  const dated = files.flatMap((file, position) => {
    const id = Number(file.id);
    const time = Number(file.taken_date || file.created_at || file.modified_at || 0);
    const date = new Date(time * 1000);
    if (!id || seen.has(id) || !(time > 0) || !Number.isFinite(date.getTime())
      || date.getFullYear() < 1970 || date.getFullYear() > 2100) return [];
    seen.add(id);
    return [{ time, date, position }];
  });
  let min = Infinity, max = -Infinity;
  for (const file of dated) { min = Math.min(min, file.time); max = Math.max(max, file.time); }
  const days = Math.floor((max - min) / 86400);
  const unit = days > 730 ? 'month' : days > 62 ? 'week' : 'day';
  const buckets = new Map<string, any>();
  const markers = new Map<string, any>();
  for (const { date, position } of dated) {
    const dayKey = `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
    if (!markers.has(dayKey)) markers.set(dayKey, {
      year: date.getFullYear(), month: date.getMonth() + 1, date: date.getDate(), position,
    });
    const start = new Date(date.getFullYear(), date.getMonth(), date.getDate());
    if (unit === 'month') start.setDate(1);
    else if (unit === 'week') start.setDate(start.getDate() - (start.getDay() + 6) % 7);
    const key = `${start.getFullYear()}-${start.getMonth()}-${start.getDate()}`;
    const bucket = buckets.get(key);
    if (bucket) bucket.count++;
    else buckets.set(key, {
      year: start.getFullYear(), month: start.getMonth() + 1, day: start.getDate(), unit, count: 1,
    });
  }
  return {
    buckets: [...buckets.values()].sort((a, b) => a.year - b.year || a.month - b.month || a.day - b.day),
    markers: [...markers.values()],
  };
}
