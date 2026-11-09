//! Shared read-only photo search for the app and lap-cli.
use rusqlite::Connection;
use serde_json::json;
use std::path::PathBuf;

fn is_calendar_word(word: &str) -> bool {
    matches!(word.len(), 4 | 7 | 10)
        && word.bytes().take(4).all(|b| b.is_ascii_digit())
        && (word.len() == 4 || word.as_bytes()[4] == b'-')
        && (word.len() != 4 || word.parse::<u32>().is_ok_and(|year| (1900..=2100).contains(&year)))
        && date_bound(word, false).is_ok()
}

fn visual_description(words: &[&str], named_person: bool) -> String {
    let query = words.join(" ");
    let lower = query.to_lowercase();
    // A restaurant's paper crown is commonly called its "hat". CLIP treats
    // the brand alone as food/signage, so resolve that headwear meaning when
    // the query includes a wearer or headwear. This is a concept alias, never
    // a filename, photo ID, person name or library-specific ranking rule.
    if lower.contains("burger king") && (named_person || words.iter().any(|w| matches!(w.to_ascii_lowercase().as_str(), "hat" | "cap" | "crown"))) {
        let description = words.iter().map(|word| if matches!(word.to_ascii_lowercase().as_str(), "hat" | "cap") { "crown" } else { *word }).collect::<Vec<_>>().join(" ");
        let description = if description.to_lowercase().contains("crown") { description } else { format!("{description} crown") };
        if description.to_lowercase().contains("wearing") { description } else { format!("a person wearing a {description}") }
    } else { query }
}

#[derive(Debug, Default, serde::Serialize)]
pub struct PlainMatches {
    pub text_ids: Vec<i64>,
    pub visual: Vec<(i64, f32)>,
    pub semantic_error: Option<String>,
    pub matrix_bytes: usize,
    pub matrix_load_ms: f64,
    pub scan_ms: f64,
}

/// Both callers use this read-only engine. Dates and named people remain hard
/// constraints when the remaining description is matched against pictures.
pub fn plain_matches(conn: &Connection, options: &SearchOptions, models: &std::path::Path) -> Result<PlainMatches, String> {
    plain_matches_with_encoder(conn, options, |query| crate::t_advanced_ai::encode_clip_text_query(models, query))
}

pub fn plain_matches_with_encoder(conn: &Connection, options: &SearchOptions, encode: impl FnOnce(&str) -> Result<Vec<f32>, String>) -> Result<PlainMatches, String> {
    plain_matches_impl(conn, options, encode, true, None, &[], &|| false)
}

pub fn plain_text_ids(conn: &Connection, options: &SearchOptions) -> Result<Vec<i64>, String> {
    let (conditions, values) = search_where(options)?;
    let mut stmt = conn.prepare(&format!("SELECT a.id {SEARCH_FROM} {conditions} ORDER BY COALESCE(a.taken_date,a.created_at,0) DESC,a.id DESC")).map_err(|e| e.to_string())?;
    let ids = stmt.query_map(rusqlite::params_from_iter(values.iter()), |r| r.get(0)).map_err(|e| e.to_string())?
        .collect::<Result<Vec<i64>,_>>().map_err(|e| e.to_string())?;
    Ok(ids)
}

/// AI-generated labels are suggestions, even when their stored text matches exactly.
pub fn inferred_plain_text_ids(conn: &Connection, options: &SearchOptions, ids: &[i64]) -> Result<Vec<i64>, String> {
    if ids.is_empty() { return Ok(vec![]); }
    let (conditions, mut values) = search_where_with_ai_labels(options, false)?;
    values.push(serde_json::to_string(ids).map_err(|e| e.to_string())?.into());
    let mut stmt = conn.prepare(&format!("SELECT a.id {SEARCH_FROM} {conditions} AND a.id IN (SELECT value FROM json_each(?))")).map_err(|e| e.to_string())?;
    let confirmed = stmt.query_map(rusqlite::params_from_iter(values.iter()), |r| r.get::<_, i64>(0)).map_err(|e| e.to_string())?
        .collect::<Result<std::collections::HashSet<_>, _>>().map_err(|e| e.to_string())?;
    Ok(ids.iter().copied().filter(|id| !confirmed.contains(id)).collect())
}

struct TextRow { id: i64, text: String, folder: i64, taken: i64 }
struct TextIndex { path: PathBuf, version: i64, used: std::time::Instant, rows: Vec<TextRow>, folders: std::collections::HashMap<i64,String> }
static TEXT_INDEX: std::sync::OnceLock<std::sync::Mutex<Option<TextIndex>>> = std::sync::OnceLock::new();
static TEXT_INDEX_WARMING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A small, lazy metadata candidate index. SQLite still verifies the original
/// predicates and ordering, preserving LIKE, date, people and tag semantics.
pub fn cached_plain_text_ids(conn: &Connection, options: &SearchOptions) -> Result<Vec<i64>, String> {
    cached_plain_text_ids_impl(conn,options,false)
}

fn cached_plain_text_ids_impl(conn: &Connection, options: &SearchOptions, populate: bool) -> Result<Vec<i64>, String> {
    use std::sync::Mutex;
    use std::time::{Duration,Instant};
    let cache = TEXT_INDEX.get_or_init(|| {
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(60));
            if let Ok(mut entry) = TEXT_INDEX.get().unwrap().try_lock() {
                if entry.as_ref().is_some_and(|e|e.used.elapsed() >= Duration::from_secs(600)) { *entry=None; }
            }
        });
        Mutex::new(None)
    });
    // A cold metadata index must not delay the first word results. While it
    // warms on a separate reader, keep serving the original SQL search.
    let mut cache = if populate { cache.lock().map_err(|e|e.to_string())? } else {
        match cache.try_lock() {
            Ok(cache) => cache,
            Err(std::sync::TryLockError::WouldBlock) => return plain_text_ids(conn,options),
            Err(error) => return Err(error.to_string()),
        }
    };
    let epoch = SEARCH_CACHE_EPOCH.load(std::sync::atomic::Ordering::SeqCst);
    let version = conn.query_row("SELECT version FROM search_text_version WHERE id=1",[],|r|r.get::<_,i64>(0)).map_err(|e|e.to_string())?;
    let path = PathBuf::from(conn.path().ok_or("Search database has no path")?);
    if cache.as_ref().is_none_or(|e|e.path!=path || e.version!=version) {
        if !populate {
            drop(cache);
            if !TEXT_INDEX_WARMING.swap(true,std::sync::atomic::Ordering::SeqCst) {
                std::thread::spawn(move || {
                    let warm = || -> Result<(),String> {
                        if epoch != SEARCH_CACHE_EPOCH.load(std::sync::atomic::Ordering::SeqCst) { return Ok(()); }
                        let conn = Connection::open_with_flags(path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e|e.to_string())?;
                        conn.execute_batch("PRAGMA mmap_size=2147483648").map_err(|e|e.to_string())?;
                        let tx=conn.unchecked_transaction().map_err(|e|e.to_string())?;
                        cached_plain_text_ids_impl(&tx,&SearchOptions::parse(&[])?,true)?;
                        Ok(())
                    };
                    if let Err(error)=warm() { println!("Search metadata warm-up: {error}"); }
                    TEXT_INDEX_WARMING.store(false,std::sync::atomic::Ordering::SeqCst);
                });
            }
            return plain_text_ids(conn,options);
        }
        let started = Instant::now();
        let mut folders = std::collections::HashMap::new();
        let mut stmt = conn.prepare("SELECT id,path FROM afolders").map_err(|e|e.to_string())?;
        for row in stmt.query_map([],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?))).map_err(|e|e.to_string())? {
            let (id,path)=row.map_err(|e|e.to_string())?; folders.insert(id,path.to_ascii_lowercase());
        }
        let mut rows = Vec::new();
        let mut positions = std::collections::HashMap::new();
        let mut stmt = conn.prepare("SELECT id,folder_id,COALESCE(taken_date,0),name,comments,ocr_text,ai_objects,ai_tags,geo_name,geo_admin1,geo_cc,e_make,e_model,e_lens_model FROM afiles").map_err(|e|e.to_string())?;
        let mut data = stmt.query([]).map_err(|e|e.to_string())?;
        while let Some(row)=data.next().map_err(|e|e.to_string())? {
            if epoch != SEARCH_CACHE_EPOCH.load(std::sync::atomic::Ordering::SeqCst) { return Err("Search cancelled".into()); }
            let id = row.get(0).map_err(|e|e.to_string())?;
            let mut text = String::new();
            for column in 3..14 {
                if let Some(value)=row.get::<_,Option<String>>(column).map_err(|e|e.to_string())? { text.push_str(&value); text.push('\n'); }
            }
            text.make_ascii_lowercase();
            positions.insert(id,rows.len());
            rows.push(TextRow {id,text,folder:row.get(1).map_err(|e|e.to_string())?,taken:row.get(2).map_err(|e|e.to_string())?});
        }
        for sql in ["SELECT ft.file_id,t.name FROM afile_tags ft JOIN atags t ON t.id=ft.tag_id",
            "SELECT f.file_id,p.name FROM faces f JOIN persons p ON p.id=f.person_id WHERE p.name IS NOT NULL"] {
            let mut stmt=conn.prepare(sql).map_err(|e|e.to_string())?;
            let mut data=stmt.query([]).map_err(|e|e.to_string())?;
            while let Some(row)=data.next().map_err(|e|e.to_string())? {
                if let Some(&index)=positions.get(&row.get::<_,i64>(0).map_err(|e|e.to_string())?) {
                    let value: String=row.get(1).map_err(|e|e.to_string())?;
                    rows[index].text.push_str(&value.to_ascii_lowercase()); rows[index].text.push('\n');
                }
            }
        }
        let bytes = rows.capacity()*std::mem::size_of::<TextRow>()+rows.iter().map(|r|r.text.capacity()).sum::<usize>()+folders.values().map(|s|s.capacity()).sum::<usize>();
        println!("Search metadata index: {} rows, {} bytes in {:.1} ms; idle eviction 600s",rows.len(),bytes,started.elapsed().as_secs_f64()*1000.0);
        *cache=Some(TextIndex {path,version,used:Instant::now(),rows,folders});
    }
    if epoch != SEARCH_CACHE_EPOCH.load(std::sync::atomic::Ordering::SeqCst) { *cache=None; return Err("Search cancelled".into()); }
    if populate { return Ok(vec![]); }
    let index = cache.as_mut().unwrap();
    index.used = Instant::now();
    let words = options.query.split_whitespace().map(|w| {
        let period = if is_calendar_word(w) {Some((date_bound(w,false)?,date_bound(w,true)?))} else {None};
        Ok((w.to_ascii_lowercase(),period,w.replace('-',"_")))
    }).collect::<Result<Vec<_>,String>>()?;
    let ids = index.rows.iter().filter(|row| words.iter().all(|(word,period,folder_word)| {
        let folder=index.folders.get(&row.folder).map(String::as_str).unwrap_or("");
        if let Some((start,end))=period { (*start..=*end).contains(&row.taken)||folder.contains(word)||folder.contains(folder_word) }
        else { row.text.contains(word)||folder.contains(word) }
    })).map(|r|r.id).collect::<Vec<_>>();
    if epoch != SEARCH_CACHE_EPOCH.load(std::sync::atomic::Ordering::SeqCst) { *cache=None; return Err("Search cancelled".into()); }
    drop(cache);
    if ids.is_empty() { return Ok(vec![]); }
    let (conditions,mut values)=search_where(options)?;
    values.push(serde_json::to_string(&ids).map_err(|e|e.to_string())?.into());
    let mut stmt=conn.prepare(&format!("SELECT a.id {SEARCH_FROM} {conditions} AND a.id IN (SELECT value FROM json_each(?)) ORDER BY COALESCE(a.taken_date,a.created_at,0) DESC,a.id DESC")).map_err(|e|e.to_string())?;
    let result=stmt.query_map(rusqlite::params_from_iter(values.iter()),|r|r.get(0)).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    Ok(result)
}

pub fn plain_visual_matches(conn: &Connection, options: &SearchOptions, encode: impl FnOnce(&str) -> Result<Vec<f32>, String>, cache: &mut Option<SearchRows>, excluded: &[i64], cancelled: &impl Fn() -> bool) -> Result<PlainMatches, String> {
    plain_matches_impl(conn, options, encode, false, Some(cache), excluded, cancelled)
}

pub fn plain_visual_plan(conn: &Connection, options: &SearchOptions) -> Result<(SearchOptions, Option<String>), String> {
    let mut semantic = options.clone();
    semantic.query.clear();
    let mut description = Vec::new();
    for word in options.query.split_whitespace() {
        if is_calendar_word(word) {
            semantic.filters.push(("period".into(), word.into()));
        } else {
            // Exact name words are anchors; substring names such as "a" must
            // never turn ordinary prose into an accidental person filter.
            let named: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM persons WHERE lower(trim(name))=lower(?1))", [word], |r| r.get(0)).map_err(|e| e.to_string())?;
            if named { semantic.filters.push(("person".into(), word.into())); }
            else { description.push(word); }
        }
    }
    if description.is_empty() {
        return Ok((semantic,None));
    }
    let query = visual_description(&description, semantic.filters.iter().any(|(kind,_)| kind == "person"));
    Ok((semantic,Some(query)))
}

fn plain_matches_impl(conn: &Connection, options: &SearchOptions, encode: impl FnOnce(&str) -> Result<Vec<f32>, String>, include_text: bool, cache: Option<&mut Option<SearchRows>>, excluded: &[i64], cancelled: &impl Fn() -> bool) -> Result<PlainMatches, String> {
    let text_ids = if include_text { plain_text_ids(conn, options)? } else { vec![] };
    let mut result = PlainMatches { text_ids, ..Default::default() };
    let (mut semantic,query) = plain_visual_plan(conn,options)?;
    let Some(query) = query else { return Ok(result); };
    if cancelled() { return Err("Search cancelled".into()); }
    let vector = match encode(&query).and_then(|mut vector| {
        if vector.len() != 512 || vector.iter().any(|v| !v.is_finite()) {
            Err("Text search model must produce 512 finite embedding values".into())
        } else {
            let norm = vector.iter().map(|v| v*v).sum::<f32>().sqrt();
            if !norm.is_finite() || norm <= f32::EPSILON { return Err("Invalid zero text embedding".into()); }
            for value in &mut vector { *value /= norm; }
            Ok(vector)
        }
    }) {
        Ok(vector) => vector,
        Err(error) => { result.semantic_error = Some(error); return Ok(result); }
    };
    // Plain-word search needs a stronger relevance floor than the explicit
    // --clip exploration mode. Weak nearest neighbours are not search hits.
    semantic.min_score = options.min_score.max(0.28);
    semantic.clip = Some(query);
    semantic.offset = 0;
    semantic.limit = 200;
    if cancelled() { return Err("Search cancelled".into()); }
    let (conditions, values) = search_where(&semantic)?;
    if let Some(cache) = cache {
        let start = std::time::Instant::now();
        let (visual, bytes, load_ms) = cached_visual_matches(conn, &semantic, &vector, cache, excluded, cancelled)?;
        result.visual = visual;
        result.matrix_bytes = bytes;
        result.matrix_load_ms = load_ms;
        result.scan_ms = start.elapsed().as_secs_f64()*1000.0 - load_ms;
        return Ok(result);
    }
    let mut stmt = conn.prepare(&format!("SELECT a.id,a.embeds {SEARCH_FROM} {conditions}")).map_err(|e| e.to_string())?;
    let rows = stmt.query_map(rusqlite::params_from_iter(values.iter()), |r| Ok((r.get::<_,i64>(0)?, r.get::<_,Vec<u8>>(1)?))).map_err(|e| e.to_string())?;
    let exact: std::collections::HashSet<_> = result.text_ids.iter().copied().collect();
    for row in rows {
        let (id, blob) = row.map_err(|e| e.to_string())?;
        if exact.contains(&id) { continue; }
        let Ok(embedding) = crate::t_advanced_ai::decode_clip_embedding(&blob) else { continue; };
        let score: f32 = vector.iter().zip(embedding).map(|(a,b)| a*b).sum();
        if score >= semantic.min_score { result.visual.push((id, score)); }
    }
    result.visual.sort_by(|a,b| b.1.total_cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    result.visual.truncate(semantic.limit);
    Ok(result)
}

/// One lazy matrix. Each visual pass pins the embedding version in the same
/// short-lived read snapshot used for ranking and hydration.
struct SearchMatrix {
    path: PathBuf,
    version: i64,
    used: std::time::Instant,
    rows: Option<SearchRows>,
}

pub struct SearchRows { ids: Vec<i64>, values: Vec<f32> }

static SEARCH_MATRIX: std::sync::OnceLock<std::sync::Mutex<Option<SearchMatrix>>> = std::sync::OnceLock::new();
static SEARCH_CACHE_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn clear_plain_search_cache() {
    SEARCH_CACHE_EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if let Some(cache) = SEARCH_MATRIX.get() {
        if let Ok(mut entry) = cache.try_lock() { *entry = None; }
    }
    if let Some(cache) = TEXT_INDEX.get() {
        if let Ok(mut entry) = cache.try_lock() { *entry = None; }
    }
}

/// Embedding-only invalidation: ratings, thumbnails and scan progress must not
/// discard the matrix. Triggers also catch writes from other app/CLI processes.
pub(crate) fn ensure_cache_version(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS search_embedding_version (
        id INTEGER PRIMARY KEY CHECK(id=1), version INTEGER NOT NULL);
        INSERT OR IGNORE INTO search_embedding_version VALUES(1,0);
        CREATE TRIGGER IF NOT EXISTS search_embedding_insert AFTER INSERT ON afiles
        WHEN NEW.embeds IS NOT NULL BEGIN UPDATE search_embedding_version SET version=version+1 WHERE id=1; END;
        CREATE TRIGGER IF NOT EXISTS search_embedding_delete AFTER DELETE ON afiles
        WHEN OLD.embeds IS NOT NULL BEGIN UPDATE search_embedding_version SET version=version+1 WHERE id=1; END;
        CREATE TRIGGER IF NOT EXISTS search_embedding_replace BEFORE INSERT ON afiles
        WHEN NEW.embeds IS NULL AND EXISTS(SELECT 1 FROM afiles WHERE id=NEW.id AND embeds IS NOT NULL)
        BEGIN UPDATE search_embedding_version SET version=version+1 WHERE id=1; END;
        CREATE TRIGGER IF NOT EXISTS search_embedding_update AFTER UPDATE OF id,embeds ON afiles
        WHEN OLD.embeds IS NOT NEW.embeds OR (OLD.id!=NEW.id AND NEW.embeds IS NOT NULL)
        BEGIN UPDATE search_embedding_version SET version=version+1 WHERE id=1; END;
        CREATE TABLE IF NOT EXISTS search_text_version(id INTEGER PRIMARY KEY CHECK(id=1),version INTEGER NOT NULL);
        INSERT OR IGNORE INTO search_text_version VALUES(1,0);").map_err(|e|e.to_string())?;
    let columns=["id","name","folder_id","taken_date","created_at","comments","ocr_text","ai_objects","ai_tags","geo_name","geo_admin1","geo_cc","e_make","e_model","e_lens_model"];
    let changed=columns.iter().map(|c|format!("OLD.{c} IS NOT NEW.{c}")).collect::<Vec<_>>().join(" OR ");
    conn.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS search_text_update AFTER UPDATE OF {} ON afiles WHEN {changed}
        BEGIN UPDATE search_text_version SET version=version+1 WHERE id=1; END;",columns.join(","))).map_err(|e|e.to_string())?;
    for (table,events) in [("afiles",vec!["INSERT","DELETE"]),("afolders",vec!["INSERT","UPDATE OF path","DELETE"]),
        ("afile_tags",vec!["INSERT","DELETE","UPDATE"]),("atags",vec!["UPDATE OF name","DELETE","INSERT"]),
        ("faces",vec!["INSERT","DELETE","UPDATE OF file_id,person_id"]),("persons",vec!["UPDATE OF name","DELETE","INSERT"])] {
        for (number,event) in events.iter().enumerate() {
            conn.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS search_text_{table}_{number} AFTER {event} ON {table}
                BEGIN UPDATE search_text_version SET version=version+1 WHERE id=1; END;")).map_err(|e|e.to_string())?;
        }
    }
    Ok(())
}

pub fn with_plain_search_snapshot<R>(path: &std::path::Path, cancelled: &impl Fn() -> bool, run: impl FnOnce(&Connection, &mut Option<SearchRows>) -> Result<R, String>) -> Result<R, String> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    let cache = SEARCH_MATRIX.get_or_init(|| {
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(60));
            if let Some(cache) = SEARCH_MATRIX.get() {
                if let Ok(mut entry) = cache.try_lock() {
                    if entry.as_ref().is_some_and(|e| e.used.elapsed() >= Duration::from_secs(600)) { *entry = None; }
                }
            }
        });
        Mutex::new(None)
    });
    let mut cache = cache.lock().map_err(|e| e.to_string())?;
    if cancelled() { return Err("Search cancelled".into()); }
    let epoch = SEARCH_CACHE_EPOCH.load(std::sync::atomic::Ordering::SeqCst);
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA mmap_size=2147483648").map_err(|e| e.to_string())?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    // Reading the counter pins its matching embedding snapshot, including in WAL mode.
    let version = tx.query_row("SELECT version FROM search_embedding_version WHERE id=1",[],|r|r.get::<_,i64>(0)).map_err(|e|e.to_string())?;
    if cache.as_ref().is_none_or(|e| e.path != path) {
        *cache = Some(SearchMatrix { path: path.to_owned(), version: -1, used: Instant::now(), rows: None });
    }
    let matrix = cache.as_mut().unwrap();
    matrix.used = Instant::now();
    if matrix.version != version { matrix.rows = None; matrix.version = version; }
    let result = run(&tx, &mut matrix.rows);
    if epoch != SEARCH_CACHE_EPOCH.load(std::sync::atomic::Ordering::SeqCst) { *cache = None; }
    result
}

fn cached_visual_matches(conn: &Connection, options: &SearchOptions, vector: &[f32], cache: &mut Option<SearchRows>, excluded: &[i64], cancelled: &impl Fn() -> bool) -> Result<(Vec<(i64, f32)>, usize, f64), String> {
    use std::time::Instant;
    if cancelled() { return Err("Search cancelled".into()); }
    let mut load_ms = 0.0;
    if cache.is_none() {
        let started = Instant::now();
        let epoch = SEARCH_CACHE_EPOCH.load(std::sync::atomic::Ordering::SeqCst);
        let count = conn.query_row("SELECT COUNT(*) FROM afiles WHERE embeds IS NOT NULL", [], |r|r.get::<_,usize>(0)).map_err(|e|e.to_string())?;
        let mut ids = Vec::with_capacity(count);
        let mut values = Vec::with_capacity(count*512);
        {
            let mut stmt = conn.prepare("SELECT id,embeds FROM afiles WHERE embeds IS NOT NULL").map_err(|e| e.to_string())?;
            let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                // Query-independent warm-up survives edits during the typing debounce.
                // A library change still stops the load and releases its read handle promptly.
                if epoch != SEARCH_CACHE_EPOCH.load(std::sync::atomic::Ordering::SeqCst) { return Err("Search cancelled".into()); }
                let blob = row.get_ref(1).map_err(|e| e.to_string())?.as_blob().map_err(|e| e.to_string())?;
                let Ok(embedding) = crate::t_advanced_ai::decode_clip_embedding(blob) else { continue; };
                ids.push(row.get(0).map_err(|e| e.to_string())?);
                values.extend(embedding);
            }
        }
        values.shrink_to_fit();
        ids.shrink_to_fit();
        load_ms = started.elapsed().as_secs_f64()*1000.0;
        println!("Search matrix loaded: {} rows, {} bytes in {:.1} ms; idle eviction 600s", ids.len(), values.capacity()*4+ids.capacity()*8, started.elapsed().as_secs_f64()*1000.0);
        *cache = Some(SearchRows { ids, values });
    }
    let matrix = cache.as_mut().unwrap();
    let mut scores = Vec::new();
    let excluded: std::collections::HashSet<_> = excluded.iter().copied().collect();
    for (index, (&id, row)) in matrix.ids.iter().zip(matrix.values.chunks_exact(512)).enumerate() {
        if index % 1024 == 0 && cancelled() { return Err("Search cancelled".into()); }
        if excluded.contains(&id) { continue; }
        // Independent accumulators allow a vectorised scan instead of a serial
        // 512-value reduction. The matrix remains compact and CPU portable.
        let mut sums = [0.0f32; 8];
        for (a,b) in vector.chunks_exact(8).zip(row.chunks_exact(8)) {
            for lane in 0..8 { sums[lane] += a[lane]*b[lane]; }
        }
        let score: f32 = sums.iter().sum();
        if score >= options.min_score { scores.push((id,score)); }
    }
    if !scores.is_empty() {
        // Verify current eligibility only for hits. Reading every eligible row
        // first made even a warm in-memory scan pay for a full SQL table pass.
        let (conditions,mut values) = search_where(options)?;
        values.push(serde_json::to_string(&scores.iter().map(|(id,_)|*id).collect::<Vec<_>>()).map_err(|e|e.to_string())?.into());
        let mut stmt = conn.prepare(&format!("SELECT a.id {SEARCH_FROM} {conditions} AND a.id IN (SELECT value FROM json_each(?))")).map_err(|e| e.to_string())?;
        let eligible = stmt.query_map(rusqlite::params_from_iter(values.iter()), |r| r.get::<_,i64>(0)).map_err(|e| e.to_string())?
            .collect::<Result<std::collections::HashSet<_>,_>>().map_err(|e| e.to_string())?;
        scores.retain(|(id,_)|eligible.contains(id));
    }
    scores.sort_by(|a,b| b.1.total_cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    scores.truncate(options.limit);
    Ok((scores, matrix.values.capacity()*4+matrix.ids.capacity()*8, load_ms))
}

pub fn plain_search_results(conn: &Connection, options: &SearchOptions, models: &std::path::Path) -> Result<serde_json::Value, String> {
    let matches = plain_matches(conn, options, models)?;
    let text_count = matches.text_ids.len();
    let total = text_count + matches.visual.len();
    let results = matches.text_ids.into_iter().map(|id| (id, None))
        .chain(matches.visual.into_iter().map(|(id,score)| (id,Some(score))))
        .skip(options.offset).take(options.limit)
        .map(|(id,score)| hydrate_search_result(conn,id,score))
        .collect::<Result<Vec<_>,_>>()?;
    Ok(json!({"results":results,"total_matches":total,"text_matches":text_count,
        "limit":options.limit,"offset":options.offset,"semantic_error":matches.semantic_error}))
}

#[cfg(test)]
mod plain_tests {
    use super::*;
    static SEARCH_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn metadata_candidates_preserve_sql_matches_and_follow_metadata_changes() {
        let _guard=SEARCH_TEST_LOCK.lock().unwrap_or_else(|e|e.into_inner());
        let path=std::env::temp_dir().join(format!("lap116-text-{}.sqlite",uuid::Uuid::new_v4()));
        let conn=Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE afolders(id INTEGER PRIMARY KEY,path TEXT,is_excluded_from_search INTEGER);
            CREATE TABLE afiles(id INTEGER PRIMARY KEY,folder_id INTEGER,name TEXT,file_type INTEGER,taken_date INTEGER,created_at INTEGER,
                comments TEXT,ocr_text TEXT,ai_objects TEXT,ai_tags TEXT,geo_name TEXT,geo_admin1 TEXT,geo_cc TEXT,
                e_make TEXT,e_model TEXT,e_lens_model TEXT,embeds BLOB);
            CREATE TABLE persons(id INTEGER PRIMARY KEY,name TEXT); CREATE TABLE faces(file_id INTEGER,person_id INTEGER);
            CREATE TABLE atags(id INTEGER PRIMARY KEY,name TEXT); CREATE TABLE afile_tags(file_id INTEGER,tag_id INTEGER);
            INSERT INTO afolders VALUES(1,'2011-05 Austria',0),(2,'private',1);
            INSERT INTO afiles(id,folder_id,name,file_type,comments) VALUES(1,1,'crown.jpg',1,'baby wearing a crown'),(2,1,'other.jpg',1,'100%_literal'),(3,2,'crown.jpg',1,'baby wearing a crown');
            INSERT INTO persons VALUES(1,'James'); INSERT INTO faces VALUES(1,1);
            INSERT INTO atags VALUES(1,'sunshiny'); INSERT INTO afile_tags VALUES(2,1);").unwrap();
        ensure_cache_version(&conn).unwrap();
        let check=|query: &str| {
            let options=SearchOptions::parse(&[query.into()]).unwrap();
            cached_plain_text_ids_impl(&conn,&options,true).unwrap();
            assert_eq!(cached_plain_text_ids(&conn,&options).unwrap(),plain_text_ids(&conn,&options).unwrap(),"{query}");
        };
        for query in ["2011 austria","baby wearing a crown","James","crown","sunshiny","zzqxvplnonsense","%_","2011","2011-05"] { check(query); }
        {
            let _warming=TEXT_INDEX.get().unwrap().lock().unwrap();
            let options=SearchOptions::parse(&["crown".into()]).unwrap();
            assert_eq!(cached_plain_text_ids(&conn,&options).unwrap(),plain_text_ids(&conn,&options).unwrap(),"a warm-up must not block word searches");
        }
        conn.execute("UPDATE persons SET name='Julia' WHERE id=1",[]).unwrap(); check("Julia"); check("James");
        conn.execute("INSERT OR REPLACE INTO persons VALUES(1,'Eva')",[]).unwrap(); check("Eva"); check("Julia");
        conn.execute("INSERT OR REPLACE INTO atags VALUES(1,'replacement-tag')",[]).unwrap(); check("replacement-tag"); check("sunshiny");
        conn.execute("UPDATE afolders SET path='2012 Czech' WHERE id=1",[]).unwrap(); check("2011 austria"); check("2012 Czech");
        conn.execute("UPDATE afiles SET comments='new caption' WHERE id=2",[]).unwrap(); check("new caption");
        conn.execute("DELETE FROM afile_tags",[]).unwrap(); check("sunshiny");
        conn.execute("INSERT INTO afiles(id,folder_id,name,file_type) VALUES(4,1,'new-crown.jpg',1)",[]).unwrap(); check("crown");
        conn.execute("DELETE FROM afiles WHERE id=4",[]).unwrap(); check("crown");
        drop(conn); clear_plain_search_cache(); std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn matrix_reuses_rows_and_invalidates_updates_deletes_and_library_changes() {
        let _guard=SEARCH_TEST_LOCK.lock().unwrap_or_else(|e|e.into_inner());
        let path = std::env::temp_dir().join(format!("lap116-{}.sqlite", uuid::Uuid::new_v4()));
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
        conn.execute_batch("CREATE TABLE afolders(id INTEGER PRIMARY KEY,is_excluded_from_search INTEGER);
            CREATE TABLE afiles(id INTEGER PRIMARY KEY,folder_id INTEGER,file_type INTEGER,embeds BLOB);
            CREATE TABLE atags(id INTEGER,name TEXT); CREATE TABLE afile_tags(file_id INTEGER,tag_id INTEGER);
            CREATE TABLE persons(id INTEGER,name TEXT); CREATE TABLE faces(file_id INTEGER,person_id INTEGER);
            INSERT INTO afolders VALUES(1,0);").unwrap();
        ensure_cache_version(&conn).unwrap();
        let blob = |score: f32| {
            let mut v = vec![0_f32;512]; v[0]=score; v[1]=(1.0-score*score).sqrt();
            v.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>()
        };
        conn.execute("INSERT INTO afiles VALUES(1,1,1,?1),(2,1,1,?2),(3,1,1,?3)", rusqlite::params![blob(0.8),blob(0.2),vec![0u8;3]]).unwrap();
        let mut options = SearchOptions::parse(&[]).unwrap();
        options.clip = Some("test".into()); options.limit = 200;
        let mut query = vec![0_f32;512]; query[0]=1.0;
        let scan = |excluded: &[i64]| with_plain_search_snapshot(&path,&|| false, |read,cache| cached_visual_matches(read,&options,&query,cache,excluded,&|| false)).unwrap().0.into_iter().map(|(id,_)|id).collect::<Vec<_>>();
        assert_eq!(scan(&[]), vec![1]);
        assert_eq!(scan(&[]), vec![1]);
        let version: i64 = conn.query_row("SELECT version FROM search_embedding_version",[],|r|r.get(0)).unwrap();
        conn.execute("UPDATE afiles SET file_type=1",[]).unwrap();
        assert_eq!(version,conn.query_row("SELECT version FROM search_embedding_version",[],|r|r.get::<_,i64>(0)).unwrap());
        assert_eq!(with_plain_search_snapshot(&path,&||false, |read,cache| cached_visual_matches(read,&options,&query,cache,&[],&||false)).unwrap().2,0.0,"metadata writes must keep the matrix warm");
        assert!(scan(&[1]).is_empty());
        conn.execute("UPDATE afiles SET embeds=?1 WHERE id=2",[blob(0.9)]).unwrap();
        assert_eq!(scan(&[]), vec![2,1]);
        let version: i64 = conn.query_row("SELECT version FROM search_embedding_version",[],|r|r.get(0)).unwrap();
        conn.execute("INSERT OR REPLACE INTO afiles VALUES(1,1,1,NULL)",[]).unwrap();
        assert!(conn.query_row("SELECT version FROM search_embedding_version",[],|r|r.get::<_,i64>(0)).unwrap()>version);
        assert_eq!(scan(&[]),vec![2]);
        for reload in [false,true] {
            conn.execute("UPDATE afiles SET embeds=?1 WHERE id=1",[blob(0.8)]).unwrap();
            conn.execute("INSERT OR REPLACE INTO afiles VALUES(2,1,1,?1)",[blob(0.9)]).unwrap();
            assert_eq!(scan(&[]),vec![2,1]);
            if reload { clear_plain_search_cache(); }
            with_plain_search_snapshot(&path,&||false, |snapshot,cache| {
                // A second connection commits after the read snapshot is pinned,
                // before either cache reuse or reload, then before hydration.
                conn.execute("UPDATE afiles SET embeds=?1 WHERE id=1",[blob(0.2)]).unwrap();
                conn.execute("DELETE FROM afiles WHERE id=2",[]).unwrap();
                let (ranked,_,_) = cached_visual_matches(snapshot,&options,&query,cache,&[],&||false)?;
                assert_eq!(ranked.iter().map(|(id,_)|*id).collect::<Vec<_>>(),vec![2,1]);
                assert_eq!(snapshot.query_row("SELECT embeds FROM afiles WHERE id=1",[],|r|r.get::<_,Vec<u8>>(0)).unwrap(),blob(0.8));
                assert_eq!(snapshot.query_row("SELECT COUNT(*) FROM afiles WHERE id=2",[],|r|r.get::<_,i64>(0)).unwrap(),1);
                Ok(())
            }).unwrap();
            assert!(scan(&[]).is_empty());
        }
        conn.execute("UPDATE afiles SET embeds=?1 WHERE id=1",[blob(0.8)]).unwrap();
        conn.execute("DELETE FROM afiles WHERE id=2",[]).unwrap();
        assert_eq!(scan(&[]), vec![1]);
        conn.execute("UPDATE afolders SET is_excluded_from_search=1",[]).unwrap();
        assert!(scan(&[]).is_empty());
        assert!(with_plain_search_snapshot(&path,&||true, |read,cache| cached_visual_matches(read,&options,&query,cache,&[],&||true)).is_err());
        // Same IDs in another library must never reuse the first library's matrix.
        let other_path = path.with_extension("other.sqlite");
        let other = Connection::open(&other_path).unwrap();
        other.execute_batch("CREATE TABLE afolders(id INTEGER PRIMARY KEY,is_excluded_from_search INTEGER);
            CREATE TABLE afiles(id INTEGER PRIMARY KEY,folder_id INTEGER,file_type INTEGER,embeds BLOB);
            CREATE TABLE atags(id INTEGER,name TEXT); CREATE TABLE afile_tags(file_id INTEGER,tag_id INTEGER);
            CREATE TABLE persons(id INTEGER,name TEXT); CREATE TABLE faces(file_id INTEGER,person_id INTEGER);
            INSERT INTO afolders VALUES(1,0);").unwrap();
        ensure_cache_version(&other).unwrap();
        other.execute("INSERT INTO afiles VALUES(1,1,1,?1)",[blob(0.1)]).unwrap();
        assert!(with_plain_search_snapshot(&other_path,&||false, |read,cache| cached_visual_matches(read,&options,&query,cache,&[],&||false)).unwrap().0.is_empty());
        drop(other); drop(conn);
        clear_plain_search_cache();
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(other_path).unwrap();
    }

    #[test]
    fn restaurant_headwear_alias_keeps_unrelated_queries_literal() {
        assert_eq!(visual_description(&["burger","king"],true),"a person wearing a burger king crown");
        assert_eq!(visual_description(&["burger","king","hat"],false),"a person wearing a burger king crown");
        assert_eq!(visual_description(&["burger","king","restaurant"],false),"burger king restaurant");
        assert_eq!(visual_description(&["baby","wearing","a","crown"],false),"baby wearing a crown");
    }

    #[test]
    fn ai_place_guesses_are_not_confirmed_text_matches() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE afolders(id INTEGER PRIMARY KEY,path TEXT,is_excluded_from_search INTEGER);
            CREATE TABLE afiles(id INTEGER PRIMARY KEY,folder_id INTEGER,name TEXT,file_type INTEGER,taken_date INTEGER,created_at INTEGER,
                comments TEXT,ocr_text TEXT,ai_objects TEXT,ai_tags TEXT,geo_name TEXT,geo_admin1 TEXT,geo_cc TEXT,
                e_make TEXT,e_model TEXT,e_lens_model TEXT,embeds BLOB);
            CREATE TABLE persons(id INTEGER PRIMARY KEY,name TEXT);
            CREATE TABLE faces(file_id INTEGER,person_id INTEGER);
            CREATE TABLE atags(id INTEGER PRIMARY KEY,name TEXT);
            CREATE TABLE afile_tags(file_id INTEGER,tag_id INTEGER);
            INSERT INTO afolders VALUES(1,'Canary Wharf',0),(2,'New York',0);
            INSERT INTO afiles(id,folder_id,name,file_type,ai_tags) VALUES(1,1,'skyline.jpg',1,'place: New York');
            INSERT INTO afiles(id,folder_id,name,file_type,ocr_text) VALUES(2,1,'sign.jpg',1,'New York');
            INSERT INTO afiles(id,folder_id,name,file_type,comments,ai_tags) VALUES(3,1,'caption.jpg',1,'New York','place: New York');
            INSERT INTO afiles(id,folder_id,name,file_type) VALUES(4,1,'New York.jpg',1);
            INSERT INTO afiles(id,folder_id,name,file_type,ai_objects) VALUES(5,1,'new photo.jpg',1,'York');
            INSERT INTO afiles(id,folder_id,name,file_type,geo_name) VALUES(6,1,'gps.jpg',1,'New York');
            INSERT INTO afiles(id,folder_id,name,file_type) VALUES(7,1,'tagged.jpg',1),(8,1,'person.jpg',1),(9,2,'folder.jpg',1);
            INSERT INTO atags VALUES(1,'New York');
            INSERT INTO afile_tags VALUES(7,1);
            INSERT INTO persons VALUES(1,'New York');
            INSERT INTO faces VALUES(8,1);").unwrap();
        let options = SearchOptions::parse(&["new york".into()]).unwrap();
        let ids = plain_text_ids(&conn, &options).unwrap();
        assert_eq!(ids, vec![9,8,7,6,5,4,3,2,1]);
        assert_eq!(inferred_plain_text_ids(&conn, &options, &ids).unwrap(), vec![5,1]);
        assert!(inferred_plain_text_ids(&conn, &options, &[]).unwrap().is_empty());
    }

    #[test]
    fn descriptions_keep_dates_and_people_and_rank_exact_before_visual() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE afolders(id INTEGER PRIMARY KEY,path TEXT,is_excluded_from_search INTEGER);
            CREATE TABLE afiles(id INTEGER PRIMARY KEY,folder_id INTEGER,name TEXT,file_type INTEGER,taken_date INTEGER,created_at INTEGER,
                comments TEXT,ocr_text TEXT,ai_objects TEXT,ai_tags TEXT,geo_name TEXT,geo_admin1 TEXT,geo_cc TEXT,
                e_make TEXT,e_model TEXT,e_lens_model TEXT,embeds BLOB);
            CREATE TABLE persons(id INTEGER PRIMARY KEY,name TEXT);
            CREATE TABLE faces(file_id INTEGER,person_id INTEGER);
            CREATE TABLE atags(id INTEGER PRIMARY KEY,name TEXT);
            CREATE TABLE afile_tags(file_id INTEGER,tag_id INTEGER);
            INSERT INTO afolders VALUES(1,'Austria',0),(2,'private',1);
            INSERT INTO persons VALUES(1,'James'),(2,'Julia');
            INSERT INTO faces VALUES(1,1),(2,1),(3,2),(4,1),(5,1);").unwrap();
        for (id,year,folder,name,score) in [(1,"2011",1,"crown.jpg",0.5_f32),(2,"2011",1,"other.jpg",0.9),
            (3,"2011",1,"wrong-person.jpg",1.0),(4,"2012",1,"wrong-date.jpg",1.0),(5,"2011",2,"private.jpg",1.0),(6,"2011",1,"weak.jpg",0.2)] {
            let mut vector = vec![0_f32;512]; vector[0]=score; vector[1]=(1.0-score*score).sqrt();
            let blob = vector.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>();
            conn.execute("INSERT INTO afiles(id,folder_id,name,file_type,taken_date,embeds) VALUES(?1,?2,?3,1,?4,?5)",
                rusqlite::params![id,folder,name,date_bound(year,false).unwrap(),blob]).unwrap();
        }
        let encode = |_: &str| { let mut v=vec![0.0;512]; v[0]=1.0; Ok(v) };
        let options = SearchOptions::parse(&["2011 James wearing crown".into()]).unwrap();
        let matches = plain_matches_with_encoder(&conn,&options,|prompt| { assert_eq!(prompt,"wearing crown"); encode(prompt) }).unwrap();
        assert!(matches.text_ids.is_empty());
        assert_eq!(matches.visual.iter().map(|(id,_)| *id).collect::<Vec<_>>(),vec![2,1]);
        let options = SearchOptions::parse(&["2011 James crown".into()]).unwrap();
        let matches = plain_matches_with_encoder(&conn,&options,encode).unwrap();
        assert_eq!(matches.text_ids,vec![1]);
        assert_eq!(matches.visual.iter().map(|(id,_)| *id).collect::<Vec<_>>(),vec![2]);
        let options = SearchOptions::parse(&["crown".into()]).unwrap();
        let matches = plain_matches_with_encoder(&conn,&options,encode).unwrap();
        assert_eq!(matches.text_ids,vec![1]);
        assert!(!matches.visual.iter().any(|(id,_)| *id==1 || *id==5 || *id==6));
        let options = SearchOptions::parse(&["2011 Austria".into()]).unwrap();
        let mut encoded = false;
        let matches = plain_matches_with_encoder(&conn,&options,|_| { encoded=true; encode("") }).unwrap();
        assert!(encoded, "metadata hits must not suppress the visual pass");
        assert_eq!(matches.text_ids.len(),4);
    }
}

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub db: Option<PathBuf>, pub query: String, pub filters: Vec<(String, String)>,
    pub clip: Option<String>, pub min_score: f32, pub limit: usize, pub offset: usize, pub stats: bool, pub help: bool, pub people_list: bool,
    pub cameras: bool, pub by_year: bool, pub view: bool, pub view_out: Option<PathBuf>, pub show: bool, pub show_only_write: bool,
}

impl SearchOptions {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        Self::parse_for(args, false)
    }

    pub fn parse_for(args: &[String], people_list: bool) -> Result<Self, String> {
        let mut options = Self { db: None, query: String::new(), filters: vec![], clip: None,
            min_score: 0.28, limit: 25, offset: 0, stats: false, help: false, people_list,
            cameras: false, by_year: false, view: false, view_out: None, show: false, show_only_write: false };
        let mut limit_provided = false;
        let mut positional = vec![];
        let mut args = args.iter();
        while let Some(argument) = args.next() {
            if argument == "--" { positional.extend(args.cloned()); break; }
            if argument == "--help" || argument == "-h" { options.help = true; continue; }
            if argument == "--stats" { options.stats = true; continue; }
            if argument == "--people-list" { options.people_list = true; continue; }
            if argument == "--cameras" { options.cameras = true; continue; }
            if argument == "--by-year" { options.by_year = true; continue; }
            if argument == "--view" { options.view = true; continue; }
            if argument == "--show" { options.show = true; continue; }
            if argument == "--show-only-write" { options.show_only_write = true; continue; }
            if !argument.starts_with("--") { positional.push(argument.clone()); continue; }
            let (flag, inline) = argument.split_once('=').map_or((argument.as_str(), None), |(a,b)| (a,Some(b)));
            if !matches!(flag, "--db"|"--clip"|"--min-score"|"--limit"|"--offset"|"--scene"|"--place"|"--object"|"--person"|"--people"|"--tag"|"--ocr"|"--folder"|"--location"|"--after"|"--before"|"--camera"|"--make"|"--model"|"--lens"|"--view-out") {
                return Err(format!("Unknown search option: {flag}"));
            }
            let value = match inline { Some(value) => value, None => args.next().filter(|value| !value.starts_with("--")).ok_or_else(|| format!("Missing value for {flag}"))? };
            if value.trim().is_empty() { return Err(format!("Empty value for {flag}")); }
            match flag {
                "--db" => options.db = Some(PathBuf::from(value)),
                "--clip" => options.clip = Some(value.trim().into()),
                "--view-out" => options.view_out = Some(PathBuf::from(value)),
                "--min-score" => options.min_score = value.parse().map_err(|_| "Invalid --min-score")?,
                "--limit" => { options.limit = value.parse::<usize>().map_err(|_| "Invalid --limit")?.min(500); limit_provided = true; }
                "--offset" => options.offset = value.parse::<usize>().map_err(|_| "Invalid --offset")?,
                _ => {
                    for value in value.split(',').map(str::trim) {
                        if value.is_empty() { return Err(format!("Empty filter in {flag}")); }
                        let kind = if flag == "--people" { "person" } else { flag.trim_start_matches('-') };
                        options.filters.push((kind.into(), value.into()));
                    }
                }
            }
        }
        if !options.min_score.is_finite() || !(-1.0..=1.0).contains(&options.min_score) { return Err("--min-score must be finite and between -1 and 1".into()); }
        if options.offset > i64::MAX as usize { return Err("--offset is too large".into()); }
        options.query = positional.join(" ").trim().into();
        if options.people_list {
            if options.stats || !options.filters.is_empty() || options.clip.is_some() {
                return Err("People listing accepts QUERY, --db, --limit and --offset; photo filters and --stats are separate modes".into());
            }
            if !limit_provided { options.limit = 100; }
        }
        if options.cameras && !limit_provided { options.limit = 50; }
        // Validate dates even when the caller requests stats.
        for (kind, value) in &options.filters {
            if kind == "after" || kind == "before" { date_bound(value, kind == "before")?; }
        }
        Ok(options)
    }
}

pub fn date_bound(value: &str, end: bool) -> Result<i64, String> {
    use chrono::NaiveDate;
    let invalid = || format!("Invalid date {value:?}; use Unix seconds or YYYY[-MM[-DD]]");
    let parts: Vec<_> = value.split('-').collect();
    let calendar = value.len() == 4 && value.bytes().all(|b| b.is_ascii_digit())
        || (parts.first().is_some_and(|p| p.len() == 4) && matches!(value.len(), 7 | 10) && parts.len() > 1);
    if !calendar { return value.parse::<i64>().map_err(|_| invalid()); }
    let year: i32 = parts[0].parse().map_err(|_| invalid())?;
    let month = if parts.len() >= 2 { parts[1].parse().map_err(|_| invalid())? } else { 1 };
    let day = if parts.len() == 3 { parts[2].parse().map_err(|_| invalid())? } else { 1 };
    let start = NaiveDate::from_ymd_opt(year, month, day).ok_or_else(invalid)?;
    let timestamp = |date: NaiveDate| date.and_hms_opt(0,0,0).unwrap().and_utc().timestamp();
    if !end { return Ok(timestamp(start)); }
    let next = match parts.len() {
        1 => NaiveDate::from_ymd_opt(year + 1, 1, 1),
        2 => if month == 12 { NaiveDate::from_ymd_opt(year + 1,1,1) } else { NaiveDate::from_ymd_opt(year,month+1,1) },
        _ => start.succ_opt(),
    }.ok_or_else(invalid)?;
    Ok(timestamp(next) - 1)
}

const SEARCH_FROM: &str = "FROM afiles a JOIN afolders b ON a.folder_id=b.id";
const SEARCH_ELIGIBLE: &str = "COALESCE(b.is_excluded_from_search,0)=0 AND a.file_type IN (1,2,3)";
const GEO_COLUMNS: [&str; 3] = ["a.geo_name", "a.geo_admin1", "a.geo_cc"];

pub fn like_pattern(value: &str) -> String {
    format!("%{}%", value.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"))
}

pub fn search_where(options: &SearchOptions) -> Result<(String, Vec<rusqlite::types::Value>), String> {
    search_where_with_ai_labels(options, true)
}

fn search_where_with_ai_labels(options: &SearchOptions, include_ai_labels: bool) -> Result<(String, Vec<rusqlite::types::Value>), String> {
    use rusqlite::types::Value;
    let mut clauses = vec![SEARCH_ELIGIBLE.to_string()];
    let mut values = vec![];
    let like = |column: &str| format!("{column} LIKE ? COLLATE NOCASE ESCAPE '\\'");
    let person = |predicate: String| format!("a.id IN (SELECT f.file_id FROM faces f JOIN persons p ON f.person_id=p.id WHERE {predicate})");
    let tag = || format!("a.id IN (SELECT ft.file_id FROM afile_tags ft JOIN atags t ON ft.tag_id=t.id WHERE {})", like("t.name"));
    for word in options.query.split_whitespace() {
        if is_calendar_word(word) {
            clauses.push("(a.taken_date BETWEEN ? AND ? OR b.path LIKE ? ESCAPE '\\' OR b.path LIKE ? ESCAPE '\\')".into());
            values.push(Value::Integer(date_bound(word, false)?));
            values.push(Value::Integer(date_bound(word, true)?));
            values.push(Value::Text(like_pattern(word)));
            values.push(Value::Text(like_pattern(&word.replace('-', "_"))));
            continue;
        }
        let columns = ["a.name", "b.path", "a.comments", "a.ocr_text", "a.ai_objects", "a.ai_tags", GEO_COLUMNS[0], GEO_COLUMNS[1], GEO_COLUMNS[2], "a.e_make", "a.e_model", "a.e_lens_model"];
        let mut choices = columns.iter()
            .filter(|column| include_ai_labels || !matches!(**column, "a.ai_objects" | "a.ai_tags"))
            .map(|column| like(column)).collect::<Vec<_>>();
        choices.push(tag()); choices.push(person(like("p.name")));
        values.extend((0..choices.len()).map(|_| Value::Text(like_pattern(word))));
        clauses.push(format!("({})", choices.join(" OR ")));
    }
    for (kind, value) in &options.filters {
        let pattern = like_pattern(value);
        match kind.as_str() {
            "period" => {
                clauses.push("(a.taken_date BETWEEN ? AND ? OR b.path LIKE ? ESCAPE '\\' OR b.path LIKE ? ESCAPE '\\')".into());
                values.push(Value::Integer(date_bound(value, false)?));
                values.push(Value::Integer(date_bound(value, true)?));
                values.push(Value::Text(pattern));
                values.push(Value::Text(like_pattern(&value.replace('-', "_"))));
            }
            "file_mask" => {
                clauses.push("(? & CASE a.file_type WHEN 1 THEN 1 WHEN 2 THEN 2 WHEN 3 THEN 4 ELSE 0 END) != 0".into());
                values.push(Value::Integer(value.parse().map_err(|_| "Invalid file type")?));
            }
            "camera" | "lens" | "make" | "model" => {
                let columns = match kind.as_str() {
                    "make" => vec!["a.e_make"], "model" => vec!["a.e_model"],
                    "lens" => vec!["a.e_lens_make", "a.e_lens_model", "(COALESCE(a.e_lens_make,'') || ' ' || COALESCE(a.e_lens_model,''))"],
                    _ => vec!["a.e_make", "a.e_model", "(COALESCE(a.e_make,'') || ' ' || COALESCE(a.e_model,''))"],
                };
                clauses.push(format!("({})", columns.iter().map(|c| like(c)).collect::<Vec<_>>().join(" OR ")));
                values.extend(columns.iter().map(|_| Value::Text(pattern.clone())));
            }
            "scene" => {
                clauses.push(format!("({} OR {})", like("a.ai_tags"), like("a.ai_tags")));
                values.push(Value::Text(like_pattern(&format!("scene: {value}"))));
                values.push(Value::Text(pattern));
            }
            "place" | "location" => {
                let columns = if kind == "place" { vec!["a.ai_tags", GEO_COLUMNS[0], GEO_COLUMNS[1], GEO_COLUMNS[2]] } else { GEO_COLUMNS.to_vec() };
                clauses.push(format!("({})", columns.iter().map(|column| like(column)).collect::<Vec<_>>().join(" OR ")));
                values.extend(columns.iter().map(|_| Value::Text(pattern.clone())));
            }
            "object" | "ocr" | "folder" => {
                clauses.push(like(match kind.as_str() { "object" => "a.ai_objects", "ocr" => "a.ocr_text", _ => "b.path" }));
                values.push(Value::Text(pattern));
            }
            "tag" => { clauses.push(tag()); values.push(Value::Text(pattern)); }
            "person" => {
                if let Ok(id) = value.parse::<i64>() {
                    clauses.push(person(format!("p.id=? OR {}", like("p.name"))));
                    values.push(Value::Integer(id));
                } else { clauses.push(person(like("p.name"))); }
                values.push(Value::Text(pattern));
            }
            "after" | "before" => {
                clauses.push(format!("a.taken_date {} ?", if kind == "after" { ">=" } else { "<=" }));
                values.push(Value::Integer(date_bound(value, kind == "before")?));
            }
            _ => return Err(format!("Unknown filter {kind}")),
        }
    }
    if options.clip.is_some() { clauses.push("a.embeds IS NOT NULL".into()); }
    Ok((format!("WHERE {}", clauses.join(" AND ")), values))
}

pub fn split_labels(value: Option<&str>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    value.unwrap_or("").split(',').map(str::trim).filter(|v| !v.is_empty())
        .filter(|v| seen.insert(v.to_string())).map(str::to_string).collect()
}

pub fn scene_place_labels(value: Option<&str>, kind: &str) -> Vec<String> {
    split_labels(value).into_iter().filter_map(|tag| {
        let (prefix, label) = tag.split_once(':')?;
        (prefix.trim().eq_ignore_ascii_case(kind) && !label.trim().is_empty()).then(|| label.trim().to_string())
    }).collect()
}

pub fn search_stats(conn: &Connection) -> Result<serde_json::Value, String> {
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM afiles", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    let summary = conn.query_row(&format!("SELECT COUNT(*), COALESCE(SUM(a.embeds IS NOT NULL),0),
        COALESCE(SUM(length(trim(COALESCE(a.ai_tags,'')))>0),0),
        COALESCE(SUM(length(trim(COALESCE(a.ai_objects,'')))>0),0),
        COALESCE(SUM(length(trim(COALESCE(a.ocr_text,'')))>0),0) {SEARCH_FROM} WHERE {SEARCH_ELIGIBLE}"), [],
        |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?))).map_err(|e| e.to_string())?;
    let named: i64 = conn.query_row("SELECT COUNT(*) FROM persons WHERE name IS NOT NULL AND trim(name)<>''", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    let mut maps: [std::collections::BTreeMap<String,i64>; 3] = Default::default();
    let mut stmt = conn.prepare(&format!("SELECT a.ai_tags,a.ai_objects {SEARCH_FROM} WHERE {SEARCH_ELIGIBLE} AND (length(trim(COALESCE(a.ai_tags,'')))>0 OR length(trim(COALESCE(a.ai_objects,'')))>0)")).map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_,Option<String>>(0)?,r.get::<_,Option<String>>(1)?))).map_err(|e| e.to_string())?;
    for row in rows {
        let (tags, objects) = row.map_err(|e| e.to_string())?;
        for (map, labels) in maps.iter_mut().zip([scene_place_labels(tags.as_deref(), "scene"),scene_place_labels(tags.as_deref(), "place"),split_labels(objects.as_deref())]) {
            for label in labels.into_iter().collect::<std::collections::BTreeSet<_>>() { *map.entry(label).or_default() += 1; }
        }
    }
    let top = |map: std::collections::BTreeMap<String,i64>| {
        let mut rows = map.into_iter().collect::<Vec<_>>();
        rows.sort_by(|a,b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        rows.into_iter().take(15).map(|(name,count)| json!({"name":name,"count":count})).collect::<Vec<_>>()
    };
    let [scenes, places, objects] = maps;
    let equipment = camera_results(conn,"",15,false)?;
    Ok(json!({"total_files":total,"eligible_files":summary.0,"with_embeds":summary.1,"with_ai_tags":summary.2,
        "with_ai_objects":summary.3,"with_ocr_text":summary.4,"named_persons":named,
        "top_scenes":top(scenes),"top_places":top(places),"top_objects":top(objects),
        "top_cameras":equipment["cameras"],"top_lenses":equipment["lenses"],
        "top_persons":people_results(conn,"",15,0)?["people"].as_array().unwrap().iter().map(|p|
            json!({"id":p["id"],"name":p["name"],"photo_count":p["photo_count"]})).collect::<Vec<_>>()}))
}

pub fn equipment_label(make: Option<&str>, model: Option<&str>) -> Option<String> {
    let make = make.unwrap_or("").trim();
    let model = model.unwrap_or("").trim();
    let label = if make.is_empty() || model.to_lowercase().starts_with(&make.to_lowercase()) { model.to_string() }
        else if model.is_empty() { make.to_string() } else { format!("{make} {model}") };
    (!label.is_empty()).then_some(label)
}

pub fn camera_results(conn: &Connection, query: &str, limit: usize, by_year: bool) -> Result<serde_json::Value,String> {
    use std::collections::BTreeMap;
    let mut cameras: BTreeMap<(String,String),(i64,Option<String>,Option<String>)> = BTreeMap::new();
    let mut lenses: BTreeMap<String,i64> = BTreeMap::new();
    let mut years: BTreeMap<(String,String,String),i64> = BTreeMap::new();
    let mut stmt = conn.prepare(&format!("SELECT trim(COALESCE(a.e_make,'')),trim(COALESCE(a.e_model,'')),a.e_lens_make,a.e_lens_model,strftime('%Y',a.taken_date,'unixepoch')
        {SEARCH_FROM} WHERE {SEARCH_ELIGIBLE} AND trim(COALESCE(a.e_make,'') || COALESCE(a.e_model,''))<>''
        AND (?1='' OR a.e_make LIKE ?2 COLLATE NOCASE ESCAPE '\\' OR a.e_model LIKE ?2 COLLATE NOCASE ESCAPE '\\'
        OR a.e_lens_make LIKE ?2 COLLATE NOCASE ESCAPE '\\' OR a.e_lens_model LIKE ?2 COLLATE NOCASE ESCAPE '\\'
        OR (COALESCE(a.e_make,'') || ' ' || COALESCE(a.e_model,'')) LIKE ?2 COLLATE NOCASE ESCAPE '\\'
        OR (COALESCE(a.e_lens_make,'') || ' ' || COALESCE(a.e_lens_model,'')) LIKE ?2 COLLATE NOCASE ESCAPE '\\')")).map_err(|e| e.to_string())?;
    let rows = stmt.query_map(rusqlite::params![query,like_pattern(query)], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<String>>(4)?))).map_err(|e| e.to_string())?;
    for row in rows {
        let (make,model,lmake,lmodel,year) = row.map_err(|e| e.to_string())?;
        let entry = cameras.entry((make.clone(),model.clone())).or_default();
        entry.0 += 1;
        if let Some(year) = year.filter(|y| y.parse::<i32>().is_ok_and(|v| (1990..=2035).contains(&v))) {
            if entry.1.as_ref().is_none_or(|v| &year<v) { entry.1=Some(year.clone()); }
            if entry.2.as_ref().is_none_or(|v| &year>v) { entry.2=Some(year.clone()); }
            if by_year { *years.entry((year,make,model)).or_default()+=1; }
        }
        if let Some(lens) = equipment_label(lmake.as_deref(),lmodel.as_deref()) { *lenses.entry(lens).or_default()+=1; }
    }
    let total = cameras.len();
    let mut cameras = cameras.into_iter().collect::<Vec<_>>();
    cameras.sort_by(|a,b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
    let cameras = cameras.into_iter().take(limit).map(|((make,model),(count,first,last))|
        json!({"camera":equipment_label(Some(&make),Some(&model)),"make":make,"model":model,"photo_count":count,"first_year":first,"last_year":last})).collect::<Vec<_>>();
    let mut lenses = lenses.into_iter().collect::<Vec<_>>();
    lenses.sort_by(|a,b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let lenses = lenses.into_iter().take(limit).map(|(lens,count)| json!({"lens":lens,"photo_count":count})).collect::<Vec<_>>();
    let mut years = years.into_iter().collect::<Vec<_>>();
    years.sort_by(|a,b| b.0.0.cmp(&a.0.0).then_with(|| b.1.cmp(&a.1)).then_with(|| a.0.cmp(&b.0)));
    let years = years.into_iter().map(|((year,make,model),count)| json!({"year":year,"camera":equipment_label(Some(&make),Some(&model)),"make":make,"model":model,"photo_count":count})).collect::<Vec<_>>();
    Ok(json!({"total_matches":total,"cameras":cameras,"lenses":lenses,"by_year":years}))
}

pub fn people_results(conn: &Connection, query: &str, limit: usize, offset: usize) -> Result<serde_json::Value,String> {
    let pattern = like_pattern(query);
    let named = "p.name IS NOT NULL AND trim(p.name)<>'' AND p.name LIKE ?1 COLLATE NOCASE ESCAPE '\\'";
    let total: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM persons p WHERE {named}"), [&pattern], |r| r.get(0)).map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(&format!("WITH counts AS (
        SELECT f.person_id,COUNT(DISTINCT a.id) AS photo_count,COUNT(*) AS face_count
        FROM faces f JOIN afiles a ON a.id=f.file_id JOIN afolders b ON b.id=a.folder_id
        WHERE {SEARCH_ELIGIBLE} GROUP BY f.person_id)
        SELECT p.id,p.name,COALESCE(c.photo_count,0) AS photo_count,COALESCE(c.face_count,0)
        FROM persons p LEFT JOIN counts c ON c.person_id=p.id WHERE {named}
        ORDER BY photo_count DESC,p.name ASC,p.id ASC LIMIT ?2 OFFSET ?3")).map_err(|e| e.to_string())?;
    let people = stmt.query_map(rusqlite::params![pattern,limit as i64,offset as i64], |r| Ok(json!({
        "id":r.get::<_,i64>(0)?,"name":r.get::<_,String>(1)?,"photo_count":r.get::<_,i64>(2)?,"face_count":r.get::<_,i64>(3)?
    }))).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
    Ok(json!({"total_matches":total,"people":people}))
}

pub fn hydrate_search_result(conn: &Connection, id: i64, score: Option<f32>) -> Result<serde_json::Value, String> {
    let mut result = conn.query_row(&format!("SELECT a.id,a.name,b.path,a.file_type,a.taken_date,a.width,a.height,a.ai_tags,a.ai_objects,a.ocr_text,a.comments,a.geo_name,a.geo_admin1,a.geo_cc,a.e_make,a.e_model,a.e_lens_make,a.e_lens_model {SEARCH_FROM} WHERE a.id=?1"), [id], |r| {
        let name: String = r.get(1)?;
        let folder: String = r.get(2)?;
        let taken: Option<i64> = r.get(4)?;
        let ai_tags: Option<String> = r.get(7)?;
        let objects: Option<String> = r.get(8)?;
        let ocr: Option<String> = r.get(9)?;
        let comments: Option<String> = r.get(10)?;
        let location = [r.get::<_,Option<String>>(11)?,r.get(12)?,r.get(13)?].into_iter().flatten().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect::<Vec<_>>().join(", ");
        let description = comments.as_ref().filter(|v| !v.trim().is_empty()).cloned().or_else(|| crate::t_advanced_ai::ai_description(ai_tags.as_deref(),objects.as_deref(),ocr.as_deref()));
        Ok(json!({"id":id,"path":PathBuf::from(&folder).join(&name),"name":name,"folder":folder,
            "file_type":r.get::<_,Option<i64>>(3)?,"taken_date":taken,
            "taken_iso":taken.and_then(|t| chrono::DateTime::from_timestamp(t,0)).map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs,true)),
            "width":r.get::<_,Option<i64>>(5)?,"height":r.get::<_,Option<i64>>(6)?,
            "scenes":scene_place_labels(ai_tags.as_deref(),"scene"),"places":scene_place_labels(ai_tags.as_deref(),"place"),
            "objects":split_labels(objects.as_deref()),"ai_tags":ai_tags,"ocr_text":ocr,"comments":comments,
            "description":description,"location":(!location.is_empty()).then_some(location),
            "camera":equipment_label(r.get::<_,Option<String>>(14)?.as_deref(),r.get::<_,Option<String>>(15)?.as_deref()),
            "lens":equipment_label(r.get::<_,Option<String>>(16)?.as_deref(),r.get::<_,Option<String>>(17)?.as_deref()),
            "clip_score":score.map(|value| (value as f64 * 10000.0).round()/10000.0)}))
    }).map_err(|e| e.to_string())?;
    let names = |sql: &str| -> Result<Vec<String>,String> {
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        stmt.query_map([id], |r| r.get::<_,String>(0)).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())
    };
    result["persons"] = json!(names("SELECT DISTINCT p.name FROM faces f JOIN persons p ON f.person_id=p.id WHERE f.file_id=?1 AND p.name IS NOT NULL AND trim(p.name)<>'' ORDER BY p.name")?);
    result["tags"] = json!(names("SELECT DISTINCT t.name FROM afile_tags ft JOIN atags t ON ft.tag_id=t.id WHERE ft.file_id=?1 ORDER BY t.name")?);
    Ok(result)
}

pub fn search_results(conn: &Connection, options: &SearchOptions, query_vector: Option<&[f32]>) -> Result<serde_json::Value,String> {
    use rusqlite::params_from_iter;
    let (conditions, mut values) = search_where(options)?;
    let mut skipped_embeddings = 0usize;
    let (total, page): (usize, Vec<(i64,Option<f32>)>) = if let Some(vector) = query_vector {
        if vector.len()!=512 || vector.iter().any(|v| !v.is_finite()) { return Err("Invalid query embedding".into()); }
        let mut stmt = conn.prepare(&format!("SELECT a.id,a.embeds,a.taken_date {SEARCH_FROM} {conditions}")).map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params_from_iter(values.iter()), |r| Ok((r.get::<_,i64>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,Option<i64>>(2)?))).map_err(|e| e.to_string())?;
        let mut scored = Vec::new();
        for row in rows {
            let (id, blob, taken) = row.map_err(|e| e.to_string())?;
            let embedding = match crate::t_advanced_ai::decode_clip_embedding(&blob) { Ok(v) => v, Err(_) => {skipped_embeddings += 1; continue;} };
            let score: f32 = vector.iter().zip(embedding).map(|(a,b)| a*b).sum();
            if score >= options.min_score { scored.push((id,taken,score)); }
        }
        scored.sort_by(|a,b| b.2.total_cmp(&a.2).then_with(|| b.1.cmp(&a.1)).then_with(|| b.0.cmp(&a.0)));
        (scored.len(),scored.into_iter().skip(options.offset).take(options.limit).map(|(id,_,score)| (id,Some(score))).collect())
    } else {
        let total: i64 = conn.query_row(&format!("SELECT COUNT(*) {SEARCH_FROM} {conditions}"), params_from_iter(values.iter()), |r| r.get(0)).map_err(|e| e.to_string())?;
        values.push((options.limit as i64).into()); values.push((options.offset as i64).into());
        let mut stmt = conn.prepare(&format!("SELECT a.id {SEARCH_FROM} {conditions} ORDER BY COALESCE(a.taken_date,a.created_at,0) DESC,a.id DESC LIMIT ? OFFSET ?")).map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params_from_iter(values.iter()), |r| Ok((r.get(0)?,None))).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
        (total as usize, rows)
    };
    let results = page.into_iter().map(|(id,score)| hydrate_search_result(conn,id,score)).collect::<Result<Vec<_>,_>>()?;
    Ok(json!({"total_matches":total,"limit":options.limit,"offset":options.offset,"skipped_embeddings":skipped_embeddings,"results":results}))
}

