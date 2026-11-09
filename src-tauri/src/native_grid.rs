//! Native photo grid (LAP-062): a child window over the page's grid area that draws the
//! People and Folders tiles itself, as Picasa does, so a click paints the whole screen at
//! once. The page reports where its grid is and which query it shows; this window covers
//! exactly that rectangle and draws the same rows (AFile::get_grouped_query_rows).
//! Windows only for now; elsewhere the commands do nothing and the web grid stays.
//!
//! Every paint is complete: missing rows are fetched and missing tiles decoded (in
//! parallel) before the frame is drawn. Decoded tiles are kept in a bounded cache (the screen
//! plus a prefetch band around it, least recently drawn evicted first), the band ahead of the
//! scroll is decoded in the background, and a background pass fills the rows and makes sure
//! the start of every section has a cached source after the view opens.
use crate::t_sqlite::{AFile, GroupedQueryGroup, GroupedQueryRow, QueryParams};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

// Events to the page: grid clicks and the section at the top while scrolling.
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
// The model last shown, so a refresh can re-read the same query (a scan adding files).
static SHOWN: Mutex<Option<Arc<Model>>> = Mutex::new(None);

fn emit(event: &str, payload: serde_json::Value) {
    use tauri::Emitter;
    if let Some(app) = APP.get() { let _ = app.emit(event, payload); }
}

/// The page's grid rectangle in CSS pixels (relative to the webview's top-left).
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct GridRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Show the native grid over `rect`, or hide it when `rect` is null. `holes` (relative to
/// `rect`) are page elements drawn over the grid, left out of the window so they show.
#[tauri::command]
pub fn native_grid_place(window: tauri::WebviewWindow, rect: Option<GridRect>, holes: Option<Vec<GridRect>>) -> Result<(), String> {
    #[cfg(windows)]
    {
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let parent = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
        let holes = holes.unwrap_or_default();
        window.run_on_main_thread(move || win::place(parent, rect, &holes, scale)).map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    let _ = (window, rect, holes);
    Ok(())
}

/// Draw this query's grouped rows (the page's People/Folders query), tiles `tile` CSS px.
#[tauri::command]
pub fn native_grid_show(window: tauri::WebviewWindow, params: QueryParams, tile: f64, face_size: u32, library_id: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let _ = APP.set(tauri::Manager::app_handle(&window).clone());
        let started = std::time::Instant::now();
        let groups = AFile::get_grouped_query_rows(&params, 0, 0)?.groups;
        trace(format_args!("show groups={} tile={tile} scale={scale} ms={}", groups.len(), started.elapsed().as_millis()));
        let cell = (tile * scale).round().max(32.0) as u32;
        // The page's item is the tile plus 20 CSS px (padding, border, gap): the same column pitch
        // gives the same column count as the page's grid (LAP-071). Exactly the page's, not the
        // rounded cell's: a cell rounded up would widen every column and lose the last one.
        let pitch = (tile + 20.0) * scale;
        let model = Arc::new(Model::new(params, groups, cell, pitch, scale, face_size, library_id));
        Model::fill_in_background(model.clone());
        *SHOWN.lock().unwrap() = Some(model.clone());
        window.run_on_main_thread(move || win::show(model)).map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    let _ = (window, params, tile, face_size, library_id);
    Ok(())
}

/// Re-read the shown query's rows (files a scan has just added), keeping the scroll position
/// and the pictures already decoded, so new photos appear without the view moving. With
/// `focus` (a folder path), bring that folder's first section to the top if it now exists;
/// returns whether it did.
#[tauri::command]
pub fn native_grid_refresh(window: tauri::WebviewWindow, focus: Option<String>) -> Result<bool, String> {
    #[cfg(windows)]
    {
        let Some(old) = SHOWN.lock().unwrap().clone() else { return Ok(false) };
        let started = std::time::Instant::now();
        // A new load seed, as the page sends on each load: the People/Folders section lists are
        // cached per load, and a scan's new folder must not be served the old list.
        let mut params = old.params.clone();
        params.random_seed = params.random_seed.wrapping_add(1);
        let groups = AFile::get_grouped_query_rows(&params, 0, 0)?.groups;
        let focus = focus.map(|f| f.to_lowercase());
        let focus_row = focus.as_deref().and_then(|f| groups.iter()
            .find(|g| g.group_id.to_lowercase().starts_with(f)).map(|g| g.row_index));
        trace(format_args!("refresh groups={} focus={focus_row:?} ms={}", groups.len(), started.elapsed().as_millis()));
        let model = Model::new(params, groups, old.cell, old.pitch, old.scale, old.face_size, old.library_id.clone());
        *model.tiles.lock().unwrap() = old.tiles.lock().unwrap().clone();
        let model = Arc::new(model);
        Model::fill_in_background(model.clone());
        *SHOWN.lock().unwrap() = Some(model.clone());
        window.run_on_main_thread(move || win::replace(model, focus_row)).map_err(|e| e.to_string())?;
        return Ok(focus_row.is_some());
    }
    #[cfg(not(windows))]
    { let _ = (window, focus); Ok(false) }
}

/// Update a photo's cached tile in the shown native grid after its edit recipe changes.
pub fn refresh_file(file_id: i64) {
    let Some(model) = SHOWN.lock().unwrap().clone() else { return };
    let edits = AFile::get_file_info(file_id).ok().flatten().and_then(|f| f.edits);
    let mut changed = Vec::new();
    {
        let mut rows = model.rows.lock().unwrap();
        for item in rows.iter_mut().flatten() {
            if item.file_id == file_id {
                item.edits = edits.clone();
                if item.rotate != 0 && edits.is_some() { item.rotate = 0; }
                changed.push(item.clone());
            }
        }
    }
    if changed.is_empty() { return; }
    {
        let mut tiles = model.tiles.lock().unwrap();
        let mut failed = model.upgrade_failed.lock().unwrap();
        for item in &changed {
            let key = (item.file_id, item.person.unwrap_or(0));
            tiles.map.remove(&key);
            failed.remove(&key);
        }
    }
    model.ensure_tiles(&changed, 2, true);
    #[cfg(windows)]
    win::invalidate();
}

/// Bring a section (its header row index) to the top, at once (Picasa jumps, no animation).
#[tauri::command]
pub fn native_grid_scroll_to(window: tauri::WebviewWindow, row_index: i64) -> Result<(), String> {
    #[cfg(windows)]
    window.run_on_main_thread(move || win::scroll_to_row(row_index)).map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    let _ = (window, row_index);
    Ok(())
}

/// The page's scrollbar (Build 46): a place in the whole list, 0..1 of the scroll range.
#[tauri::command]
pub fn native_grid_scroll_ratio(window: tauri::WebviewWindow, ratio: f64) -> Result<(), String> {
    #[cfg(windows)]
    window.run_on_main_thread(move || win::scroll_to_ratio(ratio)).map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    let _ = (window, ratio);
    Ok(())
}

/// The page's review state for one person it has loaded: pending suggestions, and faces
/// ignored this session (drawn dimmed with Undo until the page drops them).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PersonMarks {
    pub suggested: std::collections::HashSet<i64>,
    pub dimmed: std::collections::HashSet<i64>,
}

// Once the page has a person's state it is the source of truth for that person's marks;
// people it has not loaded are drawn from the database. Kept across views (restacks).
fn page_marks() -> &'static Mutex<HashMap<i64, PersonMarks>> {
    static MARKS: OnceLock<Mutex<HashMap<i64, PersonMarks>>> = OnceLock::new();
    MARKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Update people's review marks (`reset`: forget every person not in `marks`), then repaint.
#[tauri::command]
pub fn native_grid_marks(window: tauri::WebviewWindow, marks: HashMap<i64, PersonMarks>, reset: bool) -> Result<(), String> {
    {
        let mut all = page_marks().lock().unwrap();
        if reset { all.clear(); }
        all.extend(marks);
    }
    #[cfg(windows)]
    window.run_on_main_thread(win::repaint).map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    let _ = window;
    Ok(())
}

/// How a face tile is drawn in a person's section.
#[derive(Clone, Copy, PartialEq)]
enum Mark { Confirmed, Suggested, Dimmed }

/// One drawable item of a section.
#[derive(Clone)]
struct Item {
    file_id: i64,
    // Index in the page's file list, used for selection and opening.
    file_index: i64,
    rotate: i32,
    path: String,
    version: i64,
    edits: Option<String>,
    // Some(person id) for a People section (face tile), None for a folder (whole photo).
    person: Option<i64>,
    // Hidden (Picture > Hide, LAP-050): listed only with View > Hidden Pictures on, drawn at 50%.
    hidden: bool,
}

/// A person's face on one photo, and how the database says it is drawn.
struct FaceBox {
    bbox: serde_json::Value,
    face_id: i64,
    mark: Mark,
}

/// A decoded tile, 0x00RRGGBB pixels, exactly w x h.
struct Tile {
    w: u32,
    h: u32,
    px: Vec<u32>,
    // A face cut from a thumbnail too small for full quality; the sharp one is on its way.
    soft: bool,
}

/// A tile's source before it is fitted to the cell: encoded bytes or an already cut image.
enum Source {
    Bytes(Vec<u8>),
    Image(image::DynamicImage),
}

/// Decoded tiles keyed by (file id, person id or 0), each stamped when last drawn or decoded.
/// Holds about `capacity` tiles (the screen and the prefetch band, set by compose), so memory
/// stays flat however far the grid scrolls (LAP-076: unbounded, scrolling one person's 3,350
/// photos while the background pass decoded the start of 911 sections committed 10 GB).
#[derive(Clone)]
struct TileCache {
    map: HashMap<(i64, i64), (Arc<Tile>, u64)>,
    clock: u64,
    capacity: usize,
}

impl TileCache {
    fn get(&mut self, key: &(i64, i64)) -> Option<Arc<Tile>> {
        self.clock += 1;
        let clock = self.clock;
        self.map.get_mut(key).map(|(t, used)| { *used = clock; t.clone() })
    }

    /// Insert, then evict the least recently used tiles once the cache is a quarter over
    /// capacity (trimming in batches keeps eviction off the per-tile path).
    fn insert(&mut self, key: (i64, i64), tile: Arc<Tile>) {
        self.clock += 1;
        self.map.insert(key, (tile, self.clock));
        if self.map.len() > self.capacity + self.capacity / 4 { self.trim(); }
    }

    fn trim(&mut self) {
        if self.map.len() <= self.capacity { return; }
        let mut stamps: Vec<u64> = self.map.values().map(|(_, used)| *used).collect();
        let cut = self.map.len() - self.capacity;
        let oldest_kept = *stamps.select_nth_unstable(cut).1;
        self.map.retain(|_, (_, used)| *used >= oldest_kept);
    }

    fn bytes(&self) -> usize {
        self.map.values().map(|(t, _)| t.px.len() * 4).sum()
    }
}

// Calendar's date sections (t_sqlite GROUP_BY_DATE_*): Day 2, Month 3, Year 8, Quarter 11.
const GROUP_DAY: i64 = 2;
const GROUP_MONTH: i64 = 3;
const GROUP_YEAR: i64 = 8;
const GROUP_QUARTER: i64 = 11;

fn is_date_group(group_by: i64) -> bool {
    matches!(group_by, GROUP_DAY | GROUP_MONTH | GROUP_YEAR | GROUP_QUARTER)
}

/// A date section's header as the page writes it (Content.vue formatDateGroupLabel, English):
/// "January 1st, 2001", "January 2001", "2001 Q1 (Jan-Mar)", "2001"; "No date" for photos without one.
fn date_group_label(group_by: i64, group_id: &str) -> String {
    use chrono::{Datelike, TimeZone};
    let Some(t) = group_id.trim().parse::<i64>().ok().and_then(|ts| chrono::Local.timestamp_opt(ts, 0).single()) else {
        return "No date".to_string();
    };
    match group_by {
        GROUP_DAY => {
            let d = t.day();
            let suffix = match (d % 10, d % 100) { (1, n) if n != 11 => "st", (2, n) if n != 12 => "nd", (3, n) if n != 13 => "rd", _ => "th" };
            format!("{} {d}{suffix}, {}", t.format("%B"), t.year())
        }
        GROUP_MONTH => t.format("%B %Y").to_string(),
        GROUP_QUARTER => {
            const SHORT: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
            let q = (t.month0() / 3) as usize;
            format!("{} Q{} ({}-{})", t.year(), q + 1, SHORT[q * 3], SHORT[q * 3 + 2])
        }
        _ => t.year().to_string(),
    }
}

/// Rows, face boxes and decoded tiles for one shown query. Shared with background threads.
struct Model {
    params: QueryParams,
    groups: Vec<GroupedQueryGroup>,
    total_rows: usize,
    // Cell size in physical px (tile edge), column pitch (the page's item width), display scale.
    cell: u32,
    pitch: f64,
    scale: f64,
    // The page's face crop size and thumbnail library, so face crops share its cache.
    face_size: u32,
    library_id: String,
    rows: Mutex<Vec<Option<Item>>>,
    boxes: Mutex<HashMap<i64, Arc<HashMap<i64, FaceBox>>>>,
    tiles: Mutex<TileCache>,
    // A prefetch of the band around the screen is running (one at a time).
    prefetching: std::sync::atomic::AtomicBool,
    // Tiles drawn soft (or not at all) because the sharp face needs the original photo;
    // one background worker decodes those and redraws, never the paint itself.
    upgrades: Mutex<Vec<Item>>,
    upgrading: std::sync::atomic::AtomicBool,
    // Faces even the original could not give (kept out of the queue).
    upgrade_failed: Mutex<std::collections::HashSet<(i64, i64)>>,
}

const FACE_ASPECT: f64 = 5.0 / 6.0;
const ROW_CHUNK: i64 = 2000;
// Screens of tiles prefetched ahead of the scroll and behind it. The tile cache keeps
// KEEP_SCREENS screens, more than the screen plus both bands, so a frame's own tiles are never
// the ones evicted.
const AHEAD_SCREENS: f64 = 2.0;
const BEHIND_SCREENS: f64 = 1.0;
const KEEP_SCREENS: usize = 5;
// Tiles at the start of each section whose sources the background pass makes sure of.
const WARM_PER_SECTION: usize = 40;

/// Picasa's wheel motion (LAP-081, LAP-083), as reconstructed from Picasa (C:/Data/lap052/
/// picasa-wheel-re/findings.json; the page's grid uses the same, src-vite/src/common/wheelEasing.ts):
/// every detent, a lone one too, moves DETENT_PX and eases in over DETENT_MS (half with Shift):
/// 6% at 40 ms, 80% at 240 ms, 95% at 390 ms. Detents that come quickly overlap instead of
/// restarting, so fast wheeling glides and then slows down along a long tail. A reversal starts
/// from where the view is.
const DETENT_PX: f64 = 120.0;
const DETENT_MS: f64 = 900.0;

fn wheel_ease(t: f64) -> f64 {
    let raw = |x: f64| if x < 1.0 { x - 1.0 + (-x).exp() } else { (-1.0f64).exp() + (1.0 - (1.0 - x).exp()) * (1.0 - (-1.0f64).exp()) };
    if t <= 0.0 { 0.0 } else if t >= 1.0 { 1.0 } else { raw(t * 8.0) / raw(8.0) }
}

/// Scroll position under wheel motion: a base plus the detents still easing in (delta, start ms,
/// duration ms), times in ms on any steady clock.
#[derive(Default)]
struct WheelMotion {
    base: f64,
    segments: Vec<(f64, f64, f64)>,
}

impl WheelMotion {
    fn active(&self) -> bool { !self.segments.is_empty() }

    fn reset(&mut self, position: f64) {
        self.base = position;
        self.segments.clear();
    }

    /// Where the motion is heading: the position once every detent has eased in.
    fn destination(&self) -> f64 { self.base + self.segments.iter().map(|s| s.0).sum::<f64>() }

    fn sample(&mut self, now: f64, maximum: f64) -> f64 {
        let mut position = self.base;
        let mut base = self.base;
        self.segments.retain(|&(delta, start, duration)| {
            let t = (now - start) / duration;
            position += delta * wheel_ease(t);
            if t >= 1.0 { base += delta; false } else { true }
        });
        self.base = base;
        let bounded = position.clamp(0.0, maximum.max(0.0));
        if bounded != position { self.reset(bounded); }
        bounded
    }

    fn add(&mut self, delta: f64, now: f64, maximum: f64, shift: bool) {
        let position = self.sample(now, maximum);
        if self.segments.last().is_some_and(|s| s.0.signum() != delta.signum()) { self.reset(position); }
        let destination = self.destination();
        let bounded = (destination + delta).clamp(0.0, maximum.max(0.0));
        if bounded != destination {
            self.segments.push((bounded - destination, now + 10.0, if shift { DETENT_MS / 2.0 } else { DETENT_MS }));
        }
    }
}

impl Model {
    fn new(params: QueryParams, mut groups: Vec<GroupedQueryGroup>, cell: u32, pitch: f64, scale: f64, face_size: u32, library_id: String) -> Model {
        // LAP-091: date sections (Calendar) come labelled with their start time; draw the page's text.
        // People and Folders keep their own sections whatever group_by the view carries.
        if is_date_group(params.group_by) && !params.people_all && !params.folders_all {
            for g in &mut groups { g.label = date_group_label(params.group_by, &g.group_id); }
        }
        let total_rows = groups.last().map(|g| (g.row_index + g.count + 1) as usize).unwrap_or(0);
        Model { params, groups, total_rows, cell, pitch, scale, face_size, library_id, rows: Mutex::new(vec![None; total_rows]),
            boxes: Mutex::new(HashMap::new()), tiles: Mutex::new(TileCache { map: HashMap::new(), clock: 0, capacity: 256 }),
            prefetching: std::sync::atomic::AtomicBool::new(false),
            upgrades: Mutex::new(Vec::new()), upgrading: std::sync::atomic::AtomicBool::new(false),
            upgrade_failed: Mutex::new(std::collections::HashSet::new()) }
    }

    /// The person a section is, in the People view only: there a positive group id is a person.
    /// Elsewhere a numeric id is something else (a date section's start time in Calendar).
    fn is_person_group(&self, group_id: &str) -> Option<i64> {
        if !self.params.people_all { return None; }
        group_id.parse::<i64>().ok().filter(|id| *id > 0)
    }

    /// Fetch rows [start, start + n) into the store (no-op for rows already present).
    fn load_rows(&self, start: usize, n: usize) {
        let end = (start + n).min(self.total_rows);
        if start >= end { return; }
        if self.rows.lock().unwrap()[start..end].iter().all(|r| r.is_some()) { return; }
        let Ok(result) = AFile::get_grouped_query_rows(&self.params, start as i64, (end - start) as i64) else { return };
        let mut rows = self.rows.lock().unwrap();
        for (i, row) in result.rows.into_iter().enumerate() {
            if let GroupedQueryRow::Item { group_id, file_index, file, .. } = row {
                if let (Some(slot), Some(file_id)) = (rows.get_mut(start + i), file.id) {
                    *slot = Some(Item { file_id, file_index, rotate: file.rotate.unwrap_or(0), path: file.file_path.clone().unwrap_or_default(),
                        version: file.modified_at.unwrap_or(0), edits: file.edits.clone(), person: self.is_person_group(&group_id),
                        hidden: file.is_hidden.unwrap_or(false) });
                }
            }
        }
    }

    /// This person's face boxes by file id: confirmed and suggested faces, then (as the page
    /// does) faces ignored for the person, which stay in the list dimmed for a moment.
    fn person_boxes(&self, person: i64) -> Arc<HashMap<i64, FaceBox>> {
        if let Some(b) = self.boxes.lock().unwrap().get(&person) { return b.clone(); }
        let mut map = HashMap::new();
        for (file_id, json, face_id, suggested) in crate::t_sqlite::Face::boxes_for_person(person).unwrap_or_default() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
                map.entry(file_id).or_insert(FaceBox { bbox: v, face_id, mark: if suggested { Mark::Suggested } else { Mark::Confirmed } });
            }
        }
        let ignored = crate::t_sqlite::open_conn().and_then(|c| crate::t_face_suggest::ignored_for_person(&c, person)).unwrap_or_default();
        for (file_id, face_id, json) in ignored {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
                map.entry(file_id).or_insert(FaceBox { bbox: v, face_id, mark: Mark::Dimmed });
            }
        }
        let map = Arc::new(map);
        self.boxes.lock().unwrap().insert(person, map.clone());
        map
    }

    /// How this person's face on this file is drawn: the page's state once it has the
    /// person, else the database's.
    fn mark(&self, person: i64, file_id: i64) -> Mark {
        if let Some(m) = page_marks().lock().unwrap().get(&person) {
            return if m.dimmed.contains(&file_id) { Mark::Dimmed }
                else if m.suggested.contains(&file_id) { Mark::Suggested } else { Mark::Confirmed };
        }
        self.person_boxes(person).get(&file_id).map(|b| b.mark).unwrap_or(Mark::Confirmed)
    }

    /// A person section's (confirmed, pending suggestions) for its header.
    fn person_counts(&self, person: i64, rows: i64) -> (i64, i64) {
        let (suggested, dimmed) = match page_marks().lock().unwrap().get(&person) {
            Some(m) => (m.suggested.len() as i64, m.dimmed.len() as i64),
            None => (self.person_boxes(person).values().filter(|b| b.mark == Mark::Suggested).count() as i64, 0),
        };
        ((rows - suggested - dimmed).max(0), suggested)
    }

    fn tile_size(&self, item: &Item) -> (u32, u32) {
        if item.person.is_some() { ((self.cell as f64 * FACE_ASPECT).round() as u32, self.cell) }
        else { (self.cell, (self.cell as f64 * 0.75).round() as u32) }
    }

    /// Where a tile's pixels come from, cheapest first: for a face the page's face cache, a crop
    /// of the cached thumbnail, else the original photo (then cached for both grids); for a
    /// folder photo the page's thumbnail, auto-enhanced as the page shows it.
    /// Without `allow_original` (the paint path) a face too small in the thumbnail is cut soft
    /// instead (second value true) and upgraded in the background (Picasa cuts faces from its
    /// thumbnails; decoding originals while painting once stalled a frame 22 s).
    fn source(&self, item: &Item, allow_original: bool) -> Option<(Source, bool)> {
        match item.person {
            Some(person) => {
                let boxes = self.person_boxes(person);
                let FaceBox { bbox: b, face_id, .. } = boxes.get(&item.file_id)?;
                let bbox = ["x", "y", "width", "height"].map(|k| b[k].as_f64().unwrap_or(0.0));
                let aspect = if item.rotate.rem_euclid(180) == 90 { 1.0 / FACE_ASPECT } else { FACE_ASPECT };
                let key = crate::t_face_thumb::face_key(&self.library_id, item.file_id, b, aspect, *face_id,
                    self.face_size, &item.path, item.version, item.edits.as_deref()).ok()?;
                if let Some(bytes) = crate::t_thumb_cache::get_face(&key) { return Some((Source::Bytes(bytes), false)); }
                if let Some(img) = crate::t_face_thumb::cached_thumbnail_crop(item.file_id, bbox, self.face_size, aspect, *face_id, false).ok().flatten() {
                    return Some((Source::Image(img), false));
                }
                if !allow_original {
                    if let Some(img) = crate::t_face_thumb::cached_thumbnail_crop(item.file_id, bbox, self.face_size, aspect, *face_id, true).ok().flatten() {
                        return Some((Source::Image(img), true));
                    }
                }
                // No usable thumbnail crop at all (an edited photo, an odd thumbnail): even the
                // paint decodes the original, so no tile is ever left blank. Rare, unlike the
                // small faces above that used to make whole frames wait on originals.
                trace(format_args!("face from original file={} person={person}", item.file_id));
                let bytes = crate::t_face_thumb::generate_from_original(&item.path, item.file_id, bbox, self.face_size, aspect, *face_id).ok()?;
                let _ = crate::t_thumb_cache::put_face(&key, &bytes);
                Some((Source::Bytes(bytes), false))
            }
            None => {
                if crate::t_edit_recipe::recipe(item.edits.as_deref()).is_some() {
                    if let Some(bytes) = crate::t_edit_recipe::render_thumbnail_sync(item.file_id) {
                        return Some((Source::Bytes(bytes), false));
                    }
                }
                let mut bytes = crate::t_protocol::grid_thumbnail(item.file_id, &self.library_id, &mut |_, _| {})?;
                if crate::t_enhance::is_file_auto_enhanced(item.file_id) {
                    bytes = crate::t_enhance::apply_auto_enhance_to_bytes_for_file(&bytes, Some(item.file_id)).unwrap_or(bytes);
                }
                Some((Source::Bytes(bytes), false))
            }
        }
    }

    /// Make sure a tile's source is cached (a face cut from the original is the slow part)
    /// without keeping its pixels.
    fn warm(&self, item: &Item) -> bool {
        match item.person {
            Some(_) => self.source(item, true).is_some(),
            None => crate::t_protocol::grid_thumbnail(item.file_id, &self.library_id, &mut |_, _| {}).is_some(),
        }
    }

    /// Decode one tile (face crop fitted to the cell, or the whole photo fitted inside it);
    /// `allow_original` as for `source`.
    fn decode(&self, item: &Item, allow_original: bool) -> Option<Tile> {
        let (w, h) = self.tile_size(item);
        let quarter = item.rotate.rem_euclid(180) == 90;
        let (source, soft) = self.source(item, allow_original)?;
        let img = match source {
            Source::Bytes(bytes) => image::load_from_memory(&bytes).ok()?,
            Source::Image(img) => img,
        };
        let (bw, bh) = if quarter { (h, w) } else { (w, h) };
        let img = if item.person.is_some() { img.resize_exact(bw, bh, image::imageops::FilterType::Triangle) }
            else { img.resize(bw, bh, image::imageops::FilterType::Triangle) };
        let img = match item.rotate.rem_euclid(360) { 90 => img.rotate90(), 180 => img.rotate180(), 270 => img.rotate270(), _ => img };
        let rgb = img.to_rgb8();
        let (iw, ih) = rgb.dimensions();
        // Whole photos are fitted inside the cell with the background around them.
        let mut px = vec![BACKGROUND; (w * h) as usize];
        let (ox, oy) = ((w.saturating_sub(iw)) / 2, (h.saturating_sub(ih)) / 2);
        for (x, y, p) in rgb.enumerate_pixels() {
            let (tx, ty) = (x + ox, y + oy);
            if tx < w && ty < h { px[(ty * w + tx) as usize] = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32; }
        }
        Some(Tile { w, h, px, soft })
    }

    /// Run `f` over items on up to one thread per core (at most `max_threads`).
    fn in_parallel(items: &[&Item], max_threads: usize, f: impl Fn(&Item) + Sync) {
        if items.is_empty() { return; }
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(max_threads).min(items.len()).max(1);
        std::thread::scope(|s| for chunk in items.chunks(items.len().div_ceil(threads)) {
            let f = &f;
            s.spawn(move || for item in chunk { f(item) });
        });
    }

    /// These items' tiles, decoding (in parallel) any that are not cached. Returned directly,
    /// so a tile evicted meanwhile still reaches the frame; one that cannot be decoded is left out.
    /// `allow_original` as for `source`.
    fn ensure_tiles(&self, items: &[Item], max_threads: usize, allow_original: bool) -> HashMap<(i64, i64), Arc<Tile>> {
        let key = |i: &Item| (i.file_id, i.person.unwrap_or(0));
        let mut got = HashMap::new();
        let mut missing: Vec<&Item> = Vec::new();
        {
            let mut tiles = self.tiles.lock().unwrap();
            let mut seen = std::collections::HashSet::new();
            for i in items {
                if !seen.insert(key(i)) { continue; }
                match tiles.get(&key(i)) { Some(t) => { got.insert(key(i), t); } None => missing.push(i) }
            }
        }
        let fresh = Mutex::new(Vec::new());
        Self::in_parallel(&missing, max_threads, |item| match self.decode(item, allow_original) {
            Some(t) => fresh.lock().unwrap().push((key(item), Arc::new(t))),
            // The paint path leaves these to the background upgrade.
            None if !allow_original => {}
            None => trace(format_args!("tile failed file={} person={:?}", item.file_id, item.person)),
        });
        let mut tiles = self.tiles.lock().unwrap();
        for (k, t) in fresh.into_inner().unwrap() { tiles.insert(k, t.clone()); got.insert(k, t); }
        got
    }

    /// Decode the tiles of these row ranges in the background, one prefetch at a time; a request
    /// while one runs is dropped (the next frame asks again for the band it then needs).
    fn prefetch(model: &Arc<Model>, ranges: Vec<(usize, usize)>) {
        use std::sync::atomic::Ordering;
        if ranges.is_empty() || model.prefetching.swap(true, Ordering::AcqRel) { return; }
        let model = model.clone();
        std::thread::spawn(move || {
            for &(start, n) in &ranges { model.load_rows(start, n); }
            let items: Vec<Item> = {
                let rows = model.rows.lock().unwrap();
                ranges.iter().flat_map(|&(start, n)| rows[start.min(rows.len())..(start + n).min(rows.len())].iter().flatten().cloned()).collect()
            };
            model.ensure_tiles(&items, usize::MAX, false);
            model.prefetching.store(false, Ordering::Release);
        });
    }


    /// Sharp versions of on-screen tiles that were drawn soft or not at all: decoded one at a time
    /// on a background thread (from the original photo if need be), each redrawn when ready.
    fn queue_upgrades(model: &Arc<Model>, items: &[Item]) {
        let pending: Vec<Item> = {
            let tiles = model.tiles.lock().unwrap();
            let failed = model.upgrade_failed.lock().unwrap();
            items.iter().filter(|i| i.person.is_some() && !failed.contains(&(i.file_id, i.person.unwrap_or(0)))
                && tiles.map.get(&(i.file_id, i.person.unwrap_or(0))).map(|(t, _)| t.soft).unwrap_or(true)).cloned().collect()
        };
        if pending.is_empty() { return; }
        {
            let mut queue = model.upgrades.lock().unwrap();
            for i in pending { if !queue.iter().any(|q| q.file_id == i.file_id && q.person == i.person) { queue.push(i); } }
        }
        if model.upgrading.swap(true, std::sync::atomic::Ordering::AcqRel) { return; }
        let model = model.clone();
        std::thread::spawn(move || {
            loop {
                if Arc::strong_count(&model) == 1 { break; } // view replaced
                let next = { let mut q = model.upgrades.lock().unwrap(); if q.is_empty() { None } else { Some(q.remove(0)) } };
                let Some(item) = next else { break };
                let key = (item.file_id, item.person.unwrap_or(0));
                match model.decode(&item, true) {
                    Some(t) => {
                        model.tiles.lock().unwrap().insert(key, Arc::new(t));
                        #[cfg(windows)] win::invalidate();
                    }
                    None => {
                        trace(format_args!("tile failed file={} person={:?}", item.file_id, item.person));
                        model.upgrade_failed.lock().unwrap().insert(key);
                    }
                }
            }
            model.upgrading.store(false, std::sync::atomic::Ordering::Release);
        });
    }

    /// After the view opens: every row, then a cached source for the start of every section, so a
    /// click on any section paints from caches (its pixels are decoded then, not kept from now).
    fn fill_in_background(model: Arc<Model>) {
        std::thread::spawn(move || {
            let mut start = 0;
            while start < model.total_rows {
                model.load_rows(start, ROW_CHUNK as usize);
                start += ROW_CHUNK as usize;
                if Arc::strong_count(&model) == 1 { return; } // view replaced
            }
            // Warm each section's first screen the way Picasa's scanner works: small batches
            // (16) on two threads, a 10 ms sleep after each, and no work at all while the user
            // is using the app, so the warm-up never competes with what is on screen.
            let replaced = || Arc::strong_count(&model) == 1;
            for g in &model.groups {
                let first = g.row_index as usize + 1;
                let n = (g.count as usize).min(WARM_PER_SECTION);
                let items: Vec<Item> = model.rows.lock().unwrap()[first..first + n].iter().flatten().cloned().collect();
                for batch in items.chunks(16) {
                    crate::scheduler::cooperative_yield_blocking_cancellable(replaced);
                    if replaced() { return; }
                    let batch: Vec<&Item> = batch.iter().collect();
                    Self::in_parallel(&batch, 2, |item| { model.warm(item); });
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
            trace(format_args!("fill done sections={}", model.groups.len()));
        });
    }
}

// Grid background. The factory pixel gate (lapcasa-native-people.py) reads flat blocks of this
// colour as undrawn tiles, so keep it in step with BACKGROUND there.
const BACKGROUND: u32 = 0x00eeeeee;
const SELECTED: u32 = 0x005b5bd6;

/// Test tools set LAP_NATIVE_GRID_LOG to a file path to get one line per event, each
/// stamped with Unix time in milliseconds.
fn trace(line: std::fmt::Arguments) {
    if let Ok(path) = std::env::var("LAP_NATIVE_GRID_LOG") {
        use std::io::Write;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{now:.1} {line}");
        }
    }
}

#[cfg(windows)]
mod win {
    use super::{GridRect, GroupedQueryGroup, Item, Mark, Model, QueryParams, WheelMotion, AHEAD_SCREENS, BACKGROUND, BEHIND_SCREENS, KEEP_SCREENS, SELECTED};
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::Arc;
    use windows::core::{w, PCWSTR};
    use windows::core::Interface;
    use windows::Win32::Foundation::{COLORREF, HMODULE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::DirectComposition::*;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::*;
    use windows::Win32::Graphics::Dxgi::*;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    // The grid's window (0 until first shown). Touched on the UI thread only.
    static GRID: AtomicIsize = AtomicIsize::new(0);

    struct View {
        model: Arc<Model>,
        scroll: f64,
        // Where wheel motion is heading (the prefetch band leads that way), and the motion.
        target: f64,
        motion: WheelMotion,
        // The clicked tile (file index), outlined.
        selected: Option<i64>,
        // Row index of the section reported to the page as at the top.
        section: i64,
    }
    thread_local! { static VIEW: RefCell<Option<View>> = const { RefCell::new(None) }; }

    const TIMER: usize = 1;

    /// Milliseconds on a steady clock, for wheel motion.
    fn now_ms() -> f64 {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0
    }

    fn hwnd() -> Option<HWND> {
        match GRID.load(Ordering::Relaxed) { 0 => None, h => Some(HWND(h as _)) }
    }

    pub fn place(parent: isize, rect: Option<GridRect>, holes: &[GridRect], scale: f64) {
        unsafe {
            let hwnd = match hwnd() { Some(h) => h, None => match create(parent) { Some(h) => h, None => { super::trace(format_args!("place: create failed")); return } } };
            super::trace(format_args!("place {:?} holes={:?}", rect.map(|r| (r.x, r.y, r.width, r.height)), holes.iter().map(|h| (h.x as i32, h.y as i32, h.width as i32, h.height as i32)).collect::<Vec<_>>()));
            match rect {
                Some(r) if r.width > 0.0 && r.height > 0.0 => {
                    let before = client_size();
                    let _ = SetWindowPos(hwnd, Some(HWND_TOP),
                        (r.x * scale).round() as i32, (r.y * scale).round() as i32,
                        (r.width * scale).round() as i32, (r.height * scale).round() as i32,
                        SWP_NOACTIVATE | SWP_SHOWWINDOW);
                    let px = |v: f64| (v * scale).round() as i32;
                    let holes: Vec<RECT> = holes.iter().map(|h| RECT { left: px(h.x), top: px(h.y),
                        right: px(h.x + h.width), bottom: px(h.y + h.height) }).collect();
                    let size = (px(r.width), px(r.height));
                    // A new size is a new layout (columns); the old frame must not stay up.
                    if HOLES.with(|c| *c.borrow() != holes) || client_size() != before || REGION_SIZE.with(|s| s.get() != size) {
                        // Cut the holes out of the window's region too, so a click there goes to the
                        // page underneath. HTTRANSPARENT alone only passes clicks to windows of this
                        // thread, and WebView2's input window is not one: buttons over the grid (the
                        // pinned bar's Confirm all, menus) could not be clicked (LAP-064).
                        let region = CreateRectRgn(0, 0, size.0, size.1);
                        for hole in &holes {
                            let cut = CreateRectRgn(hole.left, hole.top, hole.right, hole.bottom);
                            let _ = CombineRgn(Some(region), Some(region), Some(cut), RGN_DIFF);
                            let _ = DeleteObject(cut.into());
                        }
                        if holes.is_empty() {
                            let _ = DeleteObject(region.into());
                            let _ = SetWindowRgn(hwnd, None, true);
                        } else {
                            let _ = SetWindowRgn(hwnd, Some(region), true); // the system owns it now
                        }
                        REGION_SIZE.with(|s| s.set(size));
                        HOLES.with(|c| *c.borrow_mut() = holes);
                        repaint();
                    }
                }
                _ => { let _ = ShowWindow(hwnd, SW_HIDE); }
            }
        }
    }

    pub fn show(model: Arc<Model>) {
        VIEW.with(|v| {
            let mut v = v.borrow_mut();
            let anchor = ZOOM_ANCHOR.with(|z| z.borrow_mut().take()).filter(|(_, at)| at.elapsed().as_secs_f64() < 2.0).map(|(a, _)| a);
            let mut view = View { model, scroll: 0.0, target: 0.0, motion: WheelMotion::default(), selected: None, section: 0 };
            // The same query again keeps the user's place (LAP-082): the page reloads it after a
            // tick/x review with a fresh random seed and new counts, and after a resize or zoom
            // at another tile size. Anything else is a new view, from the top.
            let same = v.as_ref().is_some_and(|old| same_query(&old.model, &view.model));
            super::trace(format_args!("view same_query={same}"));
            if let Some(old) = v.as_mut().filter(|_| same) {
                let m = &view.model;
                if old.model.cell == m.cell {
                    // A reload at the same size (a review): the same pixel offset, the motion
                    // carried on, and the decoded tiles kept so nothing redraws empty.
                    *m.tiles.lock().unwrap() = old.model.tiles.lock().unwrap().clone();
                    view.scroll = old.scroll.min(max_scroll(m));
                    view.target = old.target.min(max_scroll(m));
                    view.motion = std::mem::take(&mut old.motion);
                    if !view.motion.active() { view.motion.reset(view.scroll); }
                    view.section = top_section(m, view.scroll);
                    super::trace(format_args!("reload kept scroll={:.1} section={}", view.scroll, view.section));
                } else {
                    // Another tile size (LAP-072): the tile under the pointer after a Ctrl+wheel
                    // zoom, else the same place within the same person or folder (by group id:
                    // its row index can move when counts change).
                    let group = section_of(&old.model, old.section);
                    let within = group.map(|g| (old.scroll - section_top(&old.model, old.section)) / section_height(&old.model, g).max(1.0)).unwrap_or(0.0);
                    let section = group.and_then(|g| m.groups.iter().find(|n| n.group_id == g.group_id)).map(|g| g.row_index).unwrap_or(0);
                    let y = match &anchor {
                        Some(a) => anchor_y(m, a) - a.y as f64,
                        None => section_top(m, section) + section_of(m, section).map(|g| within * section_height(m, g)).unwrap_or(0.0),
                    };
                    view.scroll = y.clamp(0.0, max_scroll(m));
                    view.target = view.scroll;
                    view.selected = old.selected;
                    view.motion.reset(view.scroll);
                    view.section = if anchor.is_some() { top_section(m, view.scroll) } else { section };
                    super::trace(format_args!("retile cell={} section={} scroll={:.1} anchored={}", m.cell, view.section, view.scroll, anchor.is_some()));
                }
                // The page forgets its top section on every load; tell it which one is on top.
                super::emit("native-grid-section", serde_json::json!({ "rowIndex": view.section }));
            }
            *v = Some(view);
        });
        repaint();
    }

    /// The same query (library and parameters) apart from the load's random seed, which the page
    /// sends afresh with every load.
    fn same_query(a: &Model, b: &Model) -> bool {
        // View > Hidden Pictures (show_hidden) only adds or drops hidden photos: the same view (LAP-050).
        let key = |p: &QueryParams| serde_json::to_value(QueryParams { random_seed: 0, show_hidden: None, ..p.clone() }).ok();
        a.library_id == b.library_id && key(&a.params) == key(&b.params)
    }

    fn section_of(m: &Model, row_index: i64) -> Option<&GroupedQueryGroup> {
        m.groups.iter().find(|g| g.row_index == row_index)
    }

    /// What is under the pointer when Ctrl+wheel zooms: a tile (its index in the section and how
    /// far down its row the pointer is), or a point in a section header; y = pointer client y.
    #[derive(Debug)]
    struct Anchor { section: i64, item: Option<usize>, within: f64, y: i32 }
    thread_local! { static ZOOM_ANCHOR: RefCell<Option<(Anchor, std::time::Instant)>> = const { RefCell::new(None) }; }

    fn anchor_at(m: &Model, scroll: f64, x: i32, y: i32) -> Option<Anchor> {
        let content = scroll + y as f64;
        let cols = columns(m);
        let mut top = 0.0;
        for g in &m.groups {
            let sh = section_height(m, g);
            if content < top + sh {
                let below = content - top - header_h(m);
                if below < 0.0 || g.count == 0 { return Some(Anchor { section: g.row_index, item: None, within: content - top, y }); }
                let rh = row_h(m, g);
                let row = (below / rh).floor();
                let col = ((x as f64 / m.pitch).floor().max(0.0) as usize).min(cols - 1);
                let item = (row as usize * cols + col).min(g.count as usize - 1);
                return Some(Anchor { section: g.row_index, item: Some(item), within: ((below - row * rh) / rh).min(1.0), y });
            }
            top += sh;
        }
        None
    }

    /// Content y of an anchor in model `m` (another tile size, so other columns and row heights).
    fn anchor_y(m: &Model, a: &Anchor) -> f64 {
        let top = section_top(m, a.section);
        match (a.item, section_of(m, a.section)) {
            (Some(i), Some(g)) => top + header_h(m) + ((i / columns(m)) as f64 + a.within) * row_h(m, g),
            _ => top + a.within,
        }
    }

    /// New rows for the same view: keep where the user is (clamped to the new length).
    pub fn replace(model: Arc<Model>, focus_row: Option<i64>) {
        VIEW.with(|v| {
            let mut v = v.borrow_mut();
            match v.as_mut() {
                Some(view) => {
                    view.scroll = view.scroll.min(max_scroll(&model));
                    view.target = view.target.min(max_scroll(&model));
                    view.model = model;
                }
                None => *v = Some(View { model, scroll: 0.0, target: 0.0, motion: WheelMotion::default(), selected: None, section: 0 }),
            }
        });
        match focus_row { Some(row) => scroll_to_row(row), None => repaint() }
    }

    pub fn scroll_to_row(row_index: i64) {
        super::trace(format_args!("scroll_to row={row_index}"));
        VIEW.with(|v| if let Some(view) = v.borrow_mut().as_mut() {
            // Calendar's date sections: a Timeline jump lands on a row inside its section. People and
            // Folders jump to sections, as before (section_top: the section starting at the row).
            let dated = super::is_date_group(view.model.params.group_by) && !view.model.params.people_all && !view.model.params.folders_all;
            let y = if dated { row_top(&view.model, row_index) } else { section_top(&view.model, row_index) };
            view.scroll = y.min(max_scroll(&view.model));
            view.target = view.scroll;
            view.motion.reset(view.scroll);
            view.section = if dated {
                view.model.groups.iter().take_while(|g| g.row_index <= row_index).last().map(|g| g.row_index).unwrap_or(row_index)
            } else { row_index };
            emit_scroll(view);
        });
        repaint();
    }

    /// The page's scrollbar dragged or paged (Build 46): the same place as a ratio of the scroll
    /// range; the section at the top is reported as a wheel would, so the sidebar follows.
    pub fn scroll_to_ratio(ratio: f64) {
        VIEW.with(|v| if let Some(view) = v.borrow_mut().as_mut() {
            view.scroll = (ratio.clamp(0.0, 1.0) * max_scroll(&view.model)).round();
            view.target = view.scroll;
            view.motion.reset(view.scroll);
            let top = top_section(&view.model, view.scroll);
            if top != view.section {
                view.section = top;
                super::emit("native-grid-section", serde_json::json!({ "rowIndex": top }));
            }
        });
        repaint();
    }

    thread_local! { static SCROLL_EMITTED: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) }; }
    /// Where the view is, for the page's scrollbar thumb: 0..1 of the scroll range.
    fn emit_scroll(view: &View) {
        let max = max_scroll(&view.model);
        SCROLL_EMITTED.with(|t| t.set(now_ms()));
        super::emit("native-grid-scroll", serde_json::json!({ "ratio": if max > 0.0 { view.scroll / max } else { 0.0 } }));
    }

    /// Ask for a repaint from any thread (no synchronous paint).
    pub fn invalidate() {
        if let Some(h) = hwnd() { unsafe { let _ = InvalidateRect(Some(h), None, false); } }
    }

    pub fn repaint() {
        if let Some(h) = hwnd() { unsafe { let _ = InvalidateRect(Some(h), None, false); let _ = UpdateWindow(h); } }
    }

    // ---- layout (physical px) ----
    fn gap(m: &Model) -> f64 { 20.0 * m.scale }
    fn header_h(m: &Model) -> f64 { 80.0 * m.scale }
    fn client_size() -> (i32, i32) {
        let mut r = RECT::default();
        if let Some(h) = hwnd() { unsafe { let _ = GetClientRect(h, &mut r); } }
        (r.right, r.bottom)
    }
    /// As many columns as the page's grid: its width over the page's item width (the tile is
    /// centred in each). The extra pixel absorbs the window width's rounding to whole pixels.
    fn columns(m: &Model) -> usize {
        let (w, _) = client_size();
        (((w as f64 + 1.0) / m.pitch).floor() as usize).max(1)
    }
    /// Height of the tick / x row under every face tile, as on the page (h-7).
    fn controls_h(m: &Model) -> f64 { 28.0 * m.scale }
    /// Row pitch of a section: face tiles are a full cell high plus their tick / x row,
    /// folder photos 4:3.
    fn row_h(m: &Model, g: &GroupedQueryGroup) -> f64 {
        let h = if m.is_person_group(&g.group_id).is_some() { m.cell as f64 + controls_h(m) } else { (m.cell as f64 * 0.75).round() };
        h + gap(m)
    }
    fn section_height(m: &Model, g: &GroupedQueryGroup) -> f64 {
        let count = g.count;
        let rows = (count as usize).div_ceil(columns(m));
        header_h(m) + rows as f64 * row_h(m, g) + gap(m)
    }
    fn section_top(m: &Model, row_index: i64) -> f64 {
        let mut y = 0.0;
        for g in &m.groups {
            if g.row_index >= row_index { break; }
            y += section_height(m, g);
        }
        y
    }
    /// Top of any row: a section's header, or the tile row holding an item row (a Timeline jump
    /// lands inside a section, LAP-091). For a header row this is section_top.
    fn row_top(m: &Model, row_index: i64) -> f64 {
        let mut y = 0.0;
        for g in &m.groups {
            if row_index <= g.row_index { return y; }
            if row_index <= g.row_index + g.count {
                let i = (row_index - g.row_index - 1).max(0) as usize;
                return y + header_h(m) + (i / columns(m)) as f64 * row_h(m, g);
            }
            y += section_height(m, g);
        }
        y
    }
    fn content_height(m: &Model) -> f64 { m.groups.iter().map(|g| section_height(m, g)).sum() }
    fn max_scroll(m: &Model) -> f64 { (content_height(m) - client_size().1 as f64).max(0.0) }

    /// Header row index of the section at the top of the view.
    fn top_section(m: &Model, scroll: f64) -> i64 {
        let mut y = 0.0;
        let mut top = m.groups.first().map(|g| g.row_index).unwrap_or(0);
        for g in &m.groups {
            if y > scroll + 8.0 { break; }
            top = g.row_index;
            y += section_height(m, g);
        }
        top
    }

    /// The tile under a client point.
    fn tile_at(m: &Model, scroll: f64, x: i32, y: i32) -> Option<Item> {
        let (_, tiles, _) = visible(m, scroll);
        tiles.into_iter().find(|(item, tx, ty)| {
            let (_, th) = m.tile_size(item);
            x >= *tx && x < tx + m.cell as i32 && y >= *ty && y < ty + th as i32
        })
            .map(|(item, _, _)| item)
    }

    /// Everything visible at `scroll`: headers (label, count, y, person), tiles (item, x, y) and
    /// the item row ranges (first row, count) they come from.
    #[allow(clippy::type_complexity)]
    fn visible(m: &Model, scroll: f64) -> (Vec<(String, i64, f64, Option<i64>)>, Vec<(Item, i32, i32)>, Vec<(usize, usize)>) {
        visible_in(m, scroll, client_size().1 as f64)
    }

    /// The same for any band of the content `h` px high from `scroll` down.
    #[allow(clippy::type_complexity)]
    fn visible_in(m: &Model, scroll: f64, h: f64) -> (Vec<(String, i64, f64, Option<i64>)>, Vec<(Item, i32, i32)>, Vec<(usize, usize)>) {
        let cols = columns(m);
        let (mut headers, mut tiles, mut ranges) = (Vec::new(), Vec::new(), Vec::new());
        let mut y = 0.0;
        for g in &m.groups {
            let sh = section_height(m, g);
            let rh = row_h(m, g);
            if y + sh >= scroll && y <= scroll + h {
                headers.push((g.label.clone(), g.count, y - scroll, m.is_person_group(&g.group_id)));
                let first_row = ((scroll - y - header_h(m)) / rh).floor().max(0.0) as usize;
                let last_row = (((scroll + h - y - header_h(m)) / rh).ceil().max(0.0) as usize)
                    .min((g.count as usize).div_ceil(cols));
                let (a, b) = (first_row * cols, (last_row * cols).min(g.count as usize));
                if a < b { ranges.push((g.row_index as usize + 1 + a, b - a)); }
                let rows = m.rows.lock().unwrap();
                for i in a..b {
                    if let Some(Some(item)) = rows.get(g.row_index as usize + 1 + i) {
                        let x = (m.pitch - m.cell as f64) / 2.0 + (i % cols) as f64 * m.pitch;
                        let ty = y - scroll + header_h(m) + (i / cols) as f64 * rh;
                        tiles.push((item.clone(), x as i32, ty as i32));
                    }
                }
            }
            y += sh;
            if y > scroll + h { break; }
        }
        (headers, tiles, ranges)
    }

    /// The composed frame: a 32-bit DIB section with its own DC. Kept between paints so a
    /// PrintWindow capture (WM_PRINT/WM_PRINTCLIENT) returns exactly what was last shown.
    struct Frame { dc: HDC, bmp: HBITMAP, bits: *mut u32, w: i32, h: i32 }
    thread_local! { static FRAME: RefCell<Option<Frame>> = const { RefCell::new(None) }; }

    /// WebView2 draws through DirectComposition, which DWM puts above anything a GDI child
    /// paints into the main window's surface. The grid presents through DirectComposition too,
    /// so it is composed above the page, with transparent holes where page pop-ups are.
    struct Swap {
        context: ID3D11DeviceContext,
        chain: IDXGISwapChain1,
        w: i32,
        h: i32,
        _composition: (IDCompositionDevice, IDCompositionTarget, IDCompositionVisual),
    }
    // Page elements over the grid (client px): drawn transparent and not hit-tested.
    thread_local! { static HOLES: RefCell<Vec<RECT>> = const { RefCell::new(Vec::new()) }; }
    thread_local! { static REGION_SIZE: std::cell::Cell<(i32, i32)> = const { std::cell::Cell::new((0, 0)) }; }
    thread_local! { static SWAP: RefCell<Option<Swap>> = const { RefCell::new(None) }; }

    unsafe fn frame(w: i32, h: i32) -> Option<(HDC, *mut u32)> { unsafe {
        FRAME.with(|f| {
            let mut f = f.borrow_mut();
            if f.as_ref().is_some_and(|f| f.w == w && f.h == h) { return f.as_ref().map(|f| (f.dc, f.bits)); }
            if let Some(old) = f.take() { let _ = DeleteDC(old.dc); let _ = DeleteObject(old.bmp.into()); }
            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w;
            bmi.bmiHeader.biHeight = -h;
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
            let dc = CreateCompatibleDC(None);
            SelectObject(dc, bmp.into());
            *f = Some(Frame { dc, bmp, bits: bits as *mut u32, w, h });
            Some((dc, bits as *mut u32))
        })
    } }

    unsafe fn swap_chain(hwnd: HWND, w: i32, h: i32) -> Option<()> { unsafe {
        SWAP.with(|s| {
            let mut s = s.borrow_mut();
            if s.is_none() {
                let mut device = None;
                let mut context = None;
                D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context)).ok()?;
                let device: ID3D11Device = device?;
                let dxgi: IDXGIDevice = device.cast().ok()?;
                let factory: IDXGIFactory2 = dxgi.GetAdapter().ok()?.GetParent().ok()?;
                let desc = DXGI_SWAP_CHAIN_DESC1 {
                    Width: w as u32, Height: h as u32, Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT, BufferCount: 2,
                    Scaling: DXGI_SCALING_STRETCH, SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                    AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED, ..Default::default()
                };
                let made = (|| -> windows::core::Result<_> {
                    let chain = factory.CreateSwapChainForComposition(&device, &desc, None)?;
                    let composition: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
                    let target = composition.CreateTargetForHwnd(hwnd, true)?;
                    let visual = composition.CreateVisual()?;
                    visual.SetContent(&chain)?;
                    target.SetRoot(&visual)?;
                    composition.Commit()?;
                    Ok((chain, (composition, target, visual)))
                })();
                let (chain, composition) = match made {
                    Ok(made) => made,
                    Err(e) => { super::trace(format_args!("composition failed: {e}")); return None; }
                };
                *s = Some(Swap { context: context?, chain, w, h, _composition: composition });
            }
            let swap = s.as_mut()?;
            if swap.w != w || swap.h != h {
                swap.chain.ResizeBuffers(0, w as u32, h as u32, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0)).ok()?;
                swap.w = w;
                swap.h = h;
            }
            Some(())
        })
    } }

    /// Put the composed frame on screen: through the swap chain, or plain GDI if that failed.
    unsafe fn present(hwnd: HWND, hdc: HDC, w: i32, h: i32) { unsafe {
        let Some((dc, bits)) = frame(w, h) else { return };
        let shown = swap_chain(hwnd, w, h).and_then(|_| SWAP.with(|s| {
            let s = s.borrow();
            let swap = s.as_ref()?;
            let buffer: ID3D11Texture2D = swap.chain.GetBuffer(0).ok()?;
            swap.context.UpdateSubresource(&buffer, 0, None, bits as *const _, (w * 4) as u32, 0);
            swap.chain.Present(0, DXGI_PRESENT(0)).ok().ok()
        }));
        if shown.is_none() { let _ = BitBlt(hdc, 0, 0, w, h, Some(dc), 0, 0, SRCCOPY); }
    } }

    /// Review controls drawn over the pictures, as the page draws them on its tiles and
    /// person headers.
    enum Overlay {
        Face { x: i32, y: i32, w: i32, h: i32, person: i64, file_id: i64, mark: Mark },
        Header { y: i32, person: i64, label: String, suggestions: i64 },
    }

    /// What a click on a drawn control does (sent to the page, which owns the review).
    enum Hit {
        Face { person: i64, file_id: i64, action: &'static str },
        Header { person: i64, label: String, action: &'static str },
    }
    // Controls of the last composed frame (client px).
    thread_local! { static HITS: RefCell<Vec<(RECT, Hit)>> = const { RefCell::new(Vec::new()) }; }

    /// COLORREF from 0xRRGGBB.
    fn rgb(c: u32) -> COLORREF { COLORREF((c & 0xff) << 16 | (c & 0xff00) | (c >> 16 & 0xff)) }

    unsafe fn fill(dc: HDC, r: RECT, color: u32) { unsafe {
        let brush = CreateSolidBrush(rgb(color));
        FillRect(dc, &r, brush);
        let _ = DeleteObject(brush.into());
    } }

    unsafe fn frame_rect(dc: HDC, r: RECT, color: u32) { unsafe {
        let brush = CreateSolidBrush(rgb(color));
        FrameRect(dc, &r, brush);
        let _ = DeleteObject(brush.into());
    } }

    unsafe fn font(px: i32, bold: bool) -> HFONT { unsafe {
        CreateFontW(-px, 0, 0, 0, if bold { 700 } else { 400 }, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, w!("Arial"))
    } }

    /// Draw `text` centred in `r` (or right-aligned when `right`), in the given font and colour.
    unsafe fn text(dc: HDC, text: &str, mut r: RECT, px: i32, bold: bool, color: u32, right: bool) { unsafe {
        let f = font(px, bold);
        let old = SelectObject(dc, f.into());
        SetTextColor(dc, rgb(color));
        let mut t: Vec<u16> = text.encode_utf16().collect();
        let align = if right { DT_RIGHT } else { DT_CENTER };
        DrawTextW(dc, &mut t, &mut r, align | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, old);
        let _ = DeleteObject(f.into());
    } }

    unsafe fn text_width(dc: HDC, text: &str, px: i32, bold: bool) -> i32 { unsafe {
        let f = font(px, bold);
        let old = SelectObject(dc, f.into());
        let t: Vec<u16> = text.encode_utf16().collect();
        let mut size = windows::Win32::Foundation::SIZE::default();
        let _ = GetTextExtentPoint32W(dc, &t, &mut size);
        SelectObject(dc, old);
        let _ = DeleteObject(f.into());
        size.cx
    } }

    unsafe fn line(dc: HDC, points: &[(f64, f64)], width: i32, color: u32) { unsafe {
        let pen = CreatePen(PS_SOLID, width, rgb(color));
        let old = SelectObject(dc, pen.into());
        for (i, &(x, y)) in points.iter().enumerate() {
            if i == 0 { let _ = MoveToEx(dc, x.round() as i32, y.round() as i32, None); }
            else { let _ = LineTo(dc, x.round() as i32, y.round() as i32); }
        }
        SelectObject(dc, old);
        let _ = DeleteObject(pen.into());
    } }

    /// Orange "?" disc of diameter `d` at (x, y): a pending suggestion.
    unsafe fn question_disc(dc: HDC, x: i32, y: i32, d: i32, ring: i32, s: f64) { unsafe {
        let brush = CreateSolidBrush(rgb(0xf97316));
        let pen = CreatePen(PS_SOLID, ring, rgb(0xffffff));
        let (ob, op) = (SelectObject(dc, brush.into()), SelectObject(dc, pen.into()));
        let _ = Ellipse(dc, x, y, x + d, y + d);
        SelectObject(dc, ob);
        SelectObject(dc, op);
        let _ = DeleteObject(brush.into());
        let _ = DeleteObject(pen.into());
        text(dc, "?", RECT { left: x, top: y, right: x + d, bottom: y + d }, (d as f64 * 0.65).round().max(8.0 * s) as i32, true, 0xffffff, false);
    } }

    /// A small push button (the page's face-review-button): light face, grey border.
    unsafe fn push_button(dc: HDC, r: RECT, face: u32, border: u32) { unsafe {
        fill(dc, r, face);
        frame_rect(dc, r, border);
    } }

    fn hit(r: RECT, h: Hit) { HITS.with(|hits| hits.borrow_mut().push((r, h))); }

    unsafe fn draw_overlay(dc: HDC, overlay: Overlay, s: f64, header_h: i32, controls_h: i32) { unsafe {
        let px = |v: f64| (v * s).round() as i32;
        match overlay {
            Overlay::Face { x, y, w, h, person, file_id, mark } => {
                let strip = y + h;
                match mark {
                    Mark::Suggested => {
                        let d = px(16.0);
                        question_disc(dc, x + px(4.0), y + h - px(4.0) - d, d, px(1.0).max(1), s);
                        // Tick confirms, x ignores (page: two 24 x 20 buttons, 8 apart).
                        let (bw, bh, gap) = (px(24.0), px(20.0), px(8.0));
                        let bx = x + (w - (2 * bw + gap)) / 2;
                        let by = strip + (controls_h - bh) / 2;
                        let tick = RECT { left: bx, top: by, right: bx + bw, bottom: by + bh };
                        let cross = RECT { left: bx + bw + gap, top: by, right: bx + 2 * bw + gap, bottom: by + bh };
                        push_button(dc, tick, 0xf2f2f2, 0xaaaaaa);
                        push_button(dc, cross, 0xf2f2f2, 0xaaaaaa);
                        let (cx, cy) = (bx as f64 + bw as f64 / 2.0, by as f64 + bh as f64 / 2.0);
                        line(dc, &[(cx - 4.5 * s, cy), (cx - 1.5 * s, cy + 3.0 * s), (cx + 4.5 * s, cy - 3.5 * s)], px(2.0).max(1), 0x15803d);
                        let (kx, ky) = (cross.left as f64 + bw as f64 / 2.0, cy);
                        line(dc, &[(kx - 3.5 * s, ky - 3.5 * s), (kx + 3.5 * s, ky + 3.5 * s)], px(2.0).max(1), 0x991b1b);
                        line(dc, &[(kx + 3.5 * s, ky - 3.5 * s), (kx - 3.5 * s, ky + 3.5 * s)], px(2.0).max(1), 0x991b1b);
                        hit(tick, Hit::Face { person, file_id, action: "confirm-person" });
                        hit(cross, Hit::Face { person, file_id, action: "exclude-person" });
                    }
                    Mark::Dimmed => {
                        let (size, bh) = (px(12.0), px(24.0));
                        let bw = text_width(dc, "Undo", size, false) + px(16.0);
                        let bx = x + (w - bw) / 2;
                        let by = strip + (controls_h - bh) / 2;
                        let r = RECT { left: bx, top: by, right: bx + bw, bottom: by + bh };
                        push_button(dc, r, 0xf3f4f6, 0x9ca3af);
                        text(dc, "Undo", r, size, false, 0x374151, false);
                        hit(r, Hit::Face { person, file_id, action: "undo" });
                    }
                    Mark::Confirmed => {}
                }
            }
            Overlay::Header { y, person, label, suggestions } => {
                let (cw, _) = client_size();
                let right = cw - px(8.0);
                let (small, size) = (px(11.0), px(12.0));
                let row_h = px(28.0);
                let top = y + (header_h - (px(14.0) + px(2.0) + row_h)) / 2;
                let count = format!("{suggestions} {}", if suggestions == 1 { "suggestion" } else { "suggestions" });
                text(dc, &count, RECT { left: right - px(300.0), top, right, bottom: top + px(14.0) }, small, false, 0xc2410c, true);
                let row = top + px(16.0);
                let gap = px(4.0);
                // Right to left: Ignore all, Confirm all, review this person's suggestions.
                let iw = text_width(dc, "Ignore all", size, false) + px(16.0);
                let ignore = RECT { left: right - iw, top: row + px(2.0), right, bottom: row + px(26.0) };
                text(dc, "Ignore all", ignore, size, false, 0x374151, false);
                let cw2 = text_width(dc, "Confirm all", size, false) + px(24.0);
                let confirm = RECT { left: ignore.left - gap - cw2, top: row, right: ignore.left - gap, bottom: row + row_h };
                push_button(dc, confirm, 0xea580c, 0xea580c);
                text(dc, "Confirm all", confirm, size, false, 0xffffff, false);
                let review = RECT { left: confirm.left - gap - px(30.0), top: row + px(2.0), right: confirm.left - gap, bottom: row + px(26.0) };
                frame_rect(dc, review, 0xd4d4d4);
                let d = px(14.0);
                question_disc(dc, review.left + (px(30.0) - d) / 2, review.top + (px(24.0) - d) / 2, d, 1, s);
                hit(ignore, Hit::Header { person, label: label.clone(), action: "ignore-all" });
                hit(confirm, Hit::Header { person, label: label.clone(), action: "confirm-all" });
                hit(review, Hit::Header { person, label, action: "review" });
            }
        }
    } }

    /// The control under a client point, if any.
    fn control_at(x: i32, y: i32) -> Option<serde_json::Value> {
        HITS.with(|hits| hits.borrow().iter().find(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom).map(|(_, h)| match h {
            Hit::Face { person, file_id, action } => serde_json::json!({ "kind": "face", "personId": person, "fileId": file_id, "action": action }),
            Hit::Header { person, label, action } => serde_json::json!({ "kind": "header", "personId": person, "name": label, "action": action }),
        }))
    }

    /// Compose the whole visible grid into the frame. Returns its size.
    unsafe fn compose() -> Option<(i32, i32)> { unsafe {
        let started = std::time::Instant::now();
        let (w, h) = client_size();
        if w <= 0 || h <= 0 { return None; }
        let (dc, bits) = frame(w, h)?;
        let buf = std::slice::from_raw_parts_mut(bits, (w * h) as usize);
        buf.fill(BACKGROUND);
        let mut labels = Vec::new();
        let mut overlays = Vec::new();
        let (mut scale, mut header, mut controls) = (1.0, 0, 0);
        // For the trace: scroll offset, tiles that belong on screen, tiles drawn.
        let (mut offset, mut expected, mut drawn, mut cell, mut cols) = (0.0, 0usize, 0usize, 0u32, 0usize);
        let (mut cached, mut cached_mb) = (0usize, 0.0);
        VIEW.with(|v| {
            let view = v.borrow();
            let Some(view) = view.as_ref() else { return };
            let m = &view.model;
            cell = m.cell;
            cols = columns(m);
            // Complete frame: fetch missing rows and decode missing tiles first.
            let (_, _, ranges) = visible(m, view.scroll);
            for (start, n) in ranges { m.load_rows(start, n); }
            let (headers, tiles, ranges) = visible(m, view.scroll);
            offset = view.scroll;
            expected = ranges.iter().map(|(_, n)| n).sum();
            let items: Vec<Item> = tiles.iter().map(|(i, _, _)| i.clone()).collect();
            // The cache holds the screen and the bands around it (at the shortest row pitch).
            let pitch = (m.cell as f64 * 0.75).round() + gap(m);
            let screen = columns(m) * ((h as f64 / pitch).ceil() as usize + 1);
            m.tiles.lock().unwrap().capacity = screen * KEEP_SCREENS;
            let cache = m.ensure_tiles(&items, usize::MAX, false);
            Model::queue_upgrades(m, &items);
            // Prefetch the band ahead of the scroll (and behind it), so the next frames draw from memory.
            let down = view.target >= view.scroll;
            let (ahead, behind) = (AHEAD_SCREENS * h as f64, BEHIND_SCREENS * h as f64);
            let (lo, hi) = if down { (view.scroll - behind, view.scroll + h as f64 + ahead) } else { (view.scroll - ahead, view.scroll + h as f64 + behind) };
            let band = visible_in(m, lo.max(0.0), hi - lo.max(0.0)).2;
            let wanted = {
                let rows = m.rows.lock().unwrap();
                let t = m.tiles.lock().unwrap();
                band.iter().any(|&(s, n)| rows[s.min(rows.len())..(s + n).min(rows.len())].iter().flatten()
                    .any(|i| !t.map.contains_key(&(i.file_id, i.person.unwrap_or(0)))))
            };
            if wanted { Model::prefetch(&view.model, band); }
            (cached, cached_mb) = { let t = m.tiles.lock().unwrap(); (t.map.len(), t.bytes() as f64 / 1048576.0) };
            for (item, x, y) in &tiles {
                let Some(t) = cache.get(&(item.file_id, item.person.unwrap_or(0))) else { continue };
                drawn += 1;
                let mark = item.person.map(|p| m.mark(p, item.file_id));
                let tx = *x + ((m.cell as i32 - t.w as i32) / 2).max(0);
                for row in 0..t.h as i32 {
                    let py = *y + row;
                    if py < 0 || py >= h { continue; }
                    let x0 = tx.max(0);
                    let x1 = (tx + t.w as i32).min(w);
                    if x0 >= x1 { continue; }
                    let src = (row as u32 * t.w + (x0 - tx) as u32) as usize;
                    let dst = (py * w + x0) as usize;
                    let line = &mut buf[dst..dst + (x1 - x0) as usize];
                    line.copy_from_slice(&t.px[src..src + (x1 - x0) as usize]);
                    // Hidden, shown with View > Hidden Pictures: half faded, as the page draws it (50%).
                    if item.hidden {
                        for p in line.iter_mut() {
                            let half = |shift: u32| (((*p >> shift & 0xff) + (BACKGROUND >> shift & 0xff)) / 2) << shift;
                            *p = half(16) | half(8) | half(0);
                        }
                    }
                    // Ignored just now: grey and faded, as the page draws it (grayscale, 30%).
                    if mark == Some(Mark::Dimmed) {
                        for p in line.iter_mut() {
                            let grey = ((*p >> 16 & 0xff) * 30 + (*p >> 8 & 0xff) * 59 + (*p & 0xff) * 11) / 100;
                            let v = ((BACKGROUND & 0xff) * 7 + grey * 3) / 10;
                            *p = v << 16 | v << 8 | v;
                        }
                    }
                }
                if let (Some(person), Some(mark)) = (item.person, mark) {
                    if mark != Mark::Confirmed {
                        overlays.push(Overlay::Face { x: tx, y: *y, w: t.w as i32, h: t.h as i32, person, file_id: item.file_id, mark });
                    }
                }
                if view.selected == Some(item.file_index) {
                    let b = (3.0 * m.scale).round() as i32;
                    let (l, t2, r, btm) = (tx - b, *y - b, tx + t.w as i32 + b, *y + t.h as i32 + b);
                    for py in t2.max(0)..btm.min(h) {
                        for px in l.max(0)..r.min(w) {
                            if py < *y || py >= *y + t.h as i32 || px < tx || px >= tx + t.w as i32 {
                                buf[(py * w + px) as usize] = SELECTED;
                            }
                        }
                    }
                }
            }
            for (label, count, y, person) in headers {
                // A person's header counts confirmed photos and, like the page, turns orange
                // with the review buttons while the person has suggestions.
                let (count, suggestions) = match person { Some(p) => m.person_counts(p, count), None => (count, 0) };
                let top = (y as i32).max(0);
                let bottom = ((y + super::Model::header_px(m)) as i32).min(h);
                let fill = if suggestions > 0 { 0x00fff7ed } else { 0x00f7f7f7 };
                for py in top..bottom { buf[(py * w) as usize..((py + 1) * w) as usize].fill(fill); }
                if suggestions > 0 {
                    for py in [y as i32, (y + super::Model::header_px(m)) as i32 - 1] {
                        if py >= 0 && py < h { buf[(py * w) as usize..((py + 1) * w) as usize].fill(0x00fdba74); }
                    }
                    overlays.push(Overlay::Header { y: y as i32, person: person.unwrap_or(0), label: label.clone(), suggestions });
                }
                labels.push((label, count, y as i32, (16.0 * m.scale) as i32));
            }
            scale = m.scale;
            header = super::Model::header_px(m) as i32;
            controls = controls_h(m) as i32;
        });
        SetBkMode(dc, TRANSPARENT);
        HITS.with(|hits| hits.borrow_mut().clear());
        for overlay in overlays { draw_overlay(dc, overlay, scale, header, controls); }
        // Test tools click the controls by these rectangles (action:person:file:l,t,r,b).
        if std::env::var_os("LAP_NATIVE_GRID_LOG").is_some() {
            let list = HITS.with(|hits| hits.borrow().iter().map(|(r, hit)| {
                let (action, person, file) = match hit {
                    Hit::Face { person, file_id, action } => (*action, *person, *file_id),
                    Hit::Header { person, action, .. } => (*action, *person, 0),
                };
                format!("{action}:{person}:{file}:{},{},{},{}", r.left, r.top, r.right, r.bottom)
            }).collect::<Vec<_>>().join(" "));
            super::trace(format_args!("controls {list}"));
        }
        for (label, count, y, pad) in labels {
            let title = CreateFontW(-(28 * pad / 16), 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, w!("Georgia"));
            let old = SelectObject(dc, title.into());
            SetTextColor(dc, COLORREF(0x00303030));
            let t: Vec<u16> = label.encode_utf16().collect();
            let _ = TextOutW(dc, pad, y + pad, &t);
            let small = CreateFontW(-(12 * pad / 16), 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, w!("Arial"));
            SelectObject(dc, small.into());
            SetTextColor(dc, COLORREF(0x00808080));
            let c: Vec<u16> = format!("{count} {}", if count == 1 { "photo" } else { "photos" }).encode_utf16().collect();
            let _ = TextOutW(dc, pad, y + pad * 3, &c);
            SelectObject(dc, old);
            let _ = DeleteObject(title.into());
            let _ = DeleteObject(small.into());
        }
        let _ = GdiFlush();
        // GDI leaves alpha at 0: make everything opaque, then clear the holes (premultiplied).
        for p in buf.iter_mut() { *p |= 0xff00_0000; }
        HOLES.with(|holes| for r in holes.borrow().iter() {
            let (x0, x1) = (r.left.clamp(0, w), r.right.clamp(0, w));
            for y in r.top.clamp(0, h)..r.bottom.clamp(0, h) {
                buf[(y * w + x0) as usize..(y * w + x1) as usize].fill(0);
            }
        });
        super::trace(format_args!("compose {w}x{h} cell={cell} cols={cols} ms={:.1} scroll={offset:.1} visible={expected} drawn={drawn} cached={cached} cached_mb={cached_mb:.0}",
            started.elapsed().as_secs_f64() * 1000.0));
        Some((w, h))
    } }

    /// Copy the last composed frame into a capture DC (PrintWindow).
    unsafe fn print(hdc: HDC) { unsafe {
        FRAME.with(|f| if let Some(f) = f.borrow().as_ref() { let _ = BitBlt(hdc, 0, 0, f.w, f.h, Some(f.dc), 0, 0, SRCCOPY); });
    } }

    unsafe fn create(parent: isize) -> Option<HWND> { unsafe {
        let instance = GetModuleHandleW(PCWSTR::null()).ok()?;
        let class = w!("LapCasaNativeGrid");
        let wc = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(WS_EX_NOPARENTNOTIFY, class, w!(""), WS_CHILD | WS_CLIPSIBLINGS,
            0, 0, 0, 0, Some(HWND(parent as _)), None, Some(instance.into()), None).ok()?;
        GRID.store(hwnd.0 as isize, Ordering::Relaxed);
        Some(hwnd)
    } }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT { unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                if let Some((w, h)) = compose() { present(hwnd, hdc, w, h); }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            // Test tools capture the window with PrintWindow.
            WM_PRINT | WM_PRINTCLIENT => { print(HDC(wparam.0 as _)); LRESULT(0) }
            WM_ERASEBKGND => LRESULT(1),
            WM_MOUSEWHEEL => {
                crate::scheduler::record_interaction(crate::scheduler::InteractionSource::Scroll);
                let delta = ((wparam.0 >> 16) & 0xffff) as i16 as f64;
                // Ctrl+wheel zooms the tiles (Picasa): the page owns the size setting and steps it,
                // then shows the grid again at the new size around the tile under the pointer.
                if wparam.0 & 0x0008 != 0 {
                    let mut pt = POINT { x: (lparam.0 & 0xffff) as i16 as i32, y: ((lparam.0 >> 16) & 0xffff) as i16 as i32 };
                    let _ = ScreenToClient(hwnd, &mut pt);
                    VIEW.with(|v| if let Some(view) = v.borrow().as_ref() {
                        let anchor = anchor_at(&view.model, view.scroll, pt.x, pt.y);
                        super::trace(format_args!("zoom delta={delta} anchor={anchor:?}"));
                        ZOOM_ANCHOR.with(|z| *z.borrow_mut() = anchor.map(|a| (a, std::time::Instant::now())));
                    });
                    if delta != 0.0 { super::emit("native-grid-zoom", serde_json::json!({ "zoomIn": delta > 0.0 })); }
                    return LRESULT(0);
                }
                VIEW.with(|v| if let Some(view) = v.borrow_mut().as_mut() {
                    let step = super::DETENT_PX * view.model.scale;
                    let was_active = view.motion.active();
                    if !was_active { view.motion.reset(view.scroll); }
                    view.motion.add(-delta / 120.0 * step, now_ms(), max_scroll(&view.model), wparam.0 & 0x0004 != 0);
                    view.target = view.motion.destination();
                    let ease = view.motion.segments.last().map(|s| s.2).unwrap_or(0.0);
                    super::trace(format_args!("wheel delta={delta} px={:.1} ease_ms={ease:.0}", delta / 120.0 * step));
                    if !was_active && view.motion.active() { SetTimer(Some(hwnd), TIMER, 8, None); }
                });
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER => {
                let mut done = true;                   // no view: nothing left to move
                VIEW.with(|v| if let Some(view) = v.borrow_mut().as_mut() {
                    view.scroll = view.motion.sample(now_ms(), max_scroll(&view.model));
                    done = !view.motion.active();
                    let top = top_section(&view.model, view.scroll);
                    if top != view.section {
                        view.section = top;
                        super::emit("native-grid-section", serde_json::json!({ "rowIndex": top }));
                    }
                    // the page's scrollbar thumb follows the wheel, about 30 times a second
                    if done || now_ms() - SCROLL_EMITTED.with(|t| t.get()) >= 33.0 { emit_scroll(view); }
                });
                if done { let _ = KillTimer(Some(hwnd), TIMER); }
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                crate::scheduler::record_interaction(crate::scheduler::InteractionSource::Mouse);
                let (x, y) = ((lparam.0 & 0xffff) as i16 as i32, ((lparam.0 >> 16) & 0xffff) as i16 as i32);
                // Page menus close on a press outside them, as they do on the page.
                super::emit("native-grid-pointer", serde_json::json!({}));
                // Review controls: the page carries out the action. A double click on one
                // is not a second action, and does not open the photo.
                if let Some(control) = control_at(x, y) {
                    if msg == WM_LBUTTONDOWN { super::emit("native-grid-control", control); }
                    return LRESULT(0);
                }
                let ctrl = wparam.0 & 0x0008 != 0;
                let shift = wparam.0 & 0x0004 != 0;
                let hit = VIEW.with(|v| {
                    let mut v = v.borrow_mut();
                    let view = v.as_mut()?;
                    let index = tile_at(&view.model, view.scroll, x, y)?.file_index;
                    view.selected = Some(index);
                    Some(index)
                });
                if let Some(index) = hit {
                    if msg == WM_LBUTTONDBLCLK {
                        super::emit("native-grid-dblclick", serde_json::json!({ "fileIndex": index }));
                    } else {
                        super::emit("native-grid-click", serde_json::json!({ "fileIndex": index, "ctrl": ctrl, "shift": shift }));
                    }
                    repaint();
                }
                LRESULT(0)
            }
            // Right-click a tile: select it and let the page open its menu there.
            WM_RBUTTONDOWN => {
                let (x, y) = ((lparam.0 & 0xffff) as i16 as i32, ((lparam.0 >> 16) & 0xffff) as i16 as i32);
                super::emit("native-grid-pointer", serde_json::json!({}));
                let hit = VIEW.with(|v| {
                    let mut v = v.borrow_mut();
                    let view = v.as_mut()?;
                    let item = tile_at(&view.model, view.scroll, x, y)?;
                    let fresh = view.selected != Some(item.file_index);
                    view.selected = Some(item.file_index);
                    Some((item, fresh, view.model.scale))
                });
                if let Some((item, fresh, scale)) = hit {
                    super::emit("native-grid-context", serde_json::json!({ "fileIndex": item.file_index, "fileId": item.file_id,
                        "personId": item.person, "select": fresh, "x": x as f64 / scale, "y": y as f64 / scale }));
                    repaint();
                }
                LRESULT(0)
            }
            WM_NCHITTEST => {
                let mut pt = POINT { x: (lparam.0 & 0xffff) as i16 as i32, y: ((lparam.0 >> 16) & 0xffff) as i16 as i32 };
                let _ = ScreenToClient(hwnd, &mut pt);
                let in_hole = HOLES.with(|h| h.borrow().iter().any(|r| pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom));
                if in_hole { LRESULT(HTTRANSPARENT as isize) } else { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    } }
}

impl Model {
    fn header_px(m: &Model) -> f64 { 80.0 * m.scale }
}

#[cfg(test)]
mod date_label_tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn date_sections_read_as_the_page_writes_them_and_are_never_people() {
        let ts = |y, m, d| chrono::Local.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap().timestamp().to_string();
        assert_eq!(date_group_label(GROUP_DAY, &ts(2025, 7, 1)), "July 1st, 2025");
        assert_eq!(date_group_label(GROUP_DAY, &ts(2025, 7, 2)), "July 2nd, 2025");
        assert_eq!(date_group_label(GROUP_DAY, &ts(2025, 7, 11)), "July 11th, 2025");
        assert_eq!(date_group_label(GROUP_DAY, &ts(2025, 7, 23)), "July 23rd, 2025");
        assert_eq!(date_group_label(GROUP_MONTH, &ts(2025, 7, 1)), "July 2025");
        assert_eq!(date_group_label(GROUP_QUARTER, &ts(2025, 7, 1)), "2025 Q3 (Jul-Sep)");
        assert_eq!(date_group_label(GROUP_QUARTER, &ts(2001, 1, 1)), "2001 Q1 (Jan-Mar)");
        assert_eq!(date_group_label(GROUP_YEAR, &ts(2025, 1, 1)), "2025");
        assert_eq!(date_group_label(GROUP_QUARTER, "unknown-quarter"), "No date");
        assert!(is_date_group(GROUP_DAY) && is_date_group(GROUP_QUARTER) && !is_date_group(1) && !is_date_group(0));

        let group = |id: &str| GroupedQueryGroup { group_id: id.to_string(), label: id.to_string(), count: 3, size: 0, row_index: 0 };
        let mut calendar = QueryParams::default();
        calendar.group_by = GROUP_QUARTER;
        let m = Model::new(calendar, vec![group(&ts(2025, 7, 1))], 100, 120.0, 1.0, 128, String::new());
        assert_eq!(m.groups[0].label, "2025 Q3 (Jul-Sep)");
        assert_eq!(m.is_person_group(&m.groups[0].group_id), None, "a date section is not a person");
        let mut people = QueryParams::default();
        people.people_all = true;
        people.group_by = GROUP_DAY;             // People and Folders carry the stored group_by too
        let m = Model::new(people, vec![group("42")], 100, 120.0, 1.0, 128, String::new());
        assert_eq!(m.is_person_group("42"), Some(42));
        assert_eq!(m.groups[0].label, "42", "other sections keep their labels");
        let mut folders = QueryParams::default();
        folders.folders_all = true;
        folders.group_by = GROUP_DAY;
        let m = Model::new(folders, vec![group("C:/photos/2014")], 100, 120.0, 1.0, 128, String::new());
        assert_eq!(m.groups[0].label, "C:/photos/2014");
    }
}
