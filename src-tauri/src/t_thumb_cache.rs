use redb::{Database, TableDefinition, ReadableTable, ReadableTableMetadata};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

const THUMBNAILS_TABLE: TableDefinition<i64, &[u8]> = TableDefinition::new("thumbnails");
const FACE_THUMBNAILS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("face_thumbnails");
const ORIENTATION_VERSIONS: TableDefinition<i64, u32> = TableDefinition::new("thumbnail_orientation_versions");
const PREVIEWS_TABLE: TableDefinition<i64, &[u8]> = TableDefinition::new("previews");
const PREVIEW_VERSIONS: TableDefinition<i64, u32> = TableDefinition::new("preview_versions");

static DB_INSTANCE: Mutex<Option<Arc<Database>>> = Mutex::new(None);
static CLOSING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static WRITE_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
static LAST_FACE_FLUSH: OnceLock<Mutex<std::time::Instant>> = OnceLock::new();
const FACE_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
// redb's own page cache is private memory and defaults to 1 GiB, so it grew with every face crop
// read while scrolling (LAP-076: ~63 KB per tile). Repeated reads are served by the OS file cache.
const CACHE_BYTES: usize = 64 << 20;

pub use crate::slabs;
pub use crate::slabs::SlabCache;

static SLAB_INSTANCE: Mutex<Option<Arc<SlabCache>>> = Mutex::new(None);

pub fn get_slab_cache() -> Option<Arc<SlabCache>> {
    if CLOSING.load(std::sync::atomic::Ordering::Acquire) {
        return None;
    }
    let mut guard = SLAB_INSTANCE.lock().ok()?;
    if let Some(slab) = guard.as_ref() {
        return Some(Arc::clone(slab));
    }
    let app_dir = crate::t_config::get_app_data_dir().ok()?;
    let slab_dir = app_dir.join("slabs");
    let cache = match SlabCache::open(&slab_dir) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            eprintln!("Failed to open slab cache at {}: {}", slab_dir.display(), e);
            return None;
        }
    };
    *guard = Some(Arc::clone(&cache));
    Some(cache)
}

pub fn set_slab_cache_for_testing(cache: Option<Arc<SlabCache>>) {
    if let Ok(mut guard) = SLAB_INSTANCE.lock() {
        *guard = cache;
    }
}

pub fn migrate_redb_to_slabs() -> Result<usize, String> {
    let slab = get_slab_cache().ok_or("Failed to initialize slab cache")?;
    let redb_path = get_db_path()?;
    slab.migrate_from_redb(&redb_path)
}

fn begin_cache_write(db: &Database) -> Result<redb::WriteTransaction, String> {
    let mut tx = db.begin_write().map_err(|e| e.to_string())?;
    // Save allocator state with each commit so an interrupted exit does not
    // require scanning gigabytes of cached photos and face crops on reopen.
    tx.set_quick_repair(true);
    Ok(tx)
}

fn get_db_path() -> Result<PathBuf, String> {
    let app_dir = crate::t_config::get_app_data_dir()?;
    if !app_dir.exists() {
        std::fs::create_dir_all(&app_dir).map_err(|e| e.to_string())?;
    }
    Ok(app_dir.join("thumbs.redb"))
}

fn get_db() -> Result<Arc<Database>, String> {
    // Concurrent first requests must wait for the shared handle, rather than
    // treating an exclusive-open failure as a cache miss and decoding photos.
    let mut instance = DB_INSTANCE.lock().map_err(|_| "Poisoned cache initialization lock".to_string())?;
    if CLOSING.load(std::sync::atomic::Ordering::Acquire) { return Err("Thumbnail cache is closing".into()); }
    if let Some(db) = instance.as_ref() { return Ok(Arc::clone(db)); }

    let mut trace = crate::t_protocol::ThumbnailTrace::new(0, "cache_open");
    let path = get_db_path()?;
    let db = Database::builder().set_cache_size(CACHE_BYTES).create(path).map_err(|e| format!("Failed to create redb database: {}", e))?;
    trace.mark("database", 0);

    // Ensure the thumbnails table exists
    {
        let write_tx = begin_cache_write(&db)?;
        {
            let _ = write_tx
                .open_table(THUMBNAILS_TABLE)
                .map_err(|e| format!("Failed to open thumbnails table: {}", e))?;
            let _ = write_tx.open_table(FACE_THUMBNAILS_TABLE).map_err(|e| e.to_string())?;
            let _ = write_tx.open_table(ORIENTATION_VERSIONS).map_err(|e| e.to_string())?;
            let _ = write_tx.open_table(PREVIEWS_TABLE).map_err(|e| e.to_string())?;
            let _ = write_tx.open_table(PREVIEW_VERSIONS).map_err(|e| e.to_string())?;
        }
        write_tx
            .commit()
            .map_err(|e| format!("Failed to commit table creation: {}", e))?;
    }

    let arc = Arc::new(db);
    *instance = Some(Arc::clone(&arc));
    Ok(arc)
}

/// Static values are not dropped on exit. Release the database explicitly so
/// redb writes its allocator checkpoint and clears its recovery flag normally.
pub fn shutdown() -> Result<(), String> {
    let mut trace = crate::t_protocol::ThumbnailTrace::new(0, "cache_shutdown");
    CLOSING.store(true, std::sync::atomic::Ordering::Release);
    if let Some(slab) = SLAB_INSTANCE.lock().ok().and_then(|mut g| g.take()) {
        let _ = slab.flush();
    }
    let db = DB_INSTANCE.lock().map_err(|e| e.to_string())?.take();
    if let Some(db) = db {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while Arc::strong_count(&db) > 1 {
            if std::time::Instant::now() >= deadline {
                return Err("Thumbnail cache still in use at shutdown; recovery may be needed".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Persist the last batch of derived crops before releasing the handle.
        begin_cache_write(&db)?.commit().map_err(|e| e.to_string())?;
        drop(db);
    }
    trace.mark("closed", 0);
    Ok(())
}

pub fn get_face(key: &str) -> Option<Vec<u8>> {
    let db = match get_db() {
        Ok(db) => db,
        Err(error) => {
            if cfg!(debug_assertions) && std::env::var_os("LAP_FACE_CACHE_TRACE").is_some() {
                eprintln!("{}",serde_json::json!({"event":"face_cache_open_error","error":error}));
            }
            return None;
        }
    };
    let tx = db.begin_read().ok()?;
    let table = tx.open_table(FACE_THUMBNAILS_TABLE).ok()?;
    let value = table.get(key).ok()??;
    Some(value.value().to_vec())
}

pub fn put_face(key: &str, data: &[u8]) -> Result<(), String> {
    let db = get_db()?;
    let _guard = WRITE_MUTEX.get_or_init(|| Mutex::new(())).lock().map_err(|e| e.to_string())?;
    let mut last_flush = LAST_FACE_FLUSH.get_or_init(|| Mutex::new(std::time::Instant::now()))
        .lock().map_err(|e| e.to_string())?;
    let durable = last_flush.elapsed() >= FACE_FLUSH_INTERVAL;
    put_face_bytes(&db, key, data, durable)?;
    if durable { *last_flush = std::time::Instant::now(); }
    Ok(())
}

fn put_face_bytes(db: &Database, key: &str, data: &[u8], durable: bool) -> Result<(), String> {
    let mut tx = begin_cache_write(db)?;
    // These bytes are reproducible cache entries. Keep them readable immediately,
    // but avoid two disk flushes per crop on the display path. Periodic durable
    // commits free pages and retain quick repair; shutdown flushes the final batch.
    if !durable { tx.set_durability(redb::Durability::None); }
    {
        let mut table = tx.open_table(FACE_THUMBNAILS_TABLE).map_err(|e| e.to_string())?;
        table.insert(key, data).map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

// The CLI owns an explicit cache while the app is closed; importing reuses the
// app's open database. Never open the same redb file twice in one process.
pub struct FaceCache(Arc<Database>);
impl FaceCache {
    pub fn open(path: &std::path::Path) -> Result<Self, String> {
        let db = Database::create(path).map_err(|e| e.to_string())?;
        let tx = begin_cache_write(&db)?;
        { tx.open_table(FACE_THUMBNAILS_TABLE).map_err(|e| e.to_string())?; }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(Self(Arc::new(db)))
    }
    pub fn current() -> Result<Self, String> { Ok(Self(get_db()?)) }
    pub fn contains(&self, key: &str) -> Result<bool, String> {
        let tx = self.0.begin_read().map_err(|e| e.to_string())?;
        let table = tx.open_table(FACE_THUMBNAILS_TABLE).map_err(|e| e.to_string())?;
        Ok(table.get(key).map_err(|e| e.to_string())?.is_some())
    }
    pub fn put_many(&self, entries: &[(String, Vec<u8>)]) -> Result<(), String> {
        if entries.is_empty() { return Ok(()); }
        let _guard = WRITE_MUTEX.get_or_init(|| Mutex::new(())).lock().map_err(|e| e.to_string())?;
        let tx = begin_cache_write(&self.0)?;
        {
            let mut table = tx.open_table(FACE_THUMBNAILS_TABLE).map_err(|e| e.to_string())?;
            for (key, bytes) in entries {
                // Preserve a crop completed by another worker/request.
                if table.get(key.as_str()).map_err(|e| e.to_string())?.is_none() {
                    table.insert(key.as_str(), bytes.as_slice()).map_err(|e| e.to_string())?;
                }
            }
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn migrate_face_keys(&self) -> Result<usize, String> {
        let tx = self.0.begin_read().map_err(|e| e.to_string())?;
        let table = tx.open_table(FACE_THUMBNAILS_TABLE).map_err(|e| e.to_string())?;
        let mut batch = Vec::new();
        let mut copied = 0;
        for entry in table.iter().map_err(|e| e.to_string())? {
            let (key, bytes) = entry.map_err(|e| e.to_string())?;
            let Some(uri) = key.value().strip_prefix("face-v4:") else { continue; };
            let Ok(key) = crate::t_face_thumb::cache_key(uri) else { continue; };
            if self.contains(&key)? { continue; }
            batch.push((key, bytes.value().to_vec())); copied += 1;
            if batch.len() == 32 { self.put_many(&batch)?; batch.clear(); }
        }
        self.put_many(&batch)?;
        Ok(copied)
    }
}

pub fn put(file_id: i64, data: &[u8]) -> Result<(), String> {
    if file_id <= 0 || data.is_empty() {
        return Ok(());
    }
    // Primary store: high-performance append-only slab cache (Picasa architecture, no double writes)
    if let Some(slab) = get_slab_cache() {
        return slab.put(file_id, data);
    }

    // Fallback only if slab cache could not be initialized
    let mut trace = crate::t_protocol::ThumbnailTrace::new(file_id, "cache_write");
    let db = get_db()?;
    trace.mark("open", data.len());
    let write_mutex = WRITE_MUTEX.get_or_init(|| Mutex::new(()));
    let _guard = write_mutex
        .lock()
        .map_err(|_| "Poisoned redb write lock".to_string())?;
    trace.mark("lock_wait", data.len());
    let write_tx = begin_cache_write(&db)?;
    {
        let mut table = write_tx
            .open_table(THUMBNAILS_TABLE)
            .map_err(|e| format!("Failed to open thumbnails table: {}", e))?;
        table
            .insert(file_id, data)
            .map_err(|e| format!("Failed to insert into thumbnails table: {}", e))?;
    }
    write_tx.open_table(ORIENTATION_VERSIONS).map_err(|e| e.to_string())?
        .insert(file_id, crate::t_image::ORIENTATION_ALGORITHM_VERSION).map_err(|e| e.to_string())?;
    trace.mark("insert", data.len());
    write_tx
        .commit()
        .map_err(|e| format!("Failed to commit write tx: {}", e))?;
    trace.mark("commit", data.len());
    Ok(())
}

pub fn get(file_id: i64) -> Option<Vec<u8>> {
    if file_id <= 0 {
        return None;
    }
    // 1. Direct lock-free read from slab container cache (O(1) direct offset read)
    // Slab cache get() checks and honours the orientation version.
    if let Some(slab) = get_slab_cache() {
        if let Some(data) = slab.get(file_id) {
            return Some(data);
        }
    }

    // 2. Read-only fallback to legacy redb if present (NO lazy write to slab to avoid disk churn/UI stall)
    let db = get_db().ok()?;
    let read_tx = db.begin_read().ok()?;
    let versions = read_tx.open_table(ORIENTATION_VERSIONS).ok()?;
    if versions.get(file_id).ok()??.value() != crate::t_image::ORIENTATION_ALGORITHM_VERSION { return None; }
    let table = read_tx.open_table(THUMBNAILS_TABLE).ok()?;
    let val = table.get(file_id).ok()??;
    Some(val.value().to_vec())
}

/// Offline, resumable orientation maintenance owns an explicit cache handle.
pub struct PhotoCache(Arc<Database>);
impl PhotoCache {
    pub fn coverage(path: &std::path::Path, ids: &[i64]) -> Result<serde_json::Value, String> {
        // Inspection never creates tables, certifies entries, or regenerates images.
        let db = Database::open(path).map_err(|e| e.to_string())?;
        let tx = db.begin_read().map_err(|e| e.to_string())?;
        let photos = tx.open_table(THUMBNAILS_TABLE).map_err(|e| e.to_string())?;
        let versions = tx.open_table(ORIENTATION_VERSIONS).ok();
        let (mut current, mut uncertified, mut missing) = (0usize, 0usize, 0usize);
        let mut missing_sample = Vec::new();
        for id in ids {
            if photos.get(*id).map_err(|e| e.to_string())?.is_none() {
                missing += 1;
                if missing_sample.len() < 20 { missing_sample.push(*id); }
            } else if versions.as_ref().and_then(|t| t.get(*id).ok().flatten())
                .is_some_and(|v| v.value() == crate::t_image::ORIENTATION_ALGORITHM_VERSION) {
                current += 1;
            } else {
                uncertified += 1;
            }
        }
        Ok(serde_json::json!({"catalog_photos": ids.len(), "cache_photos": photos.len().map_err(|e| e.to_string())?,
            "current": current, "uncertified": uncertified, "missing": missing, "missing_sample": missing_sample}))
    }

    pub fn open(path: &std::path::Path) -> Result<Self, String> {
        let db = Arc::new(Database::open(path).map_err(|e|e.to_string())?);
        let tx = begin_cache_write(&db)?;
        { tx.open_table(ORIENTATION_VERSIONS).map_err(|e|e.to_string())?; }
        tx.commit().map_err(|e|e.to_string())?;
        Ok(Self(db))
    }
    pub fn is_current(&self, id: i64) -> Result<bool, String> {
        let tx = self.0.begin_read().map_err(|e|e.to_string())?;
        let versions = tx.open_table(ORIENTATION_VERSIONS).map_err(|e|e.to_string())?;
        let version = versions.get(id).map_err(|e|e.to_string())?.map(|v|v.value());
        let table = tx.open_table(THUMBNAILS_TABLE).map_err(|e|e.to_string())?;
        Ok(version == Some(crate::t_image::ORIENTATION_ALGORITHM_VERSION) && table.get(id).map_err(|e|e.to_string())?.is_some())
    }
    pub fn version(&self, id: i64) -> Result<Option<u32>, String> {
        let tx=self.0.begin_read().map_err(|e|e.to_string())?;
        let table=tx.open_table(ORIENTATION_VERSIONS).map_err(|e|e.to_string())?;
        Ok(table.get(id).map_err(|e|e.to_string())?.map(|v|v.value()))
    }
    pub fn legacy_bytes(&self, id: i64) -> Result<Option<Vec<u8>>, String> {
        let tx=self.0.begin_read().map_err(|e|e.to_string())?;
        let table=tx.open_table(THUMBNAILS_TABLE).map_err(|e|e.to_string())?;
        Ok(table.get(id).map_err(|e|e.to_string())?.map(|v|v.value().to_vec()))
    }
    pub fn contains_legacy(&self, id: i64) -> Result<bool, String> {
        let tx = self.0.begin_read().map_err(|e|e.to_string())?;
        let table = tx.open_table(THUMBNAILS_TABLE).map_err(|e|e.to_string())?;
        Ok(table.get(id).map_err(|e|e.to_string())?.is_some())
    }
    pub fn certify_many(&self, entries: &[(i64, Option<Vec<u8>>)]) -> Result<(), String> {
        let tx = begin_cache_write(&self.0)?;
        {
            let mut table = tx.open_table(THUMBNAILS_TABLE).map_err(|e|e.to_string())?;
            let mut versions = tx.open_table(ORIENTATION_VERSIONS).map_err(|e|e.to_string())?;
            for (id, data) in entries {
                if let Some(data) = data {table.insert(*id,data.as_slice()).map_err(|e|e.to_string())?;}
                if table.get(*id).map_err(|e|e.to_string())?.is_none() {return Err(format!("missing thumbnail {id}"));}
                versions.insert(*id,crate::t_image::ORIENTATION_ALGORITHM_VERSION).map_err(|e|e.to_string())?;
            }
        }
        tx.commit().map_err(|e|e.to_string())
    }
}

pub fn get_preview(file_id: i64) -> Option<Vec<u8>> {
    if file_id <= 0 {
        return None;
    }
    let db = get_db().ok()?;
    let read_tx = db.begin_read().ok()?;
    let versions = read_tx.open_table(PREVIEW_VERSIONS).ok()?;
    if versions.get(file_id).ok()??.value() != crate::t_image::ORIENTATION_ALGORITHM_VERSION {
        return None;
    }
    let table = read_tx.open_table(PREVIEWS_TABLE).ok()?;
    let val = table.get(file_id).ok()??;
    Some(val.value().to_vec())
}

pub fn put_preview(file_id: i64, data: &[u8]) -> Result<(), String> {
    if file_id <= 0 || data.is_empty() {
        return Ok(());
    }
    let db = get_db()?;
    let write_mutex = WRITE_MUTEX.get_or_init(|| Mutex::new(()));
    let _guard = write_mutex
        .lock()
        .map_err(|_| "Poisoned redb write lock".to_string())?;
    let write_tx = begin_cache_write(&db)?;
    {
        let mut table = write_tx
            .open_table(PREVIEWS_TABLE)
            .map_err(|e| format!("Failed to open previews table: {}", e))?;
        table
            .insert(file_id, data)
            .map_err(|e| format!("Failed to insert into previews table: {}", e))?;
    }
    write_tx
        .open_table(PREVIEW_VERSIONS)
        .map_err(|e| e.to_string())?
        .insert(file_id, crate::t_image::ORIENTATION_ALGORITHM_VERSION)
        .map_err(|e| e.to_string())?;
    write_tx
        .commit()
        .map_err(|e| format!("Failed to commit write tx: {}", e))?;
    Ok(())
}

pub fn remove_preview(file_id: i64) -> Result<(), String> {
    if file_id <= 0 {
        return Ok(());
    }
    let db = get_db()?;
    let write_mutex = WRITE_MUTEX.get_or_init(|| Mutex::new(()));
    let _guard = write_mutex
        .lock()
        .map_err(|_| "Poisoned redb write lock".to_string())?;
    let write_tx = begin_cache_write(&db)?;
    {
        let mut table = write_tx
            .open_table(PREVIEWS_TABLE)
            .map_err(|e| format!("Failed to open previews table: {}", e))?;
        let _ = table.remove(file_id);
    }
    let _ = write_tx
        .open_table(PREVIEW_VERSIONS)
        .map_err(|e| e.to_string())?
        .remove(file_id);
    write_tx
        .commit()
        .map_err(|e| format!("Failed to commit write tx: {}", e))?;
    Ok(())
}

pub fn remove(file_id: i64) -> Result<(), String> {
    remove_with(file_id, |id| {
        if let Some(slab) = get_slab_cache() { slab.remove(id) } else { Ok(()) }
    }, remove_cached_tables)
}

fn remove_with(file_id: i64, slab_remove: impl FnOnce(i64) -> Result<(), String>,
    tables_remove: impl FnOnce(i64) -> Result<(), String>) -> Result<(), String> {
    if file_id <= 0 {
        return Ok(());
    }
    slab_remove(file_id)?;
    tables_remove(file_id)
}

fn remove_cached_tables(file_id: i64) -> Result<(), String> {
    let db = get_db()?;
    let write_mutex = WRITE_MUTEX.get_or_init(|| Mutex::new(()));
    let _guard = write_mutex
        .lock()
        .map_err(|_| "Poisoned redb write lock".to_string())?;
    let write_tx = begin_cache_write(&db)?;
    {
        let mut table = write_tx
            .open_table(THUMBNAILS_TABLE)
            .map_err(|e| format!("Failed to open thumbnails table: {}", e))?;
        let _ = table.remove(file_id);
    }
    write_tx.open_table(ORIENTATION_VERSIONS).map_err(|e| e.to_string())?
        .remove(file_id).map_err(|e| e.to_string())?;
    if let Ok(mut previews) = write_tx.open_table(PREVIEWS_TABLE) {
        let _ = previews.remove(file_id);
    }
    if let Ok(mut pversions) = write_tx.open_table(PREVIEW_VERSIONS) {
        let _ = pversions.remove(file_id);
    }
    write_tx
        .commit()
        .map_err(|e| format!("Failed to commit write tx: {}", e))?;
    Ok(())
}

fn face_cache_source(key: &str) -> Option<String> {
    if let Some(json) = key.strip_prefix("face-v5:") {
        // Edited keys append a render key after the JSON identity.
        let value = serde_json::Deserializer::from_str(json)
            .into_iter::<serde_json::Value>().next()?.ok()?;
        return value.get(7)?.as_str().map(str::to_owned);
    }
    reqwest::Url::parse(key.strip_prefix("face-v4:")?).ok()?
        .query_pairs().find(|(name, _)| name == "fp").map(|(_, path)| path.into_owned())
}

fn invalidate_face_sources(db: &Database, sources: &std::collections::HashSet<&str>) -> Result<(), String> {
    let _guard = WRITE_MUTEX.get_or_init(|| Mutex::new(())).lock()
        .map_err(|_| "Poisoned redb write lock".to_string())?;
    let tx = begin_cache_write(db)?;
    {
        let mut table = tx.open_table(FACE_THUMBNAILS_TABLE).map_err(|e| e.to_string())?;
        let mut keys = Vec::new();
        for entry in table.iter().map_err(|e| e.to_string())? {
            let (key, _) = entry.map_err(|e| e.to_string())?;
            if face_cache_source(key.value()).as_deref().is_some_and(|path| sources.contains(path)) {
                keys.push(key.value().to_owned());
            }
        }
        for key in keys { table.remove(key.as_str()).map_err(|e| e.to_string())?; }
    }
    tx.commit().map_err(|e| e.to_string())
}

/// Evict only photos verified by the orientation repair, including their face crops.
pub(crate) fn invalidate_orientation(files: &[(i64, String)]) -> Result<(), String> {
    if files.is_empty() { return Ok(()); }
    let slab = get_slab_cache().ok_or("Failed to initialize orientation repair cache")?;
    for (id, _) in files {
        slab.remove(*id)?;
        remove_cached_tables(*id)?;
    }
    let sources = files.iter().map(|(_, path)| path.as_str()).collect();
    let db = get_db()?;
    invalidate_face_sources(&db, &sources)
}

#[cfg(test)]
mod orientation_version_tests {
    use super::*;
    #[test]
    fn lap106_remove_propagates_slab_failure_before_deleting_other_cache_rows() {
        let mut tables_deleted=false;
        let result=remove_with(42, |_| Err("slab write failed".into()), |_| {tables_deleted=true;Ok(())});
        assert_eq!(result,Err("slab write failed".into()));
        assert!(!tables_deleted);
        let mut order=Vec::new();
        remove_with(0, |_| panic!("invalid id reached slab"), |_| panic!("invalid id reached tables")).unwrap();
        remove_with(42, |_| Ok(()), |id| {order.push(id);Ok(())}).unwrap();
        assert_eq!(order,vec![42]);
    }
    #[test]
    fn lap106_orientation_eviction_preserves_unaffected_face_crops() {
        let path = std::env::temp_dir().join(format!("lap106-faces-{}.redb", uuid::Uuid::new_v4()));
        let source = r"C:\photos\rim.jpg";
        let key = |path: &str| format!("face-v5:{}", serde_json::json!(["/library/1",23,[1,2,3,4],512,1.0,7,"raw",path]));
        let affected = key(source);
        let edited = format!("{affected}:edits:revision-2");
        let legacy = "face-v4:http://lap.localhost/library/1?fp=C%3A%5Cphotos%5Crim.jpg";
        let other = key(r"C:\other\rim.jpg");
        {
            let db = Database::create(&path).unwrap();
            let tx = begin_cache_write(&db).unwrap();
            {
                let mut table = tx.open_table(FACE_THUMBNAILS_TABLE).unwrap();
                for k in [affected.as_str(),edited.as_str(),legacy,other.as_str(),"unrelated"] {
                    table.insert(k,b"crop".as_slice()).unwrap();
                }
            }
            tx.commit().unwrap();
            invalidate_face_sources(&db, &std::collections::HashSet::from([source])).unwrap();
            let tx = db.begin_read().unwrap();
            let table = tx.open_table(FACE_THUMBNAILS_TABLE).unwrap();
            for k in [affected.as_str(),edited.as_str(),legacy] { assert!(table.get(k).unwrap().is_none()); }
            for k in [other.as_str(),"unrelated"] { assert_eq!(table.get(k).unwrap().unwrap().value(),b"crop"); }
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn interrupted_cache_writer_child() {
        if let Some(path) = std::env::var_os("LAP_TEST_BATCH_CACHE") {
            let db = Database::create(path).unwrap();
            put_face_bytes(&db, "durable-face", b"durable crop", true).unwrap();
            put_face_bytes(&db, "pending-face", b"pending crop", false).unwrap();
            // Exit without Drop while the newest derived crop is not durable.
            std::process::exit(0);
        }
        let Some(path) = std::env::var_os("LAP_TEST_CRASH_CACHE") else { return; };
        let cache = FaceCache::open(std::path::Path::new(&path)).unwrap();
        cache.put_many(&[("face-after-crash".into(), b"face crop".to_vec())]).unwrap();
        // Deliberately bypass Drop, as happens on a killed application process.
        std::process::exit(0);
    }

    #[test]
    fn interrupted_write_reopens_without_full_repair_and_preserves_bytes() {
        let path = std::env::temp_dir().join(format!("lap-crash-cache-{}.redb", std::process::id()));
        {
            let db = Database::create(&path).unwrap();
            let tx = begin_cache_write(&db).unwrap();
            { tx.open_table(THUMBNAILS_TABLE).unwrap().insert(1, b"photo thumbnail".as_slice()).unwrap(); }
            tx.commit().unwrap();
        }
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "t_thumb_cache::orientation_version_tests::interrupted_cache_writer_child", "--nocapture"])
            .env("LAP_TEST_CRASH_CACHE", &path)
            .status().unwrap();
        assert!(child.success());
        let repaired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = Arc::clone(&repaired);
        let mut builder = Database::builder();
        builder.set_repair_callback(move |_| { observed.store(true, std::sync::atomic::Ordering::Relaxed); });
        {
            let db = builder.open(&path).unwrap();
            assert!(!repaired.load(std::sync::atomic::Ordering::Relaxed), "full cache repair was required");
            let tx = db.begin_read().unwrap();
            assert_eq!(tx.open_table(THUMBNAILS_TABLE).unwrap().get(1).unwrap().unwrap().value(), b"photo thumbnail");
            assert_eq!(tx.open_table(FACE_THUMBNAILS_TABLE).unwrap().get("face-after-crash").unwrap().unwrap().value(), b"face crop");
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn batched_face_writes_are_readable_and_flush_on_clean_close() {
        let path = std::env::temp_dir().join(format!("lap-batch-clean-{}.redb", std::process::id()));
        {
            let db = Database::create(&path).unwrap();
            put_face_bytes(&db, "first", b"first crop", false).unwrap();
            put_face_bytes(&db, "second", b"second crop", false).unwrap();
            {
                let tx = db.begin_read().unwrap();
                let table = tx.open_table(FACE_THUMBNAILS_TABLE).unwrap();
                assert_eq!(table.get("first").unwrap().unwrap().value(), b"first crop");
                assert_eq!(table.get("second").unwrap().unwrap().value(), b"second crop");
            }
            // Same final durable commit used by shutdown().
            begin_cache_write(&db).unwrap().commit().unwrap();
        }
        {
            let db = Database::open(&path).unwrap();
            let tx = db.begin_read().unwrap();
            let table = tx.open_table(FACE_THUMBNAILS_TABLE).unwrap();
            assert_eq!(table.get("first").unwrap().unwrap().value(), b"first crop");
            assert_eq!(table.get("second").unwrap().unwrap().value(), b"second crop");
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn batched_face_writes_preserve_durable_entries_after_process_exit() {
        let path = std::env::temp_dir().join(format!("lap-batch-crash-{}.redb", std::process::id()));
        {
            let db = Database::create(&path).unwrap();
            let tx = begin_cache_write(&db).unwrap();
            tx.open_table(THUMBNAILS_TABLE).unwrap().insert(1, b"retained photo".as_slice()).unwrap();
            tx.commit().unwrap();
        }
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "t_thumb_cache::orientation_version_tests::interrupted_cache_writer_child", "--nocapture"])
            .env("LAP_TEST_BATCH_CACHE", &path).status().unwrap();
        assert!(child.success());
        let repaired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = Arc::clone(&repaired);
        let mut builder = Database::builder();
        builder.set_repair_callback(move |_| { observed.store(true, std::sync::atomic::Ordering::Relaxed); });
        {
            let db = builder.open(&path).unwrap();
            assert!(!repaired.load(std::sync::atomic::Ordering::Relaxed), "full cache repair was required");
            let tx = db.begin_read().unwrap();
            assert_eq!(tx.open_table(THUMBNAILS_TABLE).unwrap().get(1).unwrap().unwrap().value(), b"retained photo");
            let table = tx.open_table(FACE_THUMBNAILS_TABLE).unwrap();
            assert_eq!(table.get("durable-face").unwrap().unwrap().value(), b"durable crop");
            if let Some(pending) = table.get("pending-face").unwrap() {
                assert_eq!(pending.value(), b"pending crop");
            }
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn legacy_and_other_algorithm_hits_require_certification() {
        let path=std::env::temp_dir().join(format!("lap-orientation-{}-{}.redb",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        {
            let db=Database::create(&path).unwrap();
            let tx=db.begin_write().unwrap();
            {let mut table=tx.open_table(THUMBNAILS_TABLE).unwrap();table.insert(1,b"retained".as_slice()).unwrap();table.insert(2,b"wrong".as_slice()).unwrap();}
            {tx.open_table(ORIENTATION_VERSIONS).unwrap().insert(2,crate::t_image::ORIENTATION_ALGORITHM_VERSION+1).unwrap();}
            tx.commit().unwrap();
        }
        {
            let cache=PhotoCache::open(&path).unwrap();
            assert!(!cache.is_current(1).unwrap());
            assert!(!cache.is_current(2).unwrap());
            cache.certify_many(&[(1,None),(2,Some(b"corrected".to_vec()))]).unwrap();
            assert!(cache.is_current(1).unwrap());assert!(cache.is_current(2).unwrap());
            let tx=cache.0.begin_read().unwrap();let table=tx.open_table(THUMBNAILS_TABLE).unwrap();
            assert_eq!(table.get(1).unwrap().unwrap().value(),b"retained");
            assert_eq!(table.get(2).unwrap().unwrap().value(),b"corrected");
        }
        {let cache=PhotoCache::open(&path).unwrap();assert!(cache.is_current(1).unwrap());assert!(cache.is_current(2).unwrap());}
        let coverage = PhotoCache::coverage(&path, &[1, 2, 3]).unwrap();
        assert_eq!(coverage["current"], 2);
        assert_eq!(coverage["missing"], 1);
        assert_eq!(coverage["missing_sample"], serde_json::json!([3]));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn preview_cache_lifecycle_and_invalidation() {
        let test_file_id = 999991;
        let test_bytes = b"jpeg_preview_data_2048";
        assert_eq!(get_preview(test_file_id), None);
        put_preview(test_file_id, test_bytes).unwrap();
        assert_eq!(get_preview(test_file_id), Some(test_bytes.to_vec()));
        remove_preview(test_file_id).unwrap();
        assert_eq!(get_preview(test_file_id), None);

        // Also verify remove(file_id) cleans up previews
        put_preview(test_file_id, test_bytes).unwrap();
        assert_eq!(get_preview(test_file_id), Some(test_bytes.to_vec()));
        remove(test_file_id).unwrap();
        assert_eq!(get_preview(test_file_id), None);
    }

    #[test]
    fn slab_and_redb_coexistence_and_migration() {
        let temp_dir = std::env::temp_dir().join(format!("lap-coexist-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let slab_dir = temp_dir.join("slabs");
        let slab = Arc::new(crate::slabs::SlabCache::open(&slab_dir).unwrap());
        set_slab_cache_for_testing(Some(Arc::clone(&slab)));

        // Put thumbnail 101 into cache
        put(101, b"coexist_thumbnail_data_101").unwrap();
        // Slab cache must have received it directly
        assert_eq!(slab.get(101).unwrap(), b"coexist_thumbnail_data_101");
        // get() returns it via direct slab lookup
        assert_eq!(get(101).unwrap(), b"coexist_thumbnail_data_101");

        // Remove thumbnail 101
        remove(101).unwrap();
        assert_eq!(slab.get(101), None);
        assert_eq!(get(101), None);

        set_slab_cache_for_testing(None);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
