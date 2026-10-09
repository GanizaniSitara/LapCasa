use redb::{Database, TableDefinition, ReadableTable};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

const THUMBNAILS_TABLE: TableDefinition<i64, &[u8]> = TableDefinition::new("thumbnails");
const FACE_THUMBNAILS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("face_thumbnails");
const ORIENTATION_VERSIONS: TableDefinition<i64, u32> = TableDefinition::new("thumbnail_orientation_versions");

static DB_INSTANCE: Mutex<Option<Arc<Database>>> = Mutex::new(None);
static CLOSING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static WRITE_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

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
    let db = Database::create(path).map_err(|e| format!("Failed to create redb database: {}", e))?;
    trace.mark("database", 0);

    // Ensure the thumbnails table exists
    {
        let write_tx = db
            .begin_write()
            .map_err(|e| format!("Failed to begin write tx: {}", e))?;
        {
            let _ = write_tx
                .open_table(THUMBNAILS_TABLE)
                .map_err(|e| format!("Failed to open thumbnails table: {}", e))?;
            let _ = write_tx.open_table(FACE_THUMBNAILS_TABLE).map_err(|e| e.to_string())?;
            let _ = write_tx.open_table(ORIENTATION_VERSIONS).map_err(|e| e.to_string())?;
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
    let db = DB_INSTANCE.lock().map_err(|e| e.to_string())?.take();
    if let Some(db) = db {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while Arc::strong_count(&db) > 1 {
            if std::time::Instant::now() >= deadline {
                return Err("Thumbnail cache still in use at shutdown; recovery may be needed".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
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
    let tx = db.begin_write().map_err(|e| e.to_string())?;
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
        let tx = db.begin_write().map_err(|e| e.to_string())?;
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
        let tx = self.0.begin_write().map_err(|e| e.to_string())?;
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
    let mut trace = crate::t_protocol::ThumbnailTrace::new(file_id, "cache_write");
    let db = get_db()?;
    trace.mark("open", data.len());
    let write_mutex = WRITE_MUTEX.get_or_init(|| Mutex::new(()));
    let _guard = write_mutex
        .lock()
        .map_err(|_| "Poisoned redb write lock".to_string())?;
    trace.mark("lock_wait", data.len());
    let write_tx = db
        .begin_write()
        .map_err(|e| format!("Failed to begin write tx: {}", e))?;
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
    pub fn open(path: &std::path::Path) -> Result<Self, String> {
        let db = Arc::new(Database::open(path).map_err(|e|e.to_string())?);
        let tx = db.begin_write().map_err(|e|e.to_string())?;
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
        let tx = self.0.begin_write().map_err(|e|e.to_string())?;
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

pub fn remove(file_id: i64) -> Result<(), String> {
    if file_id <= 0 {
        return Ok(());
    }
    let db = get_db()?;
    let write_mutex = WRITE_MUTEX.get_or_init(|| Mutex::new(()));
    let _guard = write_mutex
        .lock()
        .map_err(|_| "Poisoned redb write lock".to_string())?;
    let write_tx = db
        .begin_write()
        .map_err(|e| format!("Failed to begin write tx: {}", e))?;
    {
        let mut table = write_tx
            .open_table(THUMBNAILS_TABLE)
            .map_err(|e| format!("Failed to open thumbnails table: {}", e))?;
        let _ = table.remove(file_id);
    }
    write_tx.open_table(ORIENTATION_VERSIONS).map_err(|e| e.to_string())?
        .remove(file_id).map_err(|e| e.to_string())?;
    write_tx
        .commit()
        .map_err(|e| format!("Failed to commit write tx: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod orientation_version_tests {
    use super::*;
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
        std::fs::remove_file(path).unwrap();
    }
}
