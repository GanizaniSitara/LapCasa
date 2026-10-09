//! Per-folder scan rules for the Folder Manager.
//!
//! Each rule pins a folder (and, by inheritance, its whole subtree) to one of:
//! - `always`  - included and kept in sync (the default under an album root)
//! - `once`    - included, scanned on request, skipped by background mtime sync
//! - `exclude` - "Remove from LapCasa": never scanned, already-indexed rows are
//!   hidden from every view (kept in the DB so re-including restores metadata).
//!   Files on disk are never touched.
//!
//! A folder's effective mode is the rule on the nearest ancestor-or-self path.
//! Paths are compared on a normalised key: `/` separators, no trailing slash,
//! ASCII-lowercased on Windows (matching SQLite's ASCII-only `lower()`, which the
//! query-side predicate uses).

use rusqlite::{params, Connection};
use serde::Serialize;
use std::fs;
use std::path::Path;

pub const MODE_ALWAYS: &str = "always";
pub const MODE_ONCE: &str = "once";
pub const MODE_EXCLUDE: &str = "exclude";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FolderRule {
    pub path: String,
    #[serde(skip)]
    pub key: String,
    pub mode: String,
    pub updated_at: i64,
}

impl FolderRule {
    pub fn new(path: &str, mode: &str) -> Self {
        Self { path: path.to_string(), key: normalize_key(path), mode: mode.to_string(), updated_at: 0 }
    }
}

/// Additive migration; safe to run on every open.
pub fn migrate(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS folder_rules (
            path_key TEXT PRIMARY KEY,
            path TEXT NOT NULL,
            mode TEXT NOT NULL CHECK (mode IN ('always', 'once', 'exclude')),
            updated_at INTEGER NOT NULL
        )",
    )
    .map_err(|e| format!("Failed to create folder_rules table: {}", e))
}

pub fn normalize_key(path: &str) -> String {
    let mut key: String = path
        .chars()
        .map(|c| {
            if c == '\\' {
                '/'
            } else if cfg!(windows) {
                c.to_ascii_lowercase()
            } else {
                c
            }
        })
        .collect();
    while key.ends_with('/') {
        key.pop();
    }
    key
}

/// SQL expression producing the normalised key of a path column.
fn sql_key(column: &str) -> String {
    let base = format!("rtrim(replace({column}, char(92), '/'), '/')");
    if cfg!(windows) {
        format!("lower({base})")
    } else {
        base
    }
}

/// SQL subquery: ids of `afolders` rows excluded by a rule, i.e. covered by an
/// exclude rule with no deeper always/once rule also covering them (the same
/// nearest-rule resolution as `effective_mode`). Not correlated, so SQLite
/// builds the set once per statement (measured ~3 ms on a 1.3k-folder library).
pub fn sql_excluded_folder_ids() -> String {
    let key = sql_key("rule_f.path");
    let covers = |rule: &str| {
        format!("({key} = {rule}.path_key OR substr({key}, 1, length({rule}.path_key) + 1) = {rule}.path_key || '/')")
    };
    format!(
        "SELECT rule_f.id FROM folder_rules rule_ex JOIN afolders rule_f ON {}
            WHERE rule_ex.mode = 'exclude' AND NOT EXISTS (SELECT 1 FROM folder_rules rule_in
                WHERE rule_in.mode != 'exclude' AND length(rule_in.path_key) > length(rule_ex.path_key) AND {})",
        covers("rule_ex"),
        covers("rule_in")
    )
}

/// SQL predicate: the folder with id `id_column` is excluded by a rule.
pub fn sql_folder_excluded(id_column: &str) -> String {
    format!("({id_column} IN ({}))", sql_excluded_folder_ids())
}

/// `key` is `ancestor` itself or lies below it.
fn key_within(ancestor: &str, key: &str) -> bool {
    key == ancestor
        || (key.len() > ancestor.len()
            && key.starts_with(ancestor)
            && key.as_bytes()[ancestor.len()] == b'/')
}

pub fn effective_rule<'a>(rules: &'a [FolderRule], path: &str) -> Option<&'a FolderRule> {
    let key = normalize_key(path);
    rules
        .iter()
        .filter(|rule| key_within(&rule.key, &key))
        .max_by_key(|rule| rule.key.len())
}

pub fn effective_mode<'a>(rules: &'a [FolderRule], path: &str) -> &'a str {
    effective_rule(rules, path).map(|rule| rule.mode.as_str()).unwrap_or(MODE_ALWAYS)
}

pub fn explicit_mode(rules: &[FolderRule], path: &str) -> Option<String> {
    let key = normalize_key(path);
    rules.iter().find(|rule| rule.key == key).map(|rule| rule.mode.clone())
}

/// Mode the path would have without its own rule.
pub fn inherited_mode<'a>(rules: &'a [FolderRule], path: &str) -> &'a str {
    let key = normalize_key(path);
    rules
        .iter()
        .filter(|rule| rule.key != key && key_within(&rule.key, &key))
        .max_by_key(|rule| rule.key.len())
        .map(|rule| rule.mode.as_str())
        .unwrap_or(MODE_ALWAYS)
}

pub fn is_excluded(rules: &[FolderRule], path: &str) -> bool {
    effective_mode(rules, path) == MODE_EXCLUDE
}

pub fn is_scan_once(rules: &[FolderRule], path: &str) -> bool {
    effective_mode(rules, path) == MODE_ONCE
}

/// Some strictly-deeper rule has `mode`.
pub fn has_mode_below(rules: &[FolderRule], path: &str, mode: &str) -> bool {
    let key = normalize_key(path);
    rules.iter().any(|rule| rule.mode == mode && rule.key != key && key_within(&key, &rule.key))
}

/// A filesystem walk may skip this whole subtree: it is excluded and no
/// deeper folder has been re-included.
pub fn prunes(rules: &[FolderRule], path: &str) -> bool {
    is_excluded(rules, path)
        && !has_mode_below(rules, path, MODE_ALWAYS)
        && !has_mode_below(rules, path, MODE_ONCE)
}

/// Background mtime sync should skip this folder (excluded or scan-once, with
/// no always-scanned folder beneath it).
pub fn skip_background_sync(rules: &[FolderRule], path: &str) -> bool {
    prunes(rules, path) || (is_scan_once(rules, path) && !has_mode_below(rules, path, MODE_ALWAYS))
}

/// Load all rules. A missing table (old or test schema) yields no rules.
pub fn load_rules(conn: &Connection) -> Vec<FolderRule> {
    let Ok(mut stmt) = conn.prepare("SELECT path, path_key, mode, updated_at FROM folder_rules") else {
        return Vec::new();
    };
    let rows = stmt.query_map([], |row| {
        Ok(FolderRule {
            path: row.get(0)?,
            key: row.get(1)?,
            mode: row.get(2)?,
            updated_at: row.get(3)?,
        })
    });
    match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

pub fn current_rules() -> Vec<FolderRule> {
    match crate::t_sqlite::open_conn() {
        Ok(conn) => load_rules(&conn),
        Err(_) => Vec::new(),
    }
}

/// Set (or with `None` clear) the rule on `path`. Explicit rules on its
/// descendants are cleared so the subtree follows the new choice, and a rule
/// equal to the inherited mode is not stored.
pub fn set_rule_on(conn: &Connection, path: &str, mode: Option<&str>, now: i64) -> Result<(), String> {
    if let Some(mode) = mode {
        if ![MODE_ALWAYS, MODE_ONCE, MODE_EXCLUDE].contains(&mode) {
            return Err(format!("Invalid folder mode: {}", mode));
        }
    }
    let key = normalize_key(path);
    if key.is_empty() && cfg!(windows) {
        return Err("Invalid folder path".to_string());
    }
    let rules = load_rules(conn);
    let inherited = inherited_mode(&rules, path).to_string();
    conn.execute(
        "DELETE FROM folder_rules WHERE path_key = ?1 OR substr(path_key, 1, length(?1) + 1) = ?1 || '/'",
        params![key],
    )
    .map_err(|e| e.to_string())?;
    if let Some(mode) = mode {
        if mode != inherited {
            conn.execute(
                "INSERT INTO folder_rules (path_key, path, mode, updated_at) VALUES (?1, ?2, ?3, ?4)",
                params![key, path, mode, now],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct FolderTreeEntry {
    pub name: String,
    pub path: String,
    pub mode: String,
    pub explicit_mode: Option<String>,
    pub has_subfolders: bool,
}

fn is_system_folder_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(lower.as_str(), "$recycle.bin" | "system volume information" | "recycler" | "@eadir" | "__macosx")
}

/// Direct subfolders of `path` read from disk (not the DB), with modes.
pub fn list_tree_level(rules: &[FolderRule], path: &str) -> Result<Vec<FolderTreeEntry>, String> {
    let mut raw_entries = Vec::new();
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if is_system_folder_name(&name) || crate::t_utils::is_fs_entry_hidden(&entry) {
            continue;
        }
        
        let metadata = entry.metadata().ok();
        let mtime = metadata.as_ref().and_then(|m| crate::t_utils::systemtime_to_timestamp(m.modified().ok()));
        
        let child = entry.path();
        let child_path = child.to_string_lossy().to_string();
        raw_entries.push((child, child_path, name, mtime));
    }
    
    let paths: Vec<String> = raw_entries.iter().map(|(_, p, _, _)| p.clone()).collect();
    let cache = crate::t_sqlite::FolderSubfolderState::get_many(&paths).unwrap_or_default();

    let mut entries = Vec::new();
    for (child, child_path, name, mtime) in raw_entries {
        let has_subfolders = match cache.get(&child_path) {
            Some(state) if state.modified_at == mtime || mtime.is_none() => state.has_subfolders,
            _ => crate::t_utils::detect_visible_subfolder(&child).unwrap_or(false),
        };

        entries.push(FolderTreeEntry {
            mode: effective_mode(rules, &child_path).to_string(),
            explicit_mode: explicit_mode(rules, &child_path),
            has_subfolders,
            name,
            path: child_path,
        });
    }
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    Ok(entries)
}

#[derive(Debug, Serialize)]
pub struct FolderModeState {
    pub path: String,
    pub mode: String,
    pub explicit_mode: Option<String>,
}

#[tauri::command]
pub async fn list_folder_tree(path: String) -> Result<Vec<FolderTreeEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if !Path::new(&path).is_dir() {
            return Err(format!("Folder is unavailable: {}", path));
        }
        let rules = current_rules();
        list_tree_level(&rules, &path)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn get_folder_modes(paths: Vec<String>) -> Vec<FolderModeState> {
    let rules = current_rules();
    paths
        .into_iter()
        .map(|path| FolderModeState {
            mode: effective_mode(&rules, &path).to_string(),
            explicit_mode: explicit_mode(&rules, &path),
            path,
        })
        .collect()
}

#[tauri::command]
pub fn get_folder_rules() -> Vec<FolderRule> {
    current_rules()
}

/// Set a folder's mode. Never touches files on disk, never removes albums.
#[tauri::command]
pub fn set_folder_rule(path: String, mode: Option<String>) -> Result<FolderModeState, String> {
    let conn = crate::t_sqlite::open_conn()?;
    migrate(&conn)?;
    let now = chrono::Utc::now().timestamp_millis();
    set_rule_on(&conn, &path, mode.as_deref(), now)?;
    let rules = load_rules(&conn);
    Ok(FolderModeState {
        mode: effective_mode(&rules, &path).to_string(),
        explicit_mode: explicit_mode(&rules, &path),
        path,
    })
}

/// Top-level entries for the Folder Manager's "whole computer" tree.
#[tauri::command]
pub fn list_drives() -> Vec<String> {
    if cfg!(windows) {
        (b'A'..=b'Z')
            .map(|letter| format!("{}:\\", letter as char))
            .filter(|drive| Path::new(drive).is_dir())
            .collect()
    } else {
        vec!["/".to_string()]
    }
}

#[derive(Debug, Serialize)]
pub struct AddedRoot {
    pub album: crate::t_sqlite::Album,
    pub merged_album_ids: Vec<i64>,
}

/// Folder Manager "Scan Always / Scan Once" on a folder outside LapCasa: make it
/// an album root (merging any albums nested inside it) and record the mode.
/// The caller queues the scan. Files on disk are never touched.
#[tauri::command]
pub async fn add_album_root(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, crate::t_cmds::IndexCancellation>,
    path: String,
    mode: String,
) -> Result<AddedRoot, String> {
    if ![MODE_ALWAYS, MODE_ONCE].contains(&mode.as_str()) {
        return Err(format!("Invalid mode for a new folder: {}", mode));
    }
    if !Path::new(&path).is_dir() {
        return Err(format!("Folder is unavailable: {}", path));
    }
    crate::t_utils::authorize_directory_scope(&app_handle, &path)
        .map_err(|e| format!("Error while authorizing folder '{}': {}", path, e))?;

    // Stop scans of albums that will be merged into the new root.
    let new_path = Path::new(&path);
    let nested: Vec<i64> = crate::t_sqlite::Album::get_all_albums()?
        .into_iter()
        .filter(|album| Path::new(&album.path).starts_with(new_path) && Path::new(&album.path) != new_path)
        .filter_map(|album| album.id)
        .collect();
    let _guards: Vec<_> = nested.iter().map(|id| crate::t_utils::AlbumRemovalGuard::acquire(*id)).collect();
    for id in &nested {
        state.0.lock().unwrap().insert(*id, true);
        crate::t_utils::wait_for_album_scan_end(*id).await?;
    }

    let (album, merged_album_ids) = crate::t_sqlite::Album::add_root_absorbing(&path)?;
    let conn = crate::t_sqlite::open_conn()?;
    migrate(&conn)?;
    // Only the root's own rule changes; rules already set below it are kept.
    let key = normalize_key(&album.path);
    if mode == MODE_ONCE {
        conn.execute(
            "INSERT OR REPLACE INTO folder_rules (path_key, path, mode, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params![key, album.path, MODE_ONCE, chrono::Utc::now().timestamp_millis()],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute("DELETE FROM folder_rules WHERE path_key = ?1", params![key])
            .map_err(|e| e.to_string())?;
    }
    Ok(AddedRoot { album, merged_album_ids })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(list: &[(&str, &str)]) -> Vec<FolderRule> {
        list.iter().map(|(path, mode)| FolderRule::new(path, mode)).collect()
    }

    #[test]
    fn default_is_always_and_nearest_rule_wins() {
        let r = rules(&[(r"C:\Photos\Old", MODE_EXCLUDE), (r"C:\Photos\Old\Keep", MODE_ALWAYS)]);
        assert_eq!(effective_mode(&r, r"C:\Photos"), MODE_ALWAYS);
        assert_eq!(effective_mode(&r, r"C:\Photos\New"), MODE_ALWAYS);
        assert_eq!(effective_mode(&r, r"C:\Photos\Old"), MODE_EXCLUDE);
        assert_eq!(effective_mode(&r, r"C:\Photos\Old\2001\Jan"), MODE_EXCLUDE);
        assert_eq!(effective_mode(&r, r"C:\Photos\Old\Keep"), MODE_ALWAYS);
        assert_eq!(effective_mode(&r, r"C:\Photos\Old\Keep\Sub"), MODE_ALWAYS);
        assert_eq!(inherited_mode(&r, r"C:\Photos\Old\Keep"), MODE_EXCLUDE);
        assert_eq!(explicit_mode(&r, r"C:\Photos\Old\Keep\Sub"), None);
    }

    #[test]
    fn sibling_prefix_is_not_an_ancestor() {
        let r = rules(&[(r"C:\Photos\Old", MODE_EXCLUDE)]);
        assert_eq!(effective_mode(&r, r"C:\Photos\Older"), MODE_ALWAYS);
        assert_eq!(effective_mode(&r, r"C:\Photos\Old-2"), MODE_ALWAYS);
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_are_case_and_separator_insensitive() {
        let r = rules(&[(r"C:\Photos\OLD\", MODE_EXCLUDE)]);
        assert!(is_excluded(&r, r"c:\photos\old"));
        assert!(is_excluded(&r, "C:/Photos/Old/x.jpg"));
        assert!(is_excluded(&r, r"C:\PHOTOS\Old\Sub"));
        assert!(!is_excluded(&r, r"C:\Photos"));
    }

    #[test]
    fn pruning_keeps_walking_into_reincluded_children() {
        let r = rules(&[(r"C:\P\Old", MODE_EXCLUDE), (r"C:\P\Old\Keep", MODE_ONCE)]);
        assert!(is_excluded(&r, r"C:\P\Old"));
        assert!(!prunes(&r, r"C:\P\Old"));
        assert!(prunes(&r, r"C:\P\Old\Other"));
        assert!(is_excluded(&r, r"C:\P\Old\file.jpg"));
        assert!(!is_excluded(&r, r"C:\P\Old\Keep\file.jpg"));
        assert!(skip_background_sync(&r, r"C:\P\Old\Keep"));
        assert!(!skip_background_sync(&r, r"C:\P"));
    }

    #[test]
    fn set_rule_clears_descendants_and_drops_redundant_rules() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        set_rule_on(&conn, r"C:\P\A\B", Some(MODE_EXCLUDE), 1).unwrap();
        set_rule_on(&conn, r"C:\P\A\C", Some(MODE_ONCE), 1).unwrap();
        assert_eq!(load_rules(&conn).len(), 2);
        // Excluding the parent makes the whole subtree follow it.
        set_rule_on(&conn, r"C:\P\A", Some(MODE_EXCLUDE), 2).unwrap();
        let r = load_rules(&conn);
        assert_eq!(r.len(), 1);
        assert!(is_excluded(&r, r"C:\P\A\C"));
        // Re-including the parent with the default mode stores nothing.
        set_rule_on(&conn, r"C:\P\A", Some(MODE_ALWAYS), 3).unwrap();
        assert!(load_rules(&conn).is_empty());
        assert!(set_rule_on(&conn, r"C:\P", Some("delete"), 4).is_err());
    }

    #[test]
    fn sql_predicate_matches_rust_resolution() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        set_rule_on(&conn, r"C:\P\Old", Some(MODE_EXCLUDE), 1).unwrap();
        set_rule_on(&conn, r"C:\P\Old\Keep", Some(MODE_ALWAYS), 1).unwrap();
        set_rule_on(&conn, r"C:\P\Old\Keep\Drop", Some(MODE_EXCLUDE), 1).unwrap();
        conn.execute_batch("CREATE TABLE afolders(id INTEGER PRIMARY KEY, path TEXT)").unwrap();
        let r = load_rules(&conn);
        assert_eq!(r.len(), 3);
        let paths = [r"C:\P", r"C:\P\Old", r"c:\p\old\x", r"C:\P\Old\Keep", r"C:\P\Old\Keep\y",
            r"C:\P\Old\Keep\Drop\z", r"C:\P\Older", r"C:\P\Old\"];
        for (id, path) in paths.iter().enumerate() {
            conn.execute("INSERT INTO afolders VALUES (?1, ?2)", params![id as i64, path]).unwrap();
        }
        for (id, path) in paths.iter().enumerate() {
            let excluded: bool = conn
                .query_row(&format!("SELECT {}", sql_folder_excluded("?1")), params![id as i64], |row| row.get(0))
                .unwrap();
            assert_eq!(excluded, is_excluded(&r, path), "{}", path);
        }
    }

    #[test]
    fn tree_listing_reads_disk_and_reports_modes() {
        let root = std::env::temp_dir().join(format!("lap_folder_rules_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("A").join("A1")).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join("file.jpg"), b"x").unwrap();
        let root_str = root.to_string_lossy().to_string();
        let r = vec![FolderRule::new(&root.join("b").to_string_lossy(), MODE_EXCLUDE)];
        let level = list_tree_level(&r, &root_str).unwrap();
        let names: Vec<_> = level.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["A", "b"]);
        assert!(level[0].has_subfolders);
        assert_eq!(level[0].mode, MODE_ALWAYS);
        assert_eq!(level[1].mode, MODE_EXCLUDE);
        assert_eq!(level[1].explicit_mode.as_deref(), Some(MODE_EXCLUDE));
        let _ = fs::remove_dir_all(&root);
    }
}
