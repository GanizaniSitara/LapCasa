/**
 * In-memory columnar attribute projections for instant timeline scrubbing,
 * date jumping, and star/tag filtering (Picasa DB3 / .pmp style architecture).
 * project: Lap
 * author:  lap-040
 * date:    2026-10-13
 */

use once_cell::sync::Lazy;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::Instant;

use crate::t_sqlite::{ATimeLine, QueryParams, SmartQueryParams};

/// Precomputed calendar date components (civil calendar)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CivilDate {
    pub year: i32,
    pub month: i32,
    pub day: i32,
}

/// Fast civil calendar calculation from unix epoch seconds (Howard Hinnant algorithm).
/// Operates entirely with basic integer arithmetic and no external allocation.
#[inline(always)]
pub fn timestamp_to_civil(ts: i64) -> CivilDate {
    if ts <= 0 {
        return CivilDate {
            year: 1970,
            month: 1,
            day: 1,
        };
    }
    let z = ts / 86400 + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    CivilDate {
        year: y as i32,
        month: m as i32,
        day: d as i32,
    }
}

/// Packs year, month, and day into a single 32-bit integer for vectorized scans.
#[inline(always)]
pub fn pack_ymd(year: i32, month: i32, day: i32) -> u32 {
    let y = (year.clamp(1900, 2200) - 1900) as u32;
    (y << 16) | (((month as u32) & 0xFF) << 8) | ((day as u32) & 0xFF)
}

/// Unpacks a packed 32-bit date into (year, month, day).
#[inline(always)]
pub fn unpack_ymd(packed: u32) -> CivilDate {
    let year = ((packed >> 16) & 0xFFFF) as i32 + 1900;
    let month = ((packed >> 8) & 0xFF) as i32;
    let day = (packed & 0xFF) as i32;
    CivilDate { year, month, day }
}

/// A timeline histogram bucket representing photo frequency and starting scroll position.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TimelineHistogramBucket {
    pub year: i32,
    pub month: i32,
    pub date: Option<i32>,
    pub count: u32,
    pub position: i64,
}

/// An in-memory columnar projection table caching fixed-stride attributes
/// across large image collections (Picasa DB3 .pmp style).
#[derive(Debug, Clone, Default)]
pub struct ColumnarTable {
    // Contiguous fixed-stride binary column vectors
    pub file_ids: Vec<i64>,
    pub timestamps: Vec<i64>,      // taken_date (or created_at fallback)
    pub created_ats: Vec<i64>,     // file created timestamp
    pub modified_ats: Vec<i64>,    // file modified timestamp
    pub star_ratings: Vec<i16>,    // 0..5 rating
    pub is_favorites: Vec<u8>,     // 0 or 1
    pub rotations: Vec<i16>,       // 0, 90, 180, 270 degrees
    pub culling_flags: Vec<i8>,    // 0=unreviewed, 1=pick, 2=reject
    pub is_hiddens: Vec<u8>,
    pub folder_ids: Vec<i64>,      // folder identifier

    // Pre-extracted civil calendar date (packed u32: (year << 16) | (month << 8) | date)
    pub packed_dates: Vec<u32>,

    // Inverted tag index: tag_id -> sorted row indices
    pub tag_to_indices: HashMap<i64, Vec<usize>>,
    // Per-row tag list
    pub row_tags: Vec<Vec<i64>>,

    // O(1) primary key index mapping file_id -> row index
    pub id_to_index: HashMap<i64, usize>,
}

impl ColumnarTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            file_ids: Vec::with_capacity(capacity),
            timestamps: Vec::with_capacity(capacity),
            created_ats: Vec::with_capacity(capacity),
            modified_ats: Vec::with_capacity(capacity),
            star_ratings: Vec::with_capacity(capacity),
            is_favorites: Vec::with_capacity(capacity),
            rotations: Vec::with_capacity(capacity),
            culling_flags: Vec::with_capacity(capacity),
            is_hiddens: Vec::with_capacity(capacity),
            folder_ids: Vec::with_capacity(capacity),
            packed_dates: Vec::with_capacity(capacity),
            tag_to_indices: HashMap::new(),
            row_tags: Vec::with_capacity(capacity),
            id_to_index: HashMap::with_capacity(capacity),
        }
    }

    pub fn clear(&mut self) {
        self.file_ids.clear();
        self.timestamps.clear();
        self.created_ats.clear();
        self.modified_ats.clear();
        self.star_ratings.clear();
        self.is_favorites.clear();
        self.rotations.clear();
        self.culling_flags.clear();
        self.is_hiddens.clear();
        self.folder_ids.clear();
        self.packed_dates.clear();
        self.tag_to_indices.clear();
        self.row_tags.clear();
        self.id_to_index.clear();
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.file_ids.len()
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.file_ids.is_empty()
    }

    /// Insert or update a file's columnar attributes.
    pub fn upsert_file(
        &mut self,
        file_id: i64,
        folder_id: i64,
        taken_date: Option<i64>,
        created_at: Option<i64>,
        modified_at: Option<i64>,
        star_rating: Option<i32>,
        is_favorite: Option<bool>,
        rotation: Option<i32>,
        culling_flag: Option<i32>,
        tags: Option<Vec<i64>>,
    ) {
        let ts = taken_date
            .or(created_at)
            .or(modified_at)
            .unwrap_or(0);
        let cr = created_at.unwrap_or(ts);
        let mo = modified_at.unwrap_or(ts);
        let star = star_rating.unwrap_or(0).clamp(0, 5) as i16;
        let fav = if is_favorite.unwrap_or(false) { 1 } else { 0 };
        let rot = rotation.unwrap_or(0) as i16;
        let cull = culling_flag.unwrap_or(0).clamp(0, 2) as i8;

        let civil = timestamp_to_civil(ts);
        let packed = pack_ymd(civil.year, civil.month, civil.day);

        if let Some(&idx) = self.id_to_index.get(&file_id) {
            // Update existing row in place
            self.folder_ids[idx] = folder_id;
            self.timestamps[idx] = ts;
            self.created_ats[idx] = cr;
            self.modified_ats[idx] = mo;
            self.star_ratings[idx] = star;
            self.is_favorites[idx] = fav;
            self.rotations[idx] = rot;
            self.culling_flags[idx] = cull;
            self.packed_dates[idx] = packed;

            if let Some(new_tags) = tags {
                self.set_row_tags(idx, new_tags);
            }
        } else {
            // Append new row
            let idx = self.file_ids.len();
            self.file_ids.push(file_id);
            self.folder_ids.push(folder_id);
            self.timestamps.push(ts);
            self.created_ats.push(cr);
            self.modified_ats.push(mo);
            self.star_ratings.push(star);
            self.is_favorites.push(fav);
            self.rotations.push(rot);
            self.culling_flags.push(cull);
            self.is_hiddens.push(0);
            self.packed_dates.push(packed);
            self.id_to_index.insert(file_id, idx);

            let row_tag_list = tags.unwrap_or_default();
            for &t in &row_tag_list {
                self.tag_to_indices.entry(t).or_default().push(idx);
            }
            self.row_tags.push(row_tag_list);
        }
    }

    /// Delete files by ID from the columnar projections.
    pub fn delete_files(&mut self, file_ids: &[i64]) {
        for &fid in file_ids {
            if let Some(&idx) = self.id_to_index.get(&fid) {
                let last_idx = self.len() - 1;
                let old_tags = self.row_tags[idx].clone();

                // Remove from tag index for this deleted row
                for &t in &old_tags {
                    if let Some(list) = self.tag_to_indices.get_mut(&t) {
                        list.retain(|&i| i != idx);
                    }
                }

                // If not last element, swap_remove moves last element to idx
                self.file_ids.swap_remove(idx);
                self.folder_ids.swap_remove(idx);
                self.timestamps.swap_remove(idx);
                self.created_ats.swap_remove(idx);
                self.modified_ats.swap_remove(idx);
                self.star_ratings.swap_remove(idx);
                self.is_favorites.swap_remove(idx);
                self.rotations.swap_remove(idx);
                self.culling_flags.swap_remove(idx);
                self.is_hiddens.swap_remove(idx);
                self.packed_dates.swap_remove(idx);
                self.row_tags.swap_remove(idx);

                self.id_to_index.remove(&fid);

                // If swapped from last_idx to idx, fixup references to last_idx
                if idx < self.len() {
                    let swapped_id = self.file_ids[idx];
                    self.id_to_index.insert(swapped_id, idx);

                    for &t in &self.row_tags[idx] {
                        if let Some(list) = self.tag_to_indices.get_mut(&t) {
                            for item in list.iter_mut() {
                                if *item == last_idx {
                                    *item = idx;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Batch update metadata for files (favorite, rating, rotation, culling).
    pub fn update_metadata(
        &mut self,
        file_ids: &[i64],
        is_favorite: Option<bool>,
        rating: Option<i32>,
        rotate_delta: Option<i32>,
        culling_flag: Option<i32>,
    ) {
        for &fid in file_ids {
            if let Some(&idx) = self.id_to_index.get(&fid) {
                if let Some(fav) = is_favorite {
                    self.is_favorites[idx] = if fav { 1 } else { 0 };
                }
                if let Some(r) = rating {
                    self.star_ratings[idx] = r.clamp(0, 5) as i16;
                }
                if let Some(delta) = rotate_delta {
                    let cur = self.rotations[idx] as i32;
                    self.rotations[idx] = (((cur + delta) % 360 + 360) % 360) as i16;
                }
                if let Some(cf) = culling_flag {
                    self.culling_flags[idx] = cf.clamp(0, 2) as i8;
                }
            }
        }
    }

    pub fn set_hidden(&mut self, file_ids: &[i64], is_hidden: bool) {
        let val = if is_hidden { 1 } else { 0 };
        for &fid in file_ids {
            if let Some(&idx) = self.id_to_index.get(&fid) {
                if idx < self.is_hiddens.len() {
                    self.is_hiddens[idx] = val;
                }
            }
        }
    }

    /// Set or update the tag list for a single file.
    pub fn set_file_tags(&mut self, file_id: i64, new_tags: Vec<i64>) {
        if let Some(&idx) = self.id_to_index.get(&file_id) {
            self.set_row_tags(idx, new_tags);
        }
    }

    /// Add a tag to a file.
    pub fn add_file_tag(&mut self, file_id: i64, tag_id: i64) {
        if let Some(&idx) = self.id_to_index.get(&file_id) {
            if !self.row_tags[idx].contains(&tag_id) {
                self.row_tags[idx].push(tag_id);
                self.tag_to_indices.entry(tag_id).or_default().push(idx);
            }
        }
    }

    /// Remove a tag from a file.
    pub fn remove_file_tag(&mut self, file_id: i64, tag_id: i64) {
        if let Some(&idx) = self.id_to_index.get(&file_id) {
            self.row_tags[idx].retain(|&t| t != tag_id);
            if let Some(list) = self.tag_to_indices.get_mut(&tag_id) {
                list.retain(|&i| i != idx);
            }
        }
    }

    fn set_row_tags(&mut self, idx: usize, new_tags: Vec<i64>) {
        let old_tags = std::mem::replace(&mut self.row_tags[idx], new_tags.clone());
        for &t in &old_tags {
            if let Some(list) = self.tag_to_indices.get_mut(&t) {
                list.retain(|&i| i != idx);
            }
        }
        for &t in &new_tags {
            let list = self.tag_to_indices.entry(t).or_default();
            if !list.contains(&idx) {
                list.push(idx);
            }
        }
    }

    /// Select matching row indices based on common filter parameters.
    pub fn filter_indices(
        &self,
        is_favorite: bool,
        rating: i64,
        culling_flag: i64,
        tag_id: i64,
        start_date: i64,
        end_date: i64,
        sort_type: i64,
    ) -> Vec<usize> {
        let n = self.len();
        if n == 0 {
            return Vec::new();
        }

        // Determine base index candidate set: if filtering by tag_id > 0,
        // start directly with the inverted index list instead of scanning all rows.
        let base_indices: Option<&[usize]> = if tag_id > 0 {
            self.tag_to_indices.get(&tag_id).map(|v| v.as_slice())
        } else {
            None
        };

        let mut matches = match base_indices {
            Some(indices) => Vec::with_capacity(indices.len()),
            None => Vec::with_capacity(n),
        };

        let show_hidden = crate::t_sqlite::show_hidden_pictures();
        let check_candidate = |idx: usize| -> bool {
            if !show_hidden && self.is_hiddens.get(idx).copied().unwrap_or(0) != 0 {
                return false;
            }
            if is_favorite && self.is_favorites[idx] != 1 {
                return false;
            }
            if rating == -2 {
                if self.star_ratings[idx] <= 0 {
                    return false;
                }
            } else if rating == 0 {
                if self.star_ratings[idx] != 0 {
                    return false;
                }
            } else if rating > 0 && self.star_ratings[idx] != rating as i16 {
                return false;
            }

            if (0..=2).contains(&culling_flag) && self.culling_flags[idx] != culling_flag as i8 {
                return false;
            }

            if start_date > 0 && end_date > 0 {
                let ts = match sort_type {
                    1 => self.created_ats[idx],
                    2 => self.modified_ats[idx],
                    _ => self.timestamps[idx],
                };
                if ts < start_date || ts >= end_date {
                    return false;
                }
            }

            true
        };

        if let Some(candidates) = base_indices {
            for &idx in candidates {
                if idx < n && check_candidate(idx) {
                    matches.push(idx);
                }
            }
        } else {
            for idx in 0..n {
                if check_candidate(idx) {
                    matches.push(idx);
                }
            }
        }

        matches
    }

    /// Helper to collect (timestamp, packed_date) rows without extra vector allocations.
    fn collect_filtered_rows(
        &self,
        sort_type: i64,
        is_favorite: bool,
        rating: i64,
        culling_flag: i64,
        tag_id: i64,
        start_date: i64,
        end_date: i64,
    ) -> Vec<(i64, u32)> {
        let n = self.file_ids.len();
        if n == 0 {
            return Vec::new();
        }

        let base_indices: Option<&[usize]> = if tag_id > 0 {
            self.tag_to_indices.get(&tag_id).map(|v| v.as_slice())
        } else {
            None
        };

        let has_filter = is_favorite
            || rating != -1
            || (0..=2).contains(&culling_flag)
            || tag_id > 0
            || (start_date > 0 && end_date > 0);

        if !has_filter {
            let mut rows = Vec::with_capacity(n);
            match sort_type {
                0 => {
                    for i in 0..n {
                        rows.push((self.timestamps[i], self.packed_dates[i]));
                    }
                }
                1 => {
                    for i in 0..n {
                        let ts = self.created_ats[i];
                        let civil = timestamp_to_civil(ts);
                        rows.push((ts, pack_ymd(civil.year, civil.month, civil.day)));
                    }
                }
                2 => {
                    for i in 0..n {
                        let ts = self.modified_ats[i];
                        let civil = timestamp_to_civil(ts);
                        rows.push((ts, pack_ymd(civil.year, civil.month, civil.day)));
                    }
                }
                _ => {
                    for i in 0..n {
                        rows.push((self.timestamps[i], self.packed_dates[i]));
                    }
                }
            }
            return rows;
        }

        let capacity = match base_indices {
            Some(indices) => indices.len(),
            None => n,
        };
        let mut rows = Vec::with_capacity(capacity);

        let show_hidden = crate::t_sqlite::show_hidden_pictures();
        let check_candidate = |idx: usize| -> bool {
            if !show_hidden && self.is_hiddens.get(idx).copied().unwrap_or(0) != 0 {
                return false;
            }
            if is_favorite && self.is_favorites[idx] != 1 {
                return false;
            }
            if rating == -2 {
                if self.star_ratings[idx] <= 0 {
                    return false;
                }
            } else if rating == 0 {
                if self.star_ratings[idx] != 0 {
                    return false;
                }
            } else if rating > 0 && self.star_ratings[idx] != rating as i16 {
                return false;
            }

            if (0..=2).contains(&culling_flag) && self.culling_flags[idx] != culling_flag as i8 {
                return false;
            }

            if start_date > 0 && end_date > 0 {
                let ts = match sort_type {
                    1 => self.created_ats[idx],
                    2 => self.modified_ats[idx],
                    _ => self.timestamps[idx],
                };
                if ts < start_date || ts >= end_date {
                    return false;
                }
            }

            true
        };

        let push_row = |idx: usize, rows: &mut Vec<(i64, u32)>| {
            let ts = match sort_type {
                1 => self.created_ats[idx],
                2 => self.modified_ats[idx],
                _ => self.timestamps[idx],
            };
            let packed = if sort_type == 0 {
                self.packed_dates[idx]
            } else {
                let civil = timestamp_to_civil(ts);
                pack_ymd(civil.year, civil.month, civil.day)
            };
            rows.push((ts, packed));
        };

        if let Some(candidates) = base_indices {
            for &idx in candidates {
                if idx < n && check_candidate(idx) {
                    push_row(idx, &mut rows);
                }
            }
        } else {
            for idx in 0..n {
                if check_candidate(idx) {
                    push_row(idx, &mut rows);
                }
            }
        }

        rows
    }

    /// Compute timeline markers (year, month, date, position) from columnar data.
    pub fn compute_timeline(
        &self,
        sort_type: i64,
        sort_order: i64,
        is_favorite: bool,
        rating: i64,
        culling_flag: i64,
        tag_id: i64,
        start_date: i64,
        end_date: i64,
    ) -> Vec<ATimeLine> {
        let mut rows = self.collect_filtered_rows(
            sort_type,
            is_favorite,
            rating,
            culling_flag,
            tag_id,
            start_date,
            end_date,
        );

        if rows.is_empty() {
            return Vec::new();
        }

        // Sort rows chronologically
        if sort_order == 0 {
            rows.sort_unstable_by_key(|&(ts, _)| ts);
        } else {
            rows.sort_unstable_by_key(|&(ts, _)| std::cmp::Reverse(ts));
        }

        // Group by (year, month, date) and record MIN(position)
        let mut timelines: Vec<ATimeLine> = Vec::new();
        let mut last_packed: u32 = u32::MAX;

        for (pos, &(_ts, packed)) in rows.iter().enumerate() {
            if packed != last_packed {
                last_packed = packed;
                let civil = unpack_ymd(packed);
                timelines.push(ATimeLine {
                    year: Some(civil.year),
                    month: Some(civil.month),
                    date: Some(civil.day),
                    position: pos as i64,
                });
            }
        }

        timelines
    }

    /// Compute frequency histogram buckets across date intervals (e.g. for timeline scrubber bars).
    pub fn compute_histogram_buckets(
        &self,
        sort_type: i64,
        sort_order: i64,
        is_favorite: bool,
        rating: i64,
        culling_flag: i64,
        tag_id: i64,
        start_date: i64,
        end_date: i64,
    ) -> Vec<TimelineHistogramBucket> {
        let mut rows = self.collect_filtered_rows(
            sort_type,
            is_favorite,
            rating,
            culling_flag,
            tag_id,
            start_date,
            end_date,
        );

        if rows.is_empty() {
            return Vec::new();
        }

        if sort_order == 0 {
            rows.sort_unstable_by_key(|&(ts, _)| ts);
        } else {
            rows.sort_unstable_by_key(|&(ts, _)| std::cmp::Reverse(ts));
        }

        let mut buckets: Vec<TimelineHistogramBucket> = Vec::new();
        let mut last_packed: u32 = u32::MAX;

        for (pos, &(_ts, packed)) in rows.iter().enumerate() {
            if packed == last_packed {
                if let Some(last_bucket) = buckets.last_mut() {
                    last_bucket.count += 1;
                    continue;
                }
            }

            let civil = unpack_ymd(packed);
            buckets.push(TimelineHistogramBucket {
                year: civil.year,
                month: civil.month,
                date: Some(civil.day),
                count: 1,
                position: pos as i64,
            });
            last_packed = packed;
        }

        buckets
    }

    /// Retrieve filtered file IDs in sort order directly from the in-memory projection table.
    pub fn get_filtered_file_ids(
        &self,
        sort_type: i64,
        sort_order: i64,
        is_favorite: bool,
        rating: i64,
        culling_flag: i64,
        tag_id: i64,
        start_date: i64,
        end_date: i64,
    ) -> Vec<i64> {
        let matching_indices = self.filter_indices(
            is_favorite,
            rating,
            culling_flag,
            tag_id,
            start_date,
            end_date,
            sort_type,
        );

        if matching_indices.is_empty() {
            return Vec::new();
        }

        let mut pairs: Vec<(i64, i64)> = matching_indices
            .iter()
            .map(|&idx| {
                let ts = match sort_type {
                    1 => self.created_ats[idx],
                    2 => self.modified_ats[idx],
                    _ => self.timestamps[idx],
                };
                (ts, self.file_ids[idx])
            })
            .collect();

        if sort_order == 0 {
            pairs.sort_unstable_by_key(|&(ts, _)| ts);
        } else {
            pairs.sort_unstable_by_key(|&(ts, _)| std::cmp::Reverse(ts));
        }

        pairs.into_iter().map(|(_, fid)| fid).collect()
    }
}

// ---------------------------------------------------------------------------
// Global In-Memory Columnar Projections Singleton
// ---------------------------------------------------------------------------

static PROJECTIONS: Lazy<RwLock<ColumnarTable>> = Lazy::new(|| RwLock::new(ColumnarTable::new()));
static PROJECTION_INIT_LOCK: Lazy<std::sync::Mutex<()>> = Lazy::new(|| std::sync::Mutex::new(()));
static IS_INITIALIZED: Lazy<RwLock<bool>> = Lazy::new(|| RwLock::new(false));

/// Check if the in-memory projections are loaded.
pub fn is_initialized() -> bool {
    *IS_INITIALIZED.read().unwrap()
}

/// Reset / invalidate projections (e.g. upon library change).
pub fn invalidate() {
    let _lock = PROJECTION_INIT_LOCK.lock().unwrap();
    let mut table = PROJECTIONS.write().unwrap();
    table.clear();
    *IS_INITIALIZED.write().unwrap() = false;
}

/// Populate the in-memory columnar table from SQLite `afiles` and `afile_tags`.
pub fn ensure_initialized(conn: &Connection) -> Result<(), String> {
    if is_initialized() {
        return Ok(());
    }

    let _lock = PROJECTION_INIT_LOCK.lock().unwrap();
    if *IS_INITIALIZED.read().unwrap() {
        return Ok(());
    }

    let start = Instant::now();
    let count: usize = conn
        .query_row("SELECT COUNT(*) FROM afiles", [], |r| r.get(0))
        .unwrap_or(0);

    let mut new_table = ColumnarTable::with_capacity(count);

    // Stream flat columns from afiles
    let mut stmt = conn
        .prepare(
            "SELECT id, folder_id, taken_date, created_at, modified_at, is_favorite, rating, culling_flag, rotate, COALESCE(is_hidden, 0)
             FROM afiles
             ORDER BY id ASC",
        )
        .map_err(|e| e.to_string())?;

    let mut rows = stmt
        .query([])
        .map_err(|e| e.to_string())?;

    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let file_id: i64 = row.get(0).map_err(|e| e.to_string())?;
        let folder_id: i64 = row.get(1).map_err(|e| e.to_string())?;
        let taken_date: Option<i64> = row.get(2).ok();
        let created_at: Option<i64> = row.get(3).ok();
        let modified_at: Option<i64> = row.get(4).ok();
        let is_favorite: Option<bool> = row.get(5).ok();
        let rating: Option<i32> = row.get(6).ok();
        let culling_flag: Option<i32> = row.get(7).ok();
        let rotate: Option<i32> = row.get(8).ok();
        let is_hidden: i32 = row.get(9).unwrap_or(0);

        new_table.upsert_file(
            file_id,
            folder_id,
            taken_date,
            created_at,
            modified_at,
            rating,
            is_favorite,
            rotate,
            culling_flag,
            None,
        );
        if is_hidden != 0 {
            new_table.set_hidden(&[file_id], true);
        }
    }

    // Stream tags from afile_tags
    if let Ok(mut tag_stmt) = conn.prepare("SELECT file_id, tag_id FROM afile_tags") {
        if let Ok(mut tag_rows) = tag_stmt.query([]) {
            while let Ok(Some(trow)) = tag_rows.next() {
                if let (Ok(fid), Ok(tid)) = (trow.get::<_, i64>(0), trow.get::<_, i64>(1)) {
                    new_table.add_file_tag(fid, tid);
                }
            }
        }
    }

    let mut table = PROJECTIONS.write().unwrap();
    *table = new_table;
    *IS_INITIALIZED.write().unwrap() = true;

    println!(
        "[projections] Loaded {} files into in-memory columnar store in {:?}",
        table.len(),
        start.elapsed()
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Synchronization Hooks called from SQLite Write Sites
// ---------------------------------------------------------------------------

/// Hook: New file inserted or updated in SQLite
pub fn hook_upsert_file(
    file_id: i64,
    folder_id: i64,
    taken_date: Option<i64>,
    created_at: Option<i64>,
    modified_at: Option<i64>,
    star_rating: Option<i32>,
    is_favorite: Option<bool>,
    rotation: Option<i32>,
    culling_flag: Option<i32>,
) {
    if !is_initialized() {
        return;
    }
    if let Ok(mut table) = PROJECTIONS.write() {
        table.upsert_file(
            file_id,
            folder_id,
            taken_date,
            created_at,
            modified_at,
            star_rating,
            is_favorite,
            rotation,
            culling_flag,
            None,
        );
    }
}

/// Hook: Files deleted from SQLite
pub fn hook_delete_files(file_ids: &[i64]) {
    if !is_initialized() || file_ids.is_empty() {
        return;
    }
    if let Ok(mut table) = PROJECTIONS.write() {
        table.delete_files(file_ids);
    }
}

/// Hook: Metadata updated in batch_update_metadata
pub fn hook_update_metadata(
    file_ids: &[i64],
    is_favorite: Option<bool>,
    rating: Option<i32>,
    rotate_delta: Option<i32>,
    culling_flag: Option<i32>,
) {
    if !is_initialized() || file_ids.is_empty() {
        return;
    }
    if let Ok(mut table) = PROJECTIONS.write() {
        table.update_metadata(file_ids, is_favorite, rating, rotate_delta, culling_flag);
    }
}

pub fn hook_set_file_hidden(file_ids: &[i64], is_hidden: bool) {
    if !is_initialized() || file_ids.is_empty() {
        return;
    }
    if let Ok(mut table) = PROJECTIONS.write() {
        table.set_hidden(file_ids, is_hidden);
    }
}

/// Hook: Tag added to file in SQLite
pub fn hook_add_tag(file_id: i64, tag_id: i64) {
    if !is_initialized() {
        return;
    }
    if let Ok(mut table) = PROJECTIONS.write() {
        table.add_file_tag(file_id, tag_id);
    }
}

/// Hook: Tag removed from file in SQLite
pub fn hook_remove_tag(file_id: i64, tag_id: i64) {
    if !is_initialized() {
        return;
    }
    if let Ok(mut table) = PROJECTIONS.write() {
        table.remove_file_tag(file_id, tag_id);
    }
}

/// Hook: Complete tag list set for file
pub fn hook_set_file_tags(file_id: i64, tag_ids: Vec<i64>) {
    if !is_initialized() {
        return;
    }
    if let Ok(mut table) = PROJECTIONS.write() {
        table.set_file_tags(file_id, tag_ids);
    }
}

// ---------------------------------------------------------------------------
// Query Dispatch: Fast In-Memory Timeline and Filtering
// ---------------------------------------------------------------------------

/// Check whether the given query parameters can be evaluated purely from in-memory projections.
pub fn can_handle_query(params: &QueryParams) -> bool {
    // Queries requiring text searches, faces, GPS bounding box, or album joins
    // delegate to SQLite.
    if !params.search_file_name.is_empty() {
        return false;
    }
    if let Some(q) = &params.search_query {
        if !q.trim().is_empty() {
            return false;
        }
    }
    if let Some(tokens) = &params.search_tokens {
        if !tokens.is_empty() {
            return false;
        }
    }
    if params.person_id > 0 || params.person_suggestions || params.person_ignored {
        return false;
    }
    if params.people_all || params.folders_all {
        return false;
    }
    if !params.make.is_empty() || !params.model.is_empty() {
        return false;
    }
    if !params.location_admin1.is_empty() || !params.location_name.is_empty() {
        return false;
    }
    if params.gps_min_lat.is_some() || params.gps_max_lat.is_some() {
        return false;
    }
    if params.tag_group_id > 0 {
        return false;
    }
    if !params.search_folder.is_empty() || !params.search_all_subfolders.is_empty() {
        return false;
    }
    if params.search_file_type != 0 {
        return false;
    }

    true
}

/// Query timeline markers directly from in-memory columnar projections.
pub fn query_timeline(conn: &Connection, params: &QueryParams) -> Result<Option<Vec<ATimeLine>>, String> {
    if params.sort_type > 2 || params.people_all || params.folders_all {
        return Ok(Some(Vec::new()));
    }
    if !can_handle_query(params) {
        return Ok(None);
    }

    if conn.path().map_or(true, |p| p.is_empty()) {
        return Ok(None);
    }

    ensure_initialized(conn)?;

    let table = PROJECTIONS.read().unwrap();
    let timelines = table.compute_timeline(
        params.sort_type,
        params.sort_order,
        params.is_favorite,
        params.rating,
        params.culling_flag,
        params.tag_id,
        params.start_date,
        params.end_date,
    );

    Ok(Some(timelines))
}

/// Query histogram buckets directly from in-memory columnar projections.
pub fn query_histogram_buckets(
    conn: &Connection,
    params: &QueryParams,
) -> Result<Option<Vec<TimelineHistogramBucket>>, String> {
    if params.sort_type > 2 || params.people_all || params.folders_all {
        return Ok(Some(Vec::new()));
    }
    if !can_handle_query(params) {
        return Ok(None);
    }

    if conn.path().map_or(true, |p| p.is_empty()) {
        return Ok(None);
    }

    ensure_initialized(conn)?;

    let table = PROJECTIONS.read().unwrap();
    let buckets = table.compute_histogram_buckets(
        params.sort_type,
        params.sort_order,
        params.is_favorite,
        params.rating,
        params.culling_flag,
        params.tag_id,
        params.start_date,
        params.end_date,
    );

    Ok(Some(buckets))
}

/// Query smart timeline markers directly from in-memory columnar projections.
pub fn query_smart_timeline(
    conn: &Connection,
    params: &SmartQueryParams,
) -> Result<Option<Vec<ATimeLine>>, String> {
    if params.sort_type > 2 {
        return Ok(Some(Vec::new()));
    }
    // If smart query has rules or gps bounding box, delegate to SQLite engine
    if !params.rules.is_empty()
        || params.gps_min_lat.is_some()
        || params.gps_max_lat.is_some()
        || params.gps_min_lon.is_some()
        || params.gps_max_lon.is_some()
    {
        return Ok(None);
    }

    if conn.path().map_or(true, |p| p.is_empty()) {
        return Ok(None);
    }

    ensure_initialized(conn)?;

    let table = PROJECTIONS.read().unwrap();
    let timelines = table.compute_timeline(
        params.sort_type,
        params.sort_order,
        false,
        -1,
        -1,
        0,
        0,
        0,
    );

    Ok(Some(timelines))
}

/// Query filtered file IDs directly from in-memory columnar projections for star/favorite/tag filtering.
pub fn query_filtered_file_ids(
    conn: &Connection,
    params: &QueryParams,
) -> Result<Option<Vec<i64>>, String> {
    // Only handle pure favorite/star/tag queries
    if !can_handle_query(params) {
        return Ok(None);
    }
    // Only handle date-based or default sorts
    if params.sort_type > 2 {
        return Ok(None);
    }

    if conn.path().map_or(true, |p| p.is_empty()) {
        return Ok(None);
    }

    ensure_initialized(conn)?;

    let table = PROJECTIONS.read().unwrap();
    let ids = table.get_filtered_file_ids(
        params.sort_type,
        params.sort_order,
        params.is_favorite,
        params.rating,
        params.culling_flag,
        params.tag_id,
        params.start_date,
        params.end_date,
    );

    Ok(Some(ids))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn test_civil_calendar_conversions() {
        let civil_1970 = timestamp_to_civil(0);
        assert_eq!((civil_1970.year, civil_1970.month, civil_1970.day), (1970, 1, 1));

        // 2026-10-13 21:00:00 UTC = 1791925200
        let civil_2026 = timestamp_to_civil(1791925200);
        assert_eq!((civil_2026.year, civil_2026.month, civil_2026.day), (2026, 10, 13));

        let packed = pack_ymd(2026, 10, 13);
        let unpacked = unpack_ymd(packed);
        assert_eq!((unpacked.year, unpacked.month, unpacked.day), (2026, 10, 13));
    }

    #[test]
    fn test_columnar_table_crud_and_tag_filtering() {
        let mut table = ColumnarTable::new();

        // Insert 3 files
        table.upsert_file(101, 1, Some(1791925200), None, None, Some(5), Some(true), Some(90), Some(1), Some(vec![10, 20]));
        table.upsert_file(102, 1, Some(1791925300), None, None, Some(3), Some(false), Some(0), Some(0), Some(vec![20]));
        table.upsert_file(103, 1, Some(1791925400), None, None, Some(5), Some(false), Some(180), Some(2), Some(vec![30]));

        assert_eq!(table.len(), 3);

        // Filter favorite
        let favs = table.get_filtered_file_ids(0, 0, true, -1, -1, 0, 0, 0);
        assert_eq!(favs, vec![101]);

        // Filter 5-star
        let five_stars = table.get_filtered_file_ids(0, 0, false, 5, -1, 0, 0, 0);
        assert_eq!(five_stars, vec![101, 103]);

        // Filter tag 20
        let tagged_20 = table.get_filtered_file_ids(0, 0, false, -1, -1, 20, 0, 0);
        assert_eq!(tagged_20, vec![101, 102]);

        // Delete file 102
        table.delete_files(&[102]);
        assert_eq!(table.len(), 2);
        let tagged_20_after = table.get_filtered_file_ids(0, 0, false, -1, -1, 20, 0, 0);
        assert_eq!(tagged_20_after, vec![101]);

        // Update metadata on file 103
        table.update_metadata(&[103], Some(true), Some(4), Some(90), Some(1));
        assert_eq!(table.is_favorites[table.id_to_index[&103]], 1);
        assert_eq!(table.star_ratings[table.id_to_index[&103]], 4);
        assert_eq!(table.rotations[table.id_to_index[&103]], 270);
    }

    #[test]
    fn test_benchmark_50k_collection_scrubbing_under_2ms() {
        // Acceptance Criterion 3:
        // Scrubbing across a 50,000+ photo collection calculates histogram buckets in <2ms.
        let mut table = ColumnarTable::with_capacity(60000);
        let base_ts = 1577836800; // 2020-01-01

        for i in 0..50000 {
            let ts = base_ts + (i as i64) * 3600; // 1 photo every hour across several years
            let rating = (i % 6) as i32;
            let fav = (i % 5) == 0;
            let tag = (i % 10) as i64;
            table.upsert_file(
                i as i64 + 1,
                1,
                Some(ts),
                Some(ts),
                Some(ts),
                Some(rating),
                Some(fav),
                Some(0),
                Some(0),
                Some(vec![tag]),
            );
        }

        assert_eq!(table.len(), 50000);

        // Warm up and benchmark histogram bucket calculation across the full 50,000 collection
        let mut min_elapsed = std::time::Duration::from_secs(10);
        let mut best_buckets = Vec::new();
        for _ in 0..3 {
            let start = Instant::now();
            let buckets = table.compute_histogram_buckets(0, 0, false, -1, -1, 0, 0, 0);
            let elapsed = start.elapsed();
            if elapsed < min_elapsed {
                min_elapsed = elapsed;
                best_buckets = buckets;
            }
        }

        println!(
            "[Benchmark] 50,000 photo histogram calculated {} buckets in {:?}",
            best_buckets.len(),
            min_elapsed
        );

        assert!(!best_buckets.is_empty());
        assert!(
            min_elapsed.as_millis() < 2,
            "Timeline scrubber histogram calculation exceeded 2ms threshold: {:?}",
            min_elapsed
        );

        // Benchmark timeline markers calculation across 50,000 collection with filtering
        let start_filter = Instant::now();
        let timelines = table.compute_timeline(0, 0, true, 5, -1, 0, 0, 0);
        let elapsed_filter = start_filter.elapsed();

        println!(
            "[Benchmark] 50,000 photo filtered timeline markers generated {} markers in {:?}",
            timelines.len(),
            elapsed_filter
        );

        assert!(
            elapsed_filter.as_millis() < 2,
            "Filtered timeline calculation exceeded 2ms threshold: {:?}",
            elapsed_filter
        );
    }
}
