//! Small, persistent face crops. Cache hits never read or decode the original photo.
use image::{DynamicImage, GenericImageView};
use std::sync::OnceLock;

static GENERATION: OnceLock<tokio::sync::Semaphore> = OnceLock::new();

// A crop's identity is independent of URI scheme and parameter order. This is
// shared by the protocol and pre-generation, including source version/geometry.
pub fn cache_key(uri: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(uri).map_err(|e| e.to_string())?;
    let (rect, size, aspect, face_id) = parse_crop(url.query().unwrap_or("")).ok_or("invalid face crop")?;
    let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
    let version = params.get("v").and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
    let mut key=format!("face-v5:{}", serde_json::to_string(&(url.path(), version, rect, size, aspect, face_id,
        params.get("fc").map(|v| v.as_ref()).unwrap_or(""),
        params.get("fp").map(|v| v.as_ref()).unwrap_or(""))).map_err(|e| e.to_string())?);
    if let Some(render_key)=params.get("rk") {key.push_str(&format!(":edits:{render_key}"));}
    Ok(key)
}

pub fn parse_crop(query: &str) -> Option<([f64; 4], u32, f64, i64)> {
    let value = |name: &str| query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key == name).then_some(value)
    });
    let rect = [value("fx")?.parse().ok()?, value("fy")?.parse().ok()?,
        value("fw")?.parse().ok()?, value("fh")?.parse().ok()?];
    if rect.iter().any(|v: &f64| !v.is_finite())
        || rect[2] <= 0.0 || rect[3] <= 0.0
        || rect.iter().any(|v| v.abs() > u32::MAX as f64) { return None; }
    let size = value("fs")?.parse::<u32>().ok()?.clamp(128, 1024);
    let aspect = value("fa")?.parse::<f64>().ok()?;
    if !aspect.is_finite() || !(0.5..=2.0).contains(&aspect) { return None; }
    let face_id = value("fid").unwrap_or("0").parse::<i64>().ok()?;
    if face_id < 0 { return None; }
    Some((rect, size, aspect, face_id))
}

// Legacy detector boxes were scaled from upright thumbnails; Picasa imports are raw.
pub(crate) fn is_display_box(conn: &rusqlite::Connection, file_id: i64, face_id: i64) -> Result<bool, String> {
    if face_id == 0 { return Ok(false); }
    let json: String = conn.query_row("SELECT bbox FROM faces WHERE id = ?1 AND file_id = ?2",
        rusqlite::params![face_id, file_id], |row| row.get(0)).map_err(|e| e.to_string())?;
    if let Some(space) = serde_json::from_str::<serde_json::Value>(&json).ok()
        .and_then(|value| value.get("coordinate_space")?.as_str().map(str::to_owned)) {
        return Ok(space == "display");
    }
    let has_imports: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'picasa_import_faces')",
        [], |row| row.get(0)).map_err(|e| e.to_string())?;
    if !has_imports { return Ok(true); }
    let imported: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM picasa_import_faces WHERE face_id = ?1)",
        [face_id], |row| row.get(0)).map_err(|e| e.to_string())?;
    Ok(!imported)
}

fn source_rect(bbox: [f64; 4], width: u32, height: u32, aspect: f64, orientation: i32, display: bool) -> [f64; 4] {
    if !display { return framed_rect(bbox, width, height, aspect, orientation); }
    let (w, h) = if (5..=8).contains(&orientation) { (height, width) } else { (width, height) };
    let r = framed_rect(bbox, w, h, aspect, 1);
    let points = [(r[0], r[1]), (r[0] + r[2], r[1]), (r[0], r[1] + r[3]), (r[0] + r[2], r[1] + r[3])];
    let mut bounds = [1.0_f64, 1.0_f64, 0.0_f64, 0.0_f64];
    for (x, y) in points {
        let (x, y) = match orientation {
            2 => (1.0 - x, y), 3 => (1.0 - x, 1.0 - y), 4 => (x, 1.0 - y),
            5 => (y, x), 6 => (y, 1.0 - x), 7 => (1.0 - y, 1.0 - x), 8 => (1.0 - y, x),
            _ => (x, y),
        };
        bounds[0] = bounds[0].min(x); bounds[1] = bounds[1].min(y);
        bounds[2] = bounds[2].max(x); bounds[3] = bounds[3].max(y);
    }
    [bounds[0], bounds[1], bounds[2] - bounds[0], bounds[3] - bounds[1]]
}

// Face boxes are source raw pixels. Frame there, then orient the finished crop.
fn framed_rect(b: [f64; 4], width: u32, height: u32, aspect: f64, orientation: i32) -> [f64; 4] {
    let aspect = if (5..=8).contains(&orientation) { 1.0 / aspect } else { aspect };
    let h = (b[3] * 1.1).max(b[2] * 1.1 / aspect);
    let w = h * aspect;
    let fit = 1.0_f64.min(width as f64 / w).min(height as f64 / h);
    let (w, h) = (w * fit, h * fit);
    let x = (b[0] + b[2] / 2.0 - w / 2.0).clamp(0.0, width as f64 - w);
    let y = (b[1] + b[3] / 2.0 - h / 2.0).clamp(0.0, height as f64 - h);
    [x / width as f64, y / height as f64, w / width as f64, h / height as f64]
}

fn crop_image(img: DynamicImage, rect: [f64; 4], size: u32) -> DynamicImage {
    let (w, h) = img.dimensions();
    let x = ((rect[0] * w as f64).round() as u32).min(w - 1);
    let y = ((rect[1] * h as f64).round() as u32).min(h - 1);
    let cw = ((rect[2] * w as f64).round() as u32).max(1).min(w - x);
    let ch = ((rect[3] * h as f64).round() as u32).max(1).min(h - y);
    img.crop_imm(x, y, cw, ch).thumbnail(size, size)
}

pub fn cached(key: &str) -> Result<Option<Vec<u8>>, String> {
    let started=std::time::Instant::now();
    let canonical = cache_key(key.strip_prefix("face-v4:").ok_or("invalid face cache key")?)?;
    let cached=crate::t_thumb_cache::get_face(&canonical);
    if cfg!(debug_assertions) && std::env::var_os("LAP_FACE_CACHE_TRACE").is_some() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static HITS: AtomicUsize=AtomicUsize::new(0);
        static MISSES: AtomicUsize=AtomicUsize::new(0);
        let counter=if cached.is_some() { &HITS } else { &MISSES };
        if counter.fetch_add(1,Ordering::Relaxed)<32 {
            eprintln!("{}",serde_json::json!({"event":"face_cache_trace", "uri":key,"key":canonical,
                "hit":cached.is_some(),"lookup_ms":started.elapsed().as_secs_f64()*1000.0}));
        }
    }
    Ok(cached.or_else(|| crate::t_thumb_cache::get_face(key)))
}

pub async fn get(key: String, file_id: i64, bbox: [f64; 4], size: u32, aspect: f64, face_id: i64) -> Result<Vec<u8>, String> {
    if let Some(data) = cached(&key)? { return Ok(data); }
    let canonical = cache_key(key.strip_prefix("face-v4:").ok_or("invalid face cache key")?)?;
    let _permit = GENERATION.get_or_init(|| tokio::sync::Semaphore::new(2))
        .acquire().await.map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(data) = crate::t_thumb_cache::get_face(&canonical).or_else(|| crate::t_thumb_cache::get_face(&key)) { return Ok(data); }
        if size == 128 {
            for parent in larger_cache_keys(&key)? {
                if let Some(bytes) = crate::t_thumb_cache::get_face(&parent) {
                    let crop = image::load_from_memory(&bytes).map_err(|e| e.to_string())?.thumbnail(128, 128);
                    let data = crate::t_jpeg::encode_rgb8(&crop.to_rgb8(), 90)?;
                    crate::t_thumb_cache::put_face(&canonical, &data)?;
                    return Ok(data);
                }
            }
        }
        let file = crate::t_sqlite::AFile::get_file_info(file_id)
            .map_err(|e| e.to_string())?.ok_or("face photo not found")?;
        let path = file.file_path.ok_or("face photo has no path")?;
        let conn = crate::t_sqlite::open_conn()?;
        let data = generate(&conn, &path, file_id, bbox, size, aspect, face_id)?;
        crate::t_thumb_cache::put_face(&canonical, &data)?;
        Ok(data)
    }).await.map_err(|e| e.to_string())?
}

fn larger_cache_keys(key: &str) -> Result<Vec<String>, String> {
    let mut url = reqwest::Url::parse(key.strip_prefix("face-v4:").ok_or("invalid face cache key")?)
        .map_err(|e| e.to_string())?;
    let params: Vec<(String, String)> = url.query_pairs().map(|(k,v)| (k.into_owned(),v.into_owned())).collect();
    [256,512,1024].into_iter().map(|size| {
        url.query_pairs_mut().clear().extend_pairs(params.iter().map(|(k,v)|
            (k.clone(), if k == "fs" { size.to_string() } else { v.clone() })));
        cache_key(url.as_str())
    }).collect()
}

fn generate(conn: &rusqlite::Connection, path: &str, file_id: i64, bbox: [f64; 4], size: u32, aspect: f64, face_id: i64) -> Result<Vec<u8>, String> {
        // Resized files can have different geometry from imported/stale library metadata.
        let orientation = crate::t_image::get_image_orientation(&path);
        let (width, height) = image::image_dimensions(&path).map_err(|e| e.to_string())?;
        let display = is_display_box(conn, file_id, face_id)?;
        let edits:Option<String>=conn.query_row("SELECT edits FROM afiles WHERE id=?1",[file_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        let doc=crate::t_edit_recipe::state(edits.as_deref());
        if doc["recipe"].is_object() || doc["auto_enhance"].as_bool().unwrap_or(false) {
            let params=crate::t_edit_recipe::render_params(file_id,path,edits.as_deref(),false)?;
            let (mapped,dims)=crate::t_edit_recipe::map_bbox(bbox,width,height,orientation,display,edits.as_deref())
                .ok_or("Face is outside the edited photo")?;
            let img=crate::t_image::apply_orientation(image::open(path).map_err(|e|e.to_string())?,orientation);
            let edited=crate::t_image::apply_edit_params(img,&params)?;
            let rect=framed_rect(mapped,dims[0],dims[1],aspect,1);
            return crate::t_jpeg::encode_rgb8(&crop_image(edited,rect,size).to_rgb8(),90);
        }
        let (box_width, box_height) = if display && (5..=8).contains(&orientation) { (height, width) } else { (width, height) };
        if bbox[0] + bbox[2] <= 0.0 || bbox[1] + bbox[3] <= 0.0
            || bbox[0] >= box_width as f64 || bbox[1] >= box_height as f64 {
            return Err("face box does not intersect its photo".into());
        }
        let rect = source_rect(bbox, width, height, aspect, orientation, display);
        let img = if path.to_ascii_lowercase().ends_with(".jpg") || path.to_ascii_lowercase().ends_with(".jpeg") {
            let target = (size as f64 / rect[2].min(rect[3])).ceil().min(u32::MAX as f64) as u32;
            let (pixels, w, h) = crate::t_jpeg::decode_rgb8_scaled(&path, target, target)?;
            DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, pixels).ok_or("invalid JPEG pixels")?)
        } else {
            image::open(&path).map_err(|e| e.to_string())?
        };
        let crop = crate::t_image::apply_orientation(crop_image(img, rect, size), orientation).to_rgb8();
        crate::t_jpeg::encode_rgb8(&crop, 90)
}

#[derive(Clone)]
struct PendingCrop { face_id: i64, file_id: i64, path: String, bbox: [f64; 4], key: String, error: Option<String> }

fn named_crops(conn: &rusqlite::Connection, library_id: &str, size: u32, aspect: f64, after: i64) -> Result<Vec<PendingCrop>, String> {
    let mut stmt = conn.prepare("SELECT f.id, f.file_id, f.bbox, d.path, a.name, COALESCE(a.modified_at,0), a.edits
        FROM faces f JOIN afiles a ON a.id=f.file_id JOIN afolders d ON d.id=a.folder_id
        WHERE f.id > ?1 AND f.id NOT IN (SELECT face_id FROM face_ignores)
        AND EXISTS (SELECT 1 FROM persons p WHERE p.is_anchored=1 AND length(trim(p.name))>0
            AND (p.id=f.person_id OR p.id=f.suggested_person_id)) ORDER BY f.id LIMIT 64").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([after], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?,
        r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, i64>(5)?, r.get::<_, Option<String>>(6)?))).map_err(|e| e.to_string())?;
    rows.map(|row| {
        let (face_id, file_id, json, folder, name, version, edits) = row.map_err(|e| e.to_string())?;
        let b: serde_json::Value = serde_json::from_str(&json).unwrap_or(serde_json::Value::Null);
        let bbox = ["x", "y", "width", "height"].map(|k| b[k].as_f64().unwrap_or(f64::NAN));
        let path = std::path::PathBuf::from(folder).join(name).to_string_lossy().into_owned();
        let mut url = reqwest::Url::parse(&format!("lap://localhost/{library_id}/{file_id}")).map_err(|e| e.to_string())?;
        {
            let mut params = url.query_pairs_mut();
            for (key, value) in ["fx", "fy", "fw", "fh"].into_iter().zip(bbox) { params.append_pair(key, &value.to_string()); }
            params.append_pair("fa", &aspect.to_string()).append_pair("fid", &face_id.to_string())
                .append_pair("fs", &size.to_string()).append_pair("fp", &path);
            if let Some(space) = b["coordinate_space"].as_str() { params.append_pair("fc", space); }
            if version > 0 { params.append_pair("v", &version.to_string()); }
        }
        let doc=crate::t_edit_recipe::state(edits.as_deref());
        if let Some(render_key)=doc["render_key"].as_str() {
            url.query_pairs_mut().append_pair("rk",render_key);
        }
        let (key, error) = match cache_key(url.as_str()) {
            Ok(key) => (key, None), Err(error) => (String::new(), Some(error)),
        };
        Ok(PendingCrop { face_id, file_id, path, bbox, key, error })
    }).collect()
}

#[derive(Default, serde::Serialize)]
pub struct PreGenerateStats { pub eligible: usize, pub cached_before: usize, pub generated: usize, pub failed: usize, pub cached_after: usize }

pub fn pre_generate(db: &std::path::Path, library_id: &str, cache: &crate::t_thumb_cache::FaceCache,
    size: u32, aspect: f64, workers: usize, pause_ms: u64, status_only: bool,
    progress: impl Fn(&PreGenerateStats)) -> Result<PreGenerateStats, String> {
    if !(1..=4).contains(&workers) || ![256,512,1024].contains(&size) || !(0.5..=2.0).contains(&aspect) {
        return Err("workers must be 1..4, size 256/512/1024 and aspect 0.5..2".into());
    }
    let conn = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(10)).map_err(|e| e.to_string())?;
    cache.migrate_face_keys()?;
    let mut stats = PreGenerateStats::default();
    let mut after = 0;
    loop {
        let batch = named_crops(&conn, library_id, size, aspect, after)?;
        if batch.is_empty() { break; }
        after = batch.last().unwrap().face_id;
        stats.eligible += batch.len();
        let mut missing = Vec::new();
        for crop in batch {
            if let Some(error) = &crop.error {
                stats.failed += 1;
                eprintln!("{}", serde_json::json!({"event":"face_crop_error", "face_id":crop.face_id, "error":error}));
                continue;
            }
            if cache.contains(&crop.key)? { stats.cached_before += 1; }
            else { missing.push(crop); }
        }
        if !status_only {
            for chunk in missing.chunks(workers) {
                let results = std::thread::scope(|scope| {
                    let handles = chunk.iter().map(|crop| scope.spawn(move || {
                        let conn = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
                        generate(&conn, &crop.path, crop.file_id, crop.bbox, size, aspect, crop.face_id)
                            .map(|bytes| (crop.key.clone(), bytes))
                    })).collect::<Vec<_>>();
                    handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("face crop worker panicked".into()))).collect::<Vec<_>>()
                });
                let mut entries = Vec::new();
                for (crop, result) in chunk.iter().zip(results) {
                    match result {
                        Ok(entry) => entries.push(entry),
                        Err(error) => { stats.failed += 1; eprintln!("{}", serde_json::json!({"event":"face_crop_error", "face_id":crop.face_id, "error":error})); }
                    }
                }
                cache.put_many(&entries)?;
                stats.generated += entries.len();
                std::thread::sleep(std::time::Duration::from_millis(pause_ms));
            }
        }
        stats.cached_after = stats.cached_before + stats.generated;
        progress(&stats);
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn small_crop_reuses_only_matching_larger_cache_keys() {
        let uri = "http://lap.localhost/library/12?fx=20&fy=10&fw=40&fh=50&fs=128&fa=0.8333333333333334&fid=7&fp=C%3A%5Cphotos%5Ca.jpg&fc=raw&v=23&rk=edit123";
        let keys = larger_cache_keys(&format!("face-v4:{uri}")).unwrap();
        for (key, size) in keys.iter().zip([256,512,1024]) {
            assert_eq!(*key, cache_key(&uri.replace("fs=128", &format!("fs={size}"))).unwrap());
            assert_ne!(*key, cache_key(&uri.replace("rk=edit123", "rk=other").replace("fs=128", &format!("fs={size}"))).unwrap());
        }
        let source = DynamicImage::ImageRgb8(image::RgbImage::from_fn(426,512,|x,y| image::Rgb([x as u8,y as u8,100])));
        let bytes = crate::t_jpeg::encode_rgb8(&source.to_rgb8(),90).unwrap();
        let smaller = image::load_from_memory(&bytes).unwrap().thumbnail(128,128);
        assert_eq!(smaller.dimensions(), (107,128));
    }
    #[test]
    #[ignore = "Explicit TEST runtime-key audit; app must be closed"]
    fn observed_runtime_keys_hit_test_cache() {
        let conn=rusqlite::Connection::open_with_flags("C:/lapcasa-test/library/library.db",rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let cache=crate::t_thumb_cache::FaceCache::open(std::path::Path::new("C:/lapcasa-test/profile/thumbs.redb")).unwrap();
        let mut crops=std::collections::HashMap::new();
        let mut after=0;
        loop {
            let batch=named_crops(&conn,"library",512,5.0/6.0,after).unwrap();
            if batch.is_empty() { break; }
            after=batch.last().unwrap().face_id;
            for crop in batch { crops.insert(crop.face_id,crop.key); }
        }
        let log=std::fs::read_to_string("C:/lapcasa-test/face-cache-runtime.log").unwrap();
        for line in log.lines().filter(|line| line.contains("face_cache_trace")) {
            let trace:serde_json::Value=serde_json::from_str(line).unwrap();
            let actual=trace["key"].as_str().unwrap();
            let parts:serde_json::Value=serde_json::from_str(actual.strip_prefix("face-v5:").unwrap()).unwrap();
            let id=parts[5].as_i64().unwrap();
            let expected=crops.get(&id).unwrap();
            assert_eq!(actual,expected,"face {id}");
            assert!(cache.contains(actual).unwrap(),"face {id} must be cached");
        }
    }
    #[test]
    fn crop_keys_match_across_protocol_schemes_and_parameter_order() {
        let a = "http://lap.localhost/library/12?fx=20&fy=10&fw=40&fh=50&fs=512&fa=0.8333333333333334&fid=7&fp=C%3A%5Cphotos%5Ca.jpg&fc=raw&v=23";
        let b = "lap://localhost/library/12?v=23&fc=raw&fp=C%3A%5Cphotos%5Ca.jpg&fid=7&fa=0.8333333333333334&fs=512&fh=50&fw=40&fy=10&fx=20";
        assert_eq!(cache_key(a).unwrap(), cache_key(b).unwrap());
        for other in [a.replace("v=23", "v=24"), a.replace("fc=raw", "fc=display"),
            a.replace("fs=512", "fs=256"), a.replace("fid=7", "fid=8"), a.replace("/library/", "/other/")] {
            assert_ne!(cache_key(a).unwrap(), cache_key(&other).unwrap());
        }
    }
    #[test]
    fn sql_crop_matches_the_observed_browser_request() {
        let conn=rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE persons(id INTEGER,name TEXT,is_anchored INTEGER);
            CREATE TABLE afolders(id INTEGER,path TEXT); CREATE TABLE afiles(id INTEGER,folder_id INTEGER,name TEXT,modified_at INTEGER,edits TEXT);
            CREATE TABLE faces(id INTEGER,file_id INTEGER,bbox TEXT,person_id INTEGER,suggested_person_id INTEGER);
            CREATE TABLE face_ignores(face_id INTEGER PRIMARY KEY,ignored_at INTEGER);
            INSERT INTO persons VALUES(4432,'Example Person',1);
            INSERT INTO afiles VALUES(29211,1,'sample.jpg',1767436297,NULL);
            INSERT INTO faces VALUES(28791,29211,'{\"x\":406.95491,\"y\":330.098787,\"width\":646.043793,\"height\":777.136858,\"confidence\":1.0}',4432,NULL);").unwrap();
        conn.execute("INSERT INTO afolders VALUES(1,?1)",["C:\\Photos\\Fixture"]).unwrap();
        let crops=named_crops(&conn,"library",512,5.0/6.0,0).unwrap();
        let browser="http://lap.localhost/library/29211?fx=406.95491&fy=330.098787&fw=646.043793&fh=777.136858&fa=0.8333333333333334&fid=28791&fs=512&fp=C%3A%5CPhotos%5CFixture%5Csample.jpg&v=1767436297";
        assert_eq!(crops[0].key,cache_key(browser).unwrap());
        // Captured detector coordinates must survive JSON parsing bit-for-bit;
        // a one-ULP difference otherwise bypasses a pre-generated crop.
        conn.execute("UPDATE faces SET bbox=?1",[r#"{"x":965.0623779296875,"y":491.19256591796875,"width":214.7691650390625,"height":278.10205078125}"#]).unwrap();
        let exact=browser.replace("406.95491","965.0623779296875").replace("330.098787","491.19256591796875")
            .replace("646.043793","214.7691650390625").replace("777.136858","278.10205078125");
        assert_eq!(named_crops(&conn,"library",512,5.0/6.0,0).unwrap()[0].key,cache_key(&exact).unwrap());
        if let Ok(path) = std::env::var("LAP_FACE_PARITY_CACHE") {
            let expected=std::path::Path::new("C:/lapcasa-test/profile/thumbs.redb");
            assert_eq!(std::fs::canonicalize(&path).unwrap(),std::fs::canonicalize(expected).unwrap());
            let cache=crate::t_thumb_cache::FaceCache::open(std::path::Path::new(&path)).unwrap();
            assert!(cache.contains(&cache_key(browser).unwrap()).unwrap(),"captured request must hit the actual TEST redb table");
        }
    }

    #[test]
    fn pre_generation_resumes_and_reports_failed_sources_without_touching_labels() {
        let root = std::env::temp_dir().join(format!("lap-face-cache-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let db = root.join("library.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE persons(id INTEGER,name TEXT,is_anchored INTEGER);
            CREATE TABLE afolders(id INTEGER,path TEXT);
            CREATE TABLE afiles(id INTEGER,folder_id INTEGER,name TEXT,modified_at INTEGER,edits TEXT);
            CREATE TABLE faces(id INTEGER,file_id INTEGER,bbox TEXT,person_id INTEGER,suggested_person_id INTEGER);
            CREATE TABLE face_ignores(face_id INTEGER PRIMARY KEY,ignored_at INTEGER);
            INSERT INTO persons VALUES(1,'Named',1),(2,'Person 2',0);
            INSERT INTO afiles VALUES(1,1,'source.png',123,NULL),(2,1,'missing.png',123,NULL);
            INSERT INTO faces VALUES(1,1,'{\"x\":20,\"y\":10,\"width\":40,\"height\":50}',1,NULL),
                (2,1,'{\"x\":20,\"y\":10,\"width\":40,\"height\":50}',NULL,1),
                (3,2,'{\"x\":20,\"y\":10,\"width\":40,\"height\":50}',1,NULL),
                (4,1,'{}',2,NULL), (5,1,'{}',1,NULL);").unwrap();
        conn.execute("INSERT INTO afolders VALUES(1,?1)", [root.to_string_lossy().as_ref()]).unwrap();
        image::RgbImage::from_pixel(100,100,image::Rgb([20,180,40])).save(root.join("source.png")).unwrap();
        let cache_path = root.join("thumbs.redb");
        let cache = crate::t_thumb_cache::FaceCache::open(&cache_path).unwrap();
        let before = pre_generate(&db,"library",&cache,512,5.0/6.0,2,0,true, |_|{}).unwrap();
        assert_eq!(before.eligible,4); assert_eq!(before.cached_before,0);
        let first = pre_generate(&db,"library",&cache,512,5.0/6.0,2,0,false, |_|{}).unwrap();
        assert_eq!((first.generated,first.failed,first.cached_after),(2,2,2));
        let resumed = pre_generate(&db,"library",&cache,512,5.0/6.0,2,0,false, |_|{}).unwrap();
        assert_eq!((resumed.cached_before,resumed.generated,resumed.failed),(2,0,2));
        assert_eq!(conn.query_row("SELECT person_id FROM faces WHERE id=1",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert!(pre_generate(&db,"library",&cache,512,5.0/6.0,5,0,false, |_|{}).is_err());
        drop(cache); drop(conn);
        for name in ["library.db","source.png","thumbs.redb"] { std::fs::remove_file(root.join(name)).unwrap(); }
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn face_crop_selects_correct_region_and_bounds_output() {
        let mut img = image::RgbImage::from_pixel(1000, 600, image::Rgb([255, 0, 0]));
        for y in 0..600 { for x in 500..1000 { img.put_pixel(x, y, image::Rgb([0, 255, 0])); } }
        let crop = crop_image(DynamicImage::ImageRgb8(img), [0.5, 0.0, 0.5, 1.0], 256).to_rgb8();
        assert_eq!(crop.dimensions(), (213, 256));
        assert_eq!(crop.get_pixel(100, 100).0, [0, 255, 0]);
    }
    #[test]
    fn rejects_invalid_crops() {
        assert!(parse_crop("fx=984&fy=526&fw=134&fh=171&fs=512&fa=0.8333333").is_some());
        // Detectors can place a border face slightly outside the photo. Framing
        // clips it using the real image dimensions; the face label is unchanged.
        assert!(parse_crop("fx=-1.739538550376892&fy=97&fw=110&fh=188&fs=512&fa=1").is_some());
        for query in ["fx=NaN&fy=0&fw=100&fh=100&fs=512&fa=1", "fx=0&fy=0&fw=-1&fh=100&fs=512&fa=1", "fx=0&fy=0&fw=0&fh=100&fs=512&fa=1"] {
            assert!(parse_crop(query).is_none());
        }
    }
    #[test]
    fn border_face_is_clipped_without_changing_geometry() {
        let rect = source_rect([-1.7395,97.0,109.9255,187.625],640,480,5.0/6.0,1,true);
        assert_eq!(rect[0],0.0);
        assert!(rect.iter().all(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0));
        let crop = crop_image(DynamicImage::ImageRgb8(image::RgbImage::from_pixel(640,480,image::Rgb([0,255,0]))),rect,512);
        assert!(crop.width()>0 && crop.height()>0);
    }

    #[test]
    fn frames_raw_face_using_actual_dimensions_and_orientation() {
        let bbox = [984.302, 526.003, 133.989, 170.978];
        let rect = framed_rect(bbox, 2048, 1536, 5.0 / 6.0, 1);
        assert!(((rect[0] + rect[2] / 2.0) * 2048.0 - 1051.2965).abs() < 0.01);
        assert!(((rect[1] + rect[3] / 2.0) * 1536.0 - 611.492).abs() < 0.01);
        for orientation in 1..=8 {
            let rect = framed_rect([200.0, 100.0, 80.0, 100.0], 1000, 600, 5.0 / 6.0, orientation);
            let mut img = image::RgbImage::from_pixel(1000, 600, image::Rgb([255, 0, 0]));
            for y in 100..200 { for x in 200..280 { img.put_pixel(x, y, image::Rgb([0, 255, 0])); } }
            let crop = crate::t_image::apply_orientation(crop_image(DynamicImage::ImageRgb8(img), rect, 256), orientation).to_rgb8();
            assert!((crop.width() as f64 / crop.height() as f64 - 5.0 / 6.0).abs() < 0.02);
            assert_eq!(crop.get_pixel(crop.width() / 2, crop.height() / 2).0, [0, 255, 0]);
        }
    }

    #[test]
    fn upright_detector_box_maps_back_to_raw_pixels() {
        let r = source_rect([497.128, 556.699, 329.671, 426.153], 2048, 1536, 5.0 / 6.0, 8, true);
        assert!(((r[0] + r[2] / 2.0) * 2048.0 - (2048.0 - 769.7755)).abs() < 0.01);
        assert!(((r[1] + r[3] / 2.0) * 1536.0 - 661.9635).abs() < 0.01);
    }

    #[test]
    fn coordinate_provenance_preserves_imports_and_explicit_detector_spaces() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE faces(id INTEGER, file_id INTEGER, bbox TEXT);
            INSERT INTO faces VALUES(1,10,'{}'),(2,10,'{}'),(3,10,'{\"coordinate_space\":\"raw\"}');").unwrap();
        assert!(is_display_box(&conn, 10, 1).unwrap());
        assert!(!is_display_box(&conn, 10, 3).unwrap());
        conn.execute_batch("CREATE TABLE picasa_import_faces(face_id INTEGER); INSERT INTO picasa_import_faces VALUES(2);").unwrap();
        assert!(is_display_box(&conn, 10, 1).unwrap());
        assert!(!is_display_box(&conn, 10, 2).unwrap());
    }

    #[tokio::test]
    #[ignore = "Explicit TEST-library benchmark; app must be closed to unlock the index"]
    async fn benchmark_test_library_cache() {
        crate::t_config::set_app_identifier("com.lapcasa.dev");
        let config = crate::t_config::load_app_config().unwrap();
        assert!(config.db_storage_dir.as_deref().unwrap().replace('\\', "/").ends_with("/test-library"));
        assert!(!config.process_library_in_background);
        let id = std::env::var("LAP_FACE_BENCH_ID").unwrap().parse::<i64>().unwrap();
        let bbox = std::env::var("LAP_FACE_BENCH_BOX").ok()
            .map(|json| serde_json::from_str::<[f64; 4]>(&json).unwrap())
            .unwrap_or([400.0, 500.0, 200.0, 300.0]);
        let face_id = std::env::var("LAP_FACE_BENCH_FACE_ID").ok().map(|id| id.parse().unwrap()).unwrap_or(0);
        let key = format!("face-v4:lap://localhost/library/{id}?fx={}&fy={}&fw={}&fh={}&fs=512&fa={}&fid={face_id}&v={}",
            bbox[0],bbox[1],bbox[2],bbox[3],5.0/6.0,std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
        let start = std::time::Instant::now();
        let bytes = get(key.clone(), id, bbox, 512, 5.0 / 6.0, face_id).await.unwrap();
        let cold = start.elapsed();
        let image = image::load_from_memory(&bytes).unwrap();
        if let Ok(path) = std::env::var("LAP_FACE_BENCH_OUT") { std::fs::write(path, &bytes).unwrap(); }
        assert!(image.width() <= 512 && image.height() <= 512);
        let start = std::time::Instant::now();
        for _ in 0..100 {
            // Invalid ID proves a cache hit never looks up the file or decodes its original.
            assert_eq!(get(key.clone(), -1, bbox, 512, 5.0 / 6.0, face_id).await.unwrap(), bytes);
        }
        eprintln!("FACE CACHE: cold {:?}; warm mean {:?}; {} bytes; {:?}", cold, start.elapsed() / 100, bytes.len(), image.dimensions());
    }
}
