use lap_casa::t_search::*;
use rusqlite::Connection;
use serde_json::json;
use std::env;
use std::path::PathBuf;

fn main() {
    // Share the GUI's private GPU runtime directory as well as its library profile.
    lap_casa::t_config::set_app_identifier("com.lapcasa.dev");
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: lap-cli <search [QUERY] [FILTERS] | stats [--db PATH] | people [QUERY] [--db PATH] [--limit 100] | ocr PATH | objects PATH | ai-profile PATH PROFILE_DIR | ai-status | ai-all | ocr-all | objects-all> [MAX] [--threads N] [--pause-ms N]\n       lap-cli face-redetect --db PATH (--ids 1,2,3 | --missed) [--limit N] [--models DIR] [--dry-run]\n       lap-cli face-thumbs --db PATH --cache PATH [--size 512] [--aspect 0.8333333333333334] [--workers 2] [--pause-ms 25] [--status]");
        std::process::exit(1);
    }

    let command = &args[1];

    match command.as_str() {
        "slab-migrate" => {
            if let Err(error) = run_slab_migrate(&args[2..]) {
                eprintln!("{}", json!({"event": "fatal", "error": error}));
                std::process::exit(1);
            }
        }
        "photo-thumbs" => {
            if let Err(error) = run_photo_thumbs(&args[2..]) {
                eprintln!("{}",json!({"event":"fatal","error":error}));
                std::process::exit(1);
            }
        }
        "face-thumbs" => {
            if let Err(error) = run_face_thumbs(&args[2..]) {
                eprintln!("{}", json!({"event":"fatal", "error":error}));
                std::process::exit(1);
            }
        }
        "search" | "stats" | "people" | "cameras" => {
            if let Err(error) = run_search(&args[2..], command) {
                println!("{}", json!({"ok": false, "error": error}));
                std::process::exit(1);
            }
        }
        "ocr" => {
            if args.len() < 3 {
                eprintln!("Usage: lap-cli ocr <path>");
                std::process::exit(1);
            }
            let path = &args[2];
            run_ocr(path);
        }
        "ai-status" | "ai-all" | "ocr-all" | "objects-all" => {
            if let Err(e) = run_bulk(command, &args[2..]) {
                eprintln!("{}", json!({"event": "fatal", "error": e}));
                std::process::exit(1);
            }
        }
        "ai-profile" => {
            let result = if args.len() == 4 { profile_image(&args[2], &args[3]) }
                else { Err("Usage: lap-cli ai-profile PATH PROFILE_DIR".into()) };
            if let Err(error) = result {
                eprintln!("{}", json!({"event": "fatal", "error": error}));
                std::process::exit(1);
            }
        }
        "face-redetect" => {
            if let Err(e) = run_face_redetect(&args[2..]) {
                eprintln!("{}", json!({"error": e}));
                std::process::exit(1);
            }
        }
        "face-suggest" => {
            if let Err(e) = run_face_suggest(&args[2..]) {
                eprintln!("{}", json!({"event": "fatal", "error": e}));
                std::process::exit(1);
            }
        }
        "objects" => {
            let path = args.get(2).expect("Usage: lap-cli objects <path>");
            let mut engine = lap_casa::t_advanced_ai::ObjectEngine::load(
                &lap_casa::t_advanced_ai::default_models_dir()).expect("object model");
            println!("{}", json!({"objects": engine.detect(path).expect("object detection")}));
        }
        _ => {
            eprintln!("Unknown command: {}", command);
            std::process::exit(1);
        }
    }
}

fn run_slab_migrate(args: &[String]) -> Result<(), String> {
    let (mut redb, mut slabs) = (None, None);
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--redb" => redb = Some(PathBuf::from(args.next().ok_or("missing --redb")?)),
            "--slabs" => slabs = Some(PathBuf::from(args.next().ok_or("missing --slabs")?)),
            "--help" => {
                println!("Usage: lap-cli slab-migrate --redb PATH --slabs DIR");
                return Ok(());
            }
            _ => return Err(format!("unknown option {flag}")),
        }
    }
    let redb = redb.ok_or("--redb PATH required")?;
    let slabs_dir = slabs.ok_or("--slabs DIR required")?;
    let cache = lap_casa::slabs::SlabCache::open(&slabs_dir)?;
    let count = cache.migrate_from_redb(&redb)?;
    println!("{}", json!({"status": "ok", "migrated": count, "slabs_dir": slabs_dir.display().to_string()}));
    Ok(())
}

// Explicit paths prevent a maintenance command from touching the wrong library.
fn run_photo_thumbs(args: &[String]) -> Result<(), String> {
    use std::io::Write;
    let (mut db,mut path,mut journal)=(None,None,None);
    let (mut workers,mut pause_ms)=(2usize,25u64);
    let mut status = false;
    let mut args=args.iter();
    while let Some(flag)=args.next() {
        match flag.as_str() {
            "--help"=>{println!("lap-cli photo-thumbs --db PATH --cache PATH [--status | --journal PATH] [--workers 1..4] [--pause-ms 25]\nClose the app first. --status inspects coverage without regenerating images. Upgrade orientation versions, retain unaffected bytes, regenerate PRIMARY mismatches. Completed entries are skipped on restart. Journal only changed rows/entries (64MB cap); Windows requires 20GB free before each batch.");return Ok(());}
            "--status"=>status=true,
            "--db"=>db=Some(PathBuf::from(args.next().ok_or("missing --db")?)),
            "--cache"=>path=Some(PathBuf::from(args.next().ok_or("missing --cache")?)),
            "--journal"=>journal=Some(PathBuf::from(args.next().ok_or("missing --journal")?)),
            "--workers"=>workers=args.next().and_then(|v|v.parse().ok()).ok_or("invalid workers")?,
            "--pause-ms"=>pause_ms=args.next().and_then(|v|v.parse().ok()).ok_or("invalid pause")?,
            _=>return Err(format!("unknown photo-thumbs option {flag}")),
        }
    }
    if !(1..=4).contains(&workers) {return Err("workers must be 1..4".into());}
    let db=db.ok_or("--db PATH required")?;let path=path.ok_or("--cache PATH required; close app first")?;
    if status {
        let conn = Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
        let mut query = conn.prepare("SELECT file_id FROM athumbs WHERE error_code=0 ORDER BY file_id").map_err(|e| e.to_string())?;
        let ids = query.query_map([], |r| r.get::<_, i64>(0)).map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        println!("{}", lap_casa::t_thumb_cache::PhotoCache::coverage(&path, &ids)?);
        return Ok(());
    }
    let journal=journal.ok_or("--journal PATH required for capped changed-entry snapshots")?;
    if !db.is_file() || !path.is_file() {return Err("database and cache must exist".into());}
    require_disk_space(&db)?;require_disk_space(&path)?;
    std::fs::create_dir_all(&journal).map_err(|e|e.to_string())?;
    require_disk_space(&journal)?;
    let manifest=journal.join("metadata.jsonl");
    let mut recorded=std::collections::HashSet::new();
    if manifest.exists() {
        for line in std::fs::read_to_string(&manifest).map_err(|e|e.to_string())?.lines() {
            let row:serde_json::Value=serde_json::from_str(line).map_err(|e|e.to_string())?;
            recorded.insert(row["id"].as_i64().ok_or("invalid journal row")?);
        }
    }
    let mut journal_bytes=std::fs::read_dir(&journal).map_err(|e|e.to_string())?.try_fold(0u64,|sum,entry| {
        entry.and_then(|e|e.metadata()).map(|m|sum+m.len())
    }).map_err(|e|e.to_string())?;
    let library=db.file_stem().and_then(|v|v.to_str()).ok_or("invalid library name")?.to_string();
    let mut conn=Connection::open(&db).map_err(|e|e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(10)).map_err(|e|e.to_string())?;
    let cache=lap_casa::t_thumb_cache::PhotoCache::open(&path)?;
    let rows={
        let mut q=conn.prepare("SELECT a.id,f.path,a.name,a.file_type,COALESCE(a.e_orientation,1),t.thumb_size,t.thumb_key,t.thumb_mtime,a.width,a.height FROM afiles a JOIN afolders f ON f.id=a.folder_id JOIN athumbs t ON t.file_id=a.id WHERE t.error_code=0 ORDER BY a.id").map_err(|e|e.to_string())?;
        let rows=q.query_map([],|r|Ok((r.get::<_,i64>(0)?,PathBuf::from(r.get::<_,String>(1)?).join(r.get::<_,String>(2)?),r.get::<_,i64>(3)?,r.get::<_,i32>(4)?,r.get::<_,Option<u32>>(5)?.unwrap_or(512),r.get::<_,Option<String>>(6)?.unwrap_or_default(),r.get::<_,Option<i64>>(7)?,r.get::<_,Option<u32>>(8)?,r.get::<_,Option<u32>>(9)?))).map_err(|e|e.to_string())?;
        rows.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?
    };
    let (mut generated,mut retained,mut skipped,mut failed)=(0usize,0usize,0usize,0usize);
    let started=std::time::Instant::now();
    println!("{}",json!({"event":"photo_thumbs_start","db":db,"cache":path,"total":rows.len(),"workers":workers,"pause_ms":pause_ms}));
    for chunk in rows.chunks(64) {
        require_disk_space(&path)?;require_disk_space(&db)?;require_disk_space(&journal)?;
        let mut pending=Vec::new();
        for row in chunk {
            if row.5.starts_with(lap_casa::t_image::ORIENTATION_CACHE_PREFIX) && cache.is_current(row.0)? {skipped+=1;} else {pending.push(row);}
        }
        let results=std::thread::scope(|scope| {
            let mut handles=Vec::new();
            for lane in 0..workers {
                let work=pending.iter().skip(lane).step_by(workers).copied().collect::<Vec<_>>();
                let cache=&cache;let library=&library;
                handles.push(scope.spawn(move ||work.into_iter().map(|row| {
                    let result=(||->Result<_,String>{
                        let (id,path,kind,stored,size,key,mtime,_,_)=row;
                        let file=path.to_str().ok_or("invalid photo path")?;
                        let heif=path.extension().is_some_and(|e|matches!(e.to_string_lossy().to_ascii_lowercase().as_str(),"heic"|"heif"|"hif"));
                        let heif_geometry=if heif && (2..=8).contains(stored) {
                            use std::io::Read;
                            let mut header=[0u8;12];
                            std::fs::File::open(path).and_then(|mut f|f.read_exact(&mut header)).map_err(|e|e.to_string())?;
                            &header[4..8]==b"ftyp"
                        }else{heif};
                        // The old reader's 0/1 both leave pixels upright. Only
                        // 2..8 could have borrowed an IFD1 rotation. HEIF's
                        // decoder owns irot/imir and does not use this reader.
                        let orientation=if !heif_geometry && *kind!=2 && (2..=8).contains(stored) {lap_casa::t_image::get_image_orientation(file)} else {if (1..=8).contains(stored){*stored}else{1}};
                        let changed=orientation!=if (1..=8).contains(stored){*stored}else{1};
                        let missing=!cache.contains_legacy(*id)?;
                        let data=if changed||missing {
                            let bytes=match kind {
                                1 if heif=>lap_casa::t_heif::get_heif_thumbnail(file,orientation,*size)?,
                                1=>lap_casa::t_image::get_image_thumbnail(file,orientation,*size)?,
                                3=>lap_casa::t_image::get_raw_thumbnail(file,orientation,*size,key.strip_prefix(lap_casa::t_image::ORIENTATION_CACHE_PREFIX).unwrap_or(key).starts_with('e'))?,
                                _=>return Err("missing unaffected non-image thumbnail; use regular generator".into()),
                            }.ok_or("decoder returned no thumbnail")?;
                            std::thread::sleep(std::time::Duration::from_millis(pause_ms));Some(bytes)
                        } else {None};
                        let dimensions=if changed {image::ImageReader::open(path).ok().and_then(|r|r.with_guessed_format().ok()).and_then(|r|r.into_dimensions().ok()).map(|(w,h)|if (5..=8).contains(&orientation){(h,w)}else{(w,h)})}else{None};
                        let new_key=lap_casa::t_sqlite::AThumb::build_thumb_key(library,*id,*size,*mtime,orientation,(*kind==3).then_some(key.strip_prefix(lap_casa::t_image::ORIENTATION_CACHE_PREFIX).unwrap_or(key).starts_with('e')));
                        Ok((*id,data,orientation,dimensions,new_key))
                    })();
                    if let Err(ref error)=result {eprintln!("{}",json!({"event":"photo_thumb_error","id":row.0,"error":error}));}
                    result
                }).collect::<Vec<_>>()));
            }
            handles.into_iter().flat_map(|h|h.join().expect("photo thumbnail worker")).collect::<Vec<_>>()
        });
        let mut completed=Vec::new();
        for result in results {
            match result {Ok(value)=>completed.push(value),Err(_)=>failed+=1}
        }
        // Commit bytes/version first, metadata second. A crash between them
        // repeats this batch safely; completed pairs skip on the next run.
        let entries=completed.iter_mut().map(|(id,data,_,_,_)|(*id,data.take())).collect::<Vec<_>>();
        let mut log=std::fs::OpenOptions::new().create(true).append(true).open(&manifest).map_err(|e|e.to_string())?;
        for (id,data) in &entries {
            let original=chunk.iter().find(|r|r.0==*id).ok_or("missing source row")?;
            let backup=journal.join(format!("thumb-{id}.bin"));
            let old_bytes=if data.is_some() && !backup.exists() {cache.legacy_bytes(*id)?} else {None};
            let row=if recorded.contains(id) {String::new()} else {format!("{}\n",json!({"id":id,"thumb_key":original.5,"orientation":original.3,"width":original.7,"height":original.8,"cache_version":cache.version(*id)?}))};
            let extra=row.len() as u64+old_bytes.as_ref().map_or(0,|v|v.len() as u64);
            if journal_bytes+extra>64*1024*1024 {return Err("changed-entry journal would exceed 64MB; no further cache changes".into());}
            if let Some(bytes)=old_bytes {std::fs::write(backup,bytes).map_err(|e|e.to_string())?;}
            if !row.is_empty() {log.write_all(row.as_bytes()).map_err(|e|e.to_string())?;recorded.insert(*id);}
            journal_bytes+=extra;
        }
        log.sync_all().map_err(|e|e.to_string())?;
        generated+=entries.iter().filter(|(_,data)|data.is_some()).count();
        retained+=entries.iter().filter(|(_,data)|data.is_none()).count();
        cache.certify_many(&entries)?;
        let tx=conn.transaction().map_err(|e|e.to_string())?;
        for (id,_,orientation,dimensions,key) in completed {
            tx.execute("UPDATE athumbs SET thumb_key=?2 WHERE file_id=?1",rusqlite::params![id,key]).map_err(|e|e.to_string())?;
            if let Some((w,h))=dimensions {tx.execute("UPDATE afiles SET e_orientation=?2,width=?3,height=?4 WHERE id=?1",rusqlite::params![id,orientation,w,h]).map_err(|e|e.to_string())?;}
        }
        tx.commit().map_err(|e|e.to_string())?;
        if (generated+retained+skipped)%4096<64 {println!("{}",json!({"event":"photo_thumbs_progress","generated":generated,"retained":retained,"skipped":skipped,"failed":failed,"elapsed_s":started.elapsed().as_secs_f64()}));}
        std::thread::sleep(std::time::Duration::from_millis(pause_ms));
    }
    println!("{}",json!({"event":"photo_thumbs_finish","generated":generated,"retained":retained,"skipped":skipped,"failed":failed,"journal_bytes":journal_bytes,"elapsed_s":started.elapsed().as_secs_f64()}));
    if failed>0 {return Err(format!("{failed} failed entries remain pending"));} Ok(())
}

fn require_disk_space(path:&std::path::Path)->Result<(),String> {
    #[cfg(target_os="windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name="kernel32")]
        unsafe extern "system" {fn GetDiskFreeSpaceExW(path:*const u16,available:*mut u64,total:*mut u64,free:*mut u64)->i32;}
        let directory=if path.is_dir(){path}else{path.parent().ok_or("invalid disk path")?};
        let name=directory.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
        let mut available=0;
        if unsafe{GetDiskFreeSpaceExW(name.as_ptr(),&mut available,std::ptr::null_mut(),std::ptr::null_mut())}==0 {return Err(std::io::Error::last_os_error().to_string());}
        if available<20*1024*1024*1024 {return Err(format!("{} has {:.2}GB free; 20GB minimum, stopped before cache operation",directory.display(),available as f64/1024.0f64.powi(3)));}
    }
    let _=path;
    Ok(())
}

fn run_face_thumbs(args: &[String]) -> Result<(), String> {
    let (mut db, mut cache) = (None, None);
    let (mut size, mut aspect, mut workers, mut pause_ms, mut status) = (512, 5.0 / 6.0, 2, 25, false);
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--help" => { println!("lap-cli face-thumbs --db PATH --cache PATH [--size 256|512|1024] [--aspect 0.8333333333333334] [--workers 1..4] [--pause-ms 25] [--status]\nClose the app first. Completed cache entries are skipped on restart; failures are reported without changing labels."); return Ok(()); }
            "--db" => db = Some(PathBuf::from(args.next().ok_or("missing --db PATH")?)),
            "--cache" => cache = Some(PathBuf::from(args.next().ok_or("missing --cache PATH")?)),
            "--size" => size = args.next().and_then(|v| v.parse().ok()).ok_or("invalid size")?,
            "--aspect" => aspect = args.next().and_then(|v| v.parse().ok()).ok_or("invalid aspect")?,
            "--workers" => workers = args.next().and_then(|v| v.parse().ok()).ok_or("invalid workers")?,
            "--pause-ms" => pause_ms = args.next().and_then(|v| v.parse().ok()).ok_or("invalid pause")?,
            "--status" => status = true,
            _ => return Err(format!("unknown face-thumbs option {flag}")),
        }
    }
    let db = db.ok_or("face-thumbs requires --db PATH --cache PATH; close the app first")?;
    let path = cache.ok_or("--cache PATH is required")?;
    if !db.is_file() || !path.is_file() { return Err("database and cache must already exist".into()); }
    let library_id = db.file_stem().and_then(|s| s.to_str()).ok_or("invalid library filename")?;
    let cache = lap_casa::t_thumb_cache::FaceCache::open(&path)?;
    let started = std::time::Instant::now();
    println!("{}", json!({"event":"face_thumbs_start", "db":db, "cache":path, "workers":workers, "size":size, "aspect":aspect, "status_only":status}));
    let stats = lap_casa::t_face_thumb::pre_generate(&db, library_id, &cache, size, aspect, workers, pause_ms, status,
        |stats| println!("{}", json!({"event":"face_thumbs_progress", "stats":stats, "elapsed_s":started.elapsed().as_secs_f64()})))?;
    println!("{}", json!({"event":"face_thumbs_finish", "stats":stats, "elapsed_s":started.elapsed().as_secs_f64()}));
    if stats.failed > 0 { return Err(format!("{} crops failed and remain resumable", stats.failed)); }
    Ok(())
}

// Diagnostic only: one image, no database connection or result updates. Each
// trace records ONNX kernel provider placement, not just provider registration.
fn profile_image(path: &str, profile_dir: &str) -> Result<(), String> {
    println!("{}", json!({"event": "runtime", "onnxruntime_build": ort::info(), "exe": env::current_exe().map_err(|e| e.to_string())?}));
    let dir = PathBuf::from(profile_dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let models = lap_casa::t_advanced_ai::default_models_dir();
    let mut objects = lap_casa::t_advanced_ai::ObjectEngine::load_profiled(&models, 2, &dir)?;
    let start = std::time::Instant::now();
    let labels = objects.detect(path);
    let elapsed = start.elapsed().as_secs_f64();
    let object_profile = objects.end_profiling()?;
    println!("{}", json!({"event": "objects_profile", "elapsed_s": elapsed, "profile": object_profile, "objects": labels?}));
    drop(objects);
    let mut ocr = lap_casa::t_advanced_ai::OcrEngine::load_profiled(&models, 2, &dir)?;
    let start = std::time::Instant::now();
    let text = ocr.ocr_image(path);
    let elapsed = start.elapsed().as_secs_f64();
    let profiles = ocr.end_profiling()?;
    println!("{}", json!({"event": "ocr_profile", "elapsed_s": elapsed, "profiles": profiles, "text_bytes": text?.len()}));
    Ok(())
}

fn get_app_data_dir() -> Result<PathBuf, String> {
    let identifier = "com.lapcasa.dev";
    let app_dir_name = if cfg!(debug_assertions) {
        format!("{}.debug", identifier)
    } else {
        identifier.to_string()
    };
    dirs::data_local_dir()
        .ok_or_else(|| "Failed to get local AppData directory".to_string())
        .map(|p| p.join(app_dir_name))
}

fn get_current_db_path() -> Result<String, String> {
    let config_path = get_app_data_dir()?.join("app-config.json");
    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read app-config.json: {}", e))?;
    let config: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse app-config.json: {}", e))?;
    let current_library_id = config
        .get("current_library_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "current_library_id not found".to_string())?;

    let db_dir = if let Some(dir) = config.get("db_storage_dir").or_else(|| config.get("dbStorageDir")).and_then(|v| v.as_str()) {
        PathBuf::from(dir)
    } else {
        get_app_data_dir()?.join("libraries")
    };
    Ok(db_dir
        .join(format!("{}.db", current_library_id))
        .to_string_lossy()
        .into_owned())
}

const SEARCH_HELP: &str = "lap-cli search [QUERY] [--db PATH] [--scene SCENE] [--place PLACE] [--object OBJ] [--person NAME_OR_ID] [--tag TAG] [--ocr TEXT] [--folder PATH] [--location LOC] [--after DATE] [--before DATE] [--clip QUERY] [--min-score 0.28] [--limit 25] [--offset 0] [--stats] [--help]
lap-cli stats [--db PATH]
lap-cli people [QUERY] [--db PATH] [--limit 100]
lap-cli search --people-list [QUERY] [--db PATH] [--limit 100]
lap-cli cameras [QUERY] [--by-year] [--db PATH] [--limit 50]
lap-cli search --cameras [QUERY] [--by-year]
Photo filters: --camera TEXT, --make TEXT, --model TEXT, --lens TEXT.
Search output: --view (open Brave), --view-out PATH (HTML only), --show (activate app), --show-only-write (nonactivating handoff).
--people is an alias for the --person photo filter. People lists match names;
photo_count counts distinct eligible photos, face_count counts their face rows.
Filters combine with AND; repeat a filter or separate its values with commas.
Dates accept Unix seconds or YYYY / YYYY-MM / YYYY-MM-DD (UTC, inclusive whole-period bounds).
Four-digit unsigned numbers are years. Substring filters treat % and _ literally.
Plain words match across fields; dates also match dated folders when capture metadata is wrong.
Plain search ranks metadata before visual matches and caps visual results at 200 above cosine 0.28.
Limit is capped at 500; offset is nonnegative. CLIP uses a single CPU thread.
Stats cover eligible, non-excluded photo/video/raw files, with total_files including all files.
Search never writes the library. Errors are JSON with ok:false and a nonzero exit code.";

fn resolve_search_db_path(explicit: Option<&std::path::Path>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return if path.is_file() { Ok(path.to_path_buf()) } else { Err(format!("Database does not exist: {}", path.display())) };
    }
    if let Ok(current) = get_current_db_path() {
        let path = PathBuf::from(current);
        if path.is_file() { return Ok(path); }
    }
    Err("No library database found; pass --db PATH".into())
}

fn open_search_db(path: &std::path::Path) -> Result<Connection, String> {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA mmap_size=2147483648").map_err(|e| e.to_string())?;
    Ok(conn)
}

fn html_escape(value: &str) -> String {
    value.replace('&',"&amp;").replace('<',"&lt;").replace('>',"&gt;").replace('"',"&quot;").replace('\'',"&#39;")
}

fn file_url(path: &std::path::Path) -> Result<String,String> {
    tauri::Url::from_file_path(path).map(|url| url.to_string()).map_err(|_| format!("Invalid absolute file path: {}",path.display()))
}

fn contact_sheet(result: &serde_json::Value) -> Result<String,String> {
    let mut html = String::from("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>LapCasa search</title><style>body{background:#16181c;color:#e8e8e8;font:14px system-ui;margin:24px}main{display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));gap:18px}article{background:#252830;padding:12px;border-radius:8px;overflow-wrap:anywhere}img{width:100%;height:220px;object-fit:contain}span{display:inline-block;background:#353a46;padding:4px 7px;margin:3px;border-radius:5px}p{color:#bfc5d2}</style><h1>LapCasa search</h1><main>");
    for photo in result["results"].as_array().ok_or("Missing photo results")? {
        let url = html_escape(&file_url(std::path::Path::new(photo["path"].as_str().ok_or("Missing photo path")?))?);
        html.push_str(&format!("<article><a href=\"{url}\" target=\"_blank\" rel=\"noopener\"><img loading=\"lazy\" src=\"{url}\" alt=\"{}\"></a><h2>{}</h2><p>{}</p>",html_escape(photo["name"].as_str().unwrap_or("")),html_escape(photo["name"].as_str().unwrap_or("")),html_escape(photo["folder"].as_str().unwrap_or(""))));
        for key in ["taken_iso","clip_score","persons","scenes","places","objects","camera","lens","location","ocr_text"] {
            let values = if let Some(array)=photo[key].as_array() { array.clone() } else { vec![photo[key].clone()] };
            for value in values.into_iter().filter(|v| !v.is_null()) {
                let text = value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string());
                if !text.is_empty() { html.push_str(&format!("<span>{}: {}</span>",key,html_escape(&text))); }
            }
        }
        html.push_str("</article>");
    }
    html.push_str("</main></html>");
    Ok(html)
}

fn launch_search_view(path: &std::path::Path) -> Result<(),String> {
    let url = file_url(path)?;
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        // WMI starts the explicitly requested browser outside an agent's hidden
        // desktop. The helper itself never creates a console window.
        let line = format!("\"C:\\Program Files\\BraveSoftware\\Brave-Browser\\Application\\brave.exe\" --new-window \"{url}\"");
        let script = format!("$result=Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{{CommandLine='{}'}}; if ($result.ReturnValue -ne 0) {{ exit 1 }}",line.replace('\'',"''"));
        let output = std::process::Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-Command",&script]).creation_flags(0x08000000).output().map_err(|e| e.to_string())?;
        if !output.status.success() { return Err(format!("Could not open Brave: {}",String::from_utf8_lossy(&output.stderr))); }
        Ok(())
    }
    #[cfg(not(windows))]
    std::process::Command::new("brave").args(["--new-window",&url]).spawn().map(|_| ()).map_err(|e| e.to_string())
}

fn write_search_handoff(directory: &std::path::Path, options: &SearchOptions, result: &serde_json::Value) -> Result<(),String> {
    let results = result["results"].as_array().ok_or("Missing photo results")?;
    let ids = results.iter().map(|r| r["id"].as_i64().ok_or("Missing photo ID").map(|id| id.to_string())).collect::<Result<Vec<_>,_>>()?.join(",");
    let mut summary = vec![options.query.clone()];
    summary.extend(options.filters.iter().map(|(kind,value)| format!("{kind}:{value}")));
    if let Some(clip)=&options.clip { summary.push(format!("clip:{clip}")); }
    let text = summary.into_iter().filter(|v| !v.is_empty()).collect::<Vec<_>>().join(" ");
    let text = if text.is_empty() { "All photos".to_string() } else { text };
    let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_millis();
    let payload = json!({"id":id as u64,"text":text,"tokens":[{"type":"ids","value":ids}],"count":results.len(),"focus":options.show});
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let temporary = directory.join(format!("external-search.{}.{}.tmp",std::process::id(),id));
    std::fs::write(&temporary,payload.to_string()).map_err(|e| e.to_string())?;
    // The consumer and writer never observe a partially written JSON document.
    // Windows rename cannot replace a destination; remove the previous request
    // first. A consumer can see absence during replacement, never partial JSON.
    let destination = directory.join("external-search.json");
    match std::fs::remove_file(&destination) {
        Ok(()) => {}
        Err(e) if e.kind()==std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    std::fs::rename(&temporary,&destination).map_err(|e| e.to_string())
}

fn run_search(args: &[String], command: &str) -> Result<(),String> {
    let started = std::time::Instant::now();
    let mut parsed_args = args.to_vec();
    if command == "cameras" { parsed_args.insert(0,"--cameras".into()); }
    let mut options = if command == "people" { SearchOptions::parse_for(&parsed_args,true)? } else { SearchOptions::parse(&parsed_args)? };
    options.stats |= command == "stats";
    if [options.stats,options.people_list,options.cameras].into_iter().filter(|v| *v).count()>1 { return Err("Stats, people and cameras are separate modes".into()); }
    if options.by_year && !options.cameras { return Err("--by-year requires cameras".into()); }
    if options.cameras && (!options.filters.is_empty() || options.clip.is_some()) { return Err("Cameras accepts QUERY, --by-year, --db and --limit".into()); }
    if (options.stats || options.people_list || options.cameras) && (options.view || options.view_out.is_some() || options.show || options.show_only_write) { return Err("--view and --show output options require photo search".into()); }
    if options.show && options.show_only_write { return Err("Choose --show or --show-only-write".into()); }
    if options.help { println!("{SEARCH_HELP}"); return Ok(()); }
    let db = resolve_search_db_path(options.db.as_deref())?;
    let conn = open_search_db(&db)?;
    // One consistent read snapshot for counts, matches, and page hydration.
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let mut result = if options.stats { json!({"stats":search_stats(&tx)?}) }
    else if options.people_list { people_results(&tx,&options.query,options.limit,options.offset)? }
    else if options.cameras { camera_results(&tx,&options.query,options.limit,options.by_year)? } else {
        let vector = options.clip.as_deref().map(|query| {
            // Search must work from any shell, including a directory containing
            // only the OCR models used by older CLI commands.
            let models = env::current_exe().map_err(|e| e.to_string())?.with_file_name("models");
            lap_casa::t_advanced_ai::encode_clip_text_query(&models,query)
        }).transpose()?;
        if options.clip.is_none() && !options.query.is_empty() {
            let models = env::current_exe().map_err(|e| e.to_string())?.with_file_name("models");
            plain_search_results(&tx,&options,&models)?
        } else { search_results(&tx,&options,vector.as_deref())? }
    };
    if options.view || options.view_out.is_some() {
        let path = options.view_out.clone().unwrap_or_else(|| env::temp_dir().join("lapcasa-search-view.html"));
        let path = std::path::absolute(path).map_err(|e| e.to_string())?;
        std::fs::write(&path,contact_sheet(&result)?).map_err(|e| e.to_string())?;
        if options.view { launch_search_view(&path)?; }
        result["view_html"] = json!(path);
    }
    if options.show || options.show_only_write {
        // A test-only write can be isolated from a running owner's app. Normal
        // handoffs always use the application's actual data directory.
        let directory = if options.show_only_write { env::var_os("LAP_CLI_HANDOFF_DIR").map(PathBuf::from) } else { None };
        write_search_handoff(&directory.unwrap_or(get_app_data_dir()?),&options,&result)?;
        result["shown_in_app"] = json!(true);
    }
    result["ok"] = json!(true); result["db"] = json!(db); result["elapsed_ms"] = json!(started.elapsed().as_secs_f64()*1000.0);
    println!("{result}");
    Ok(())
}

fn run_ocr(path: &str) {
    match lap_casa::t_advanced_ai::run_ocr_pipeline(path, None) {
        Ok(text) => {
            let res = json!({
                "status": "success",
                "text": text
            });
            println!("{}", res.to_string());
        }
        Err(e) => {
            let res = json!({
                "status": "error",
                "message": e
            });
            println!("{}", res.to_string());
        }
    }
}

// Detect faces again on photos the indexer missed, on an explicit library, with the oriented
// thumbnail-scale path; new faces go into People suggestions only (no clustering, nothing
// assigned, named people untouched). Close the app first.
//   lap-cli face-redetect --db PATH (--ids 1,2,3 | --missed) [--limit N] [--models DIR] [--dry-run]
// --missed: photos scanned without a face (has_faces = 2) whose objects include 'person'.
fn run_face_redetect(args: &[String]) -> Result<(), String> {
    let mut db = None;
    let mut ids: Vec<i64> = Vec::new();
    let mut missed = false;
    let mut limit = usize::MAX;
    let mut dry_run = false;
    let mut models = env::current_exe().map_err(|e| e.to_string())?
        .parent().unwrap().join("models");
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--db" => db = it.next().cloned(),
            "--ids" => ids = it.next().ok_or("missing ids")?.split(',').filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().parse::<i64>().map_err(|_| format!("bad id {s}"))).collect::<Result<_, _>>()?,
            "--missed" => missed = true,
            "--limit" => limit = it.next().and_then(|v| v.parse().ok()).ok_or("invalid limit")?,
            "--models" => models = PathBuf::from(it.next().ok_or("missing models dir")?),
            "--dry-run" => dry_run = true,
            "--help" => { println!("lap-cli face-redetect --db PATH (--ids 1,2,3 | --missed) [--limit N] [--models DIR] [--dry-run]"); return Ok(()); }
            _ => return Err(format!("unknown option {flag}")),
        }
    }
    let db = db.ok_or("--db PATH is required (close the app first)")?;
    if ids.is_empty() == !missed { return Err("give --ids 1,2,3 or --missed, not both".into()); }
    let conn = Connection::open(&db).map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(10)).map_err(|e| e.to_string())?;
    lap_casa::t_face_suggest::migrate(&conn)?;
    if missed {
        ids = conn.prepare(
            "SELECT a.id FROM afiles a
               JOIN afolders f ON a.folder_id = f.id
              WHERE a.file_type = 1 AND a.has_faces = 2 AND a.ai_objects LIKE '%person%'
                AND COALESCE(f.faces_excluded, 0) = 0
                AND a.width IS NOT NULL AND a.width > 0 AND a.height IS NOT NULL AND a.height > 0
                AND NOT EXISTS (SELECT 1 FROM faces f2 WHERE f2.file_id = a.id) ORDER BY a.id")
            .map_err(|e| e.to_string())?
            .query_map([], |r| r.get(0)).map_err(|e| e.to_string())?
            .collect::<Result<_, _>>().map_err(|e| e.to_string())?;
    }
    ids.truncate(limit);
    println!("{}", json!({"event": "face_redetect_start", "db": db, "files": ids.len(), "dry_run": dry_run}));
    if dry_run { return Ok(()); }
    let started = std::time::Instant::now();
    let mut engine = lap_casa::t_face::FaceEngine::new();
    engine.load_models_from_dir(&models)?;
    let stats = lap_casa::t_face::redetect(&conn, &mut engine, &ids, |done, total, file_id, faces| {
        if faces > 0 || done % 100 == 0 || done == total {
            println!("{}", json!({"event": "face_redetect_progress", "done": done, "total": total, "file_id": file_id, "faces": faces, "elapsed_s": started.elapsed().as_secs_f64()}));
        }
    })?;
    println!("{}", json!({"event": "face_redetect_finish", "stats": stats, "elapsed_s": started.elapsed().as_secs_f64()}));
    Ok(())
}

// Picasa-style face suggestions on an explicit library (never the configured one by
// accident): lap-cli face-suggest --db PATH [--threshold X] [--models DIR] [--no-embed]
fn run_face_suggest(args: &[String]) -> Result<(), String> {
    let mut db = None;
    let mut threshold = lap_casa::t_face_suggest::DEFAULT_THRESHOLD;
    let mut models = env::current_exe().map_err(|e| e.to_string())?
        .parent().unwrap().join("models");
    let mut embed = true;
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--db" => db = it.next().cloned(),
            "--threshold" => threshold = it.next().and_then(|v| v.parse().ok()).ok_or("invalid threshold")?,
            "--models" => models = PathBuf::from(it.next().ok_or("missing models dir")?),
            "--no-embed" => embed = false,
            _ => return Err(format!("unknown option {flag}")),
        }
    }
    let db = db.ok_or("--db PATH is required")?;
    let conn = Connection::open(&db).map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(10)).map_err(|e| e.to_string())?;
    lap_casa::t_face_suggest::migrate(&conn)?;
    let started = std::time::Instant::now();
    let mut engine = lap_casa::t_face::FaceEngine::new();
    if embed {
        engine.load_models_from_dir(&models)?;
    }
    let seed = lap_casa::t_face_suggest::seed_named_embeddings(&conn, embed.then_some(&mut engine))?;
    println!("{}", json!({"event": "seed", "stats": seed, "elapsed_s": started.elapsed().as_secs_f64()}));
    let suggest = lap_casa::t_face_suggest::compute_suggestions(&conn, threshold)?;
    println!("{}", json!({"event": "suggest", "stats": suggest, "elapsed_s": started.elapsed().as_secs_f64()}));
    Ok(())
}

#[derive(Debug)]
struct BulkOptions {
    max: usize,
    threads: usize,
    pause_ms: u64,
    // --shard I/N: this process only takes photos whose id % N == I, so N
    // processes can scan side by side without doing the same photo twice.
    shard: (i64, i64),
}

impl BulkOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut result = Self { max: usize::MAX, threads: 2, pause_ms: 100, shard: (0, 1) };
        let mut args = args.iter().peekable();
        if args.peek().is_some_and(|v| !v.starts_with("--")) {
            result.max = args.next().unwrap().parse().map_err(|_| "invalid MAX")?;
        }
        while let Some(flag) = args.next() {
            let value = args.next().ok_or_else(|| format!("missing value for {flag}"))?;
            match flag.as_str() {
                "--threads" => result.threads = value.parse().map_err(|_| "invalid threads")?,
                "--pause-ms" => result.pause_ms = value.parse().map_err(|_| "invalid pause")?,
                "--shard" => {
                    let (i, n) = value.split_once('/').ok_or("shard must be I/N")?;
                    result.shard = (i.parse().map_err(|_| "invalid shard")?, n.parse().map_err(|_| "invalid shard")?);
                    if !(1..=16).contains(&result.shard.1) || !(0..result.shard.1).contains(&result.shard.0) {
                        return Err("shard must be I/N with 0 <= I < N <= 16".into());
                    }
                }
                _ => return Err(format!("unknown option {flag}")),
            }
        }
        if !(1..=8).contains(&result.threads) {
            return Err("threads must be between 1 and 8".into());
        }
        Ok(result)
    }
}

const ELIGIBLE: &str = "FROM afiles a JOIN afolders b ON a.folder_id = b.id
    WHERE COALESCE(b.ai_excluded, 0) = 0 AND a.file_type IN (1, 2, 3)";

fn counts(conn: &Connection) -> Result<serde_json::Value, String> {
    conn.query_row(&format!("SELECT COUNT(*), COALESCE(SUM(a.ocr_text IS NULL),0),
        COALESCE(SUM(a.ai_objects IS NULL),0) {ELIGIBLE}"), [], |row| {
        Ok(json!({"eligible": row.get::<_, i64>(0)?, "ocr_pending": row.get::<_, i64>(1)?,
            "objects_pending": row.get::<_, i64>(2)?}))
    }).map_err(|e| e.to_string())
}

// A keyset cursor skips failures for this pass only. NULL remains the resumable
// queue across invocations; a failed decode/inference is never stored as success.
fn pending(conn: &Connection, after: i64, ocr: bool, objects: bool, limit: usize, shard: (i64, i64))
    -> Result<Vec<(i64, PathBuf, bool, bool)>, String>
{
    let mut stmt = conn.prepare(&format!("SELECT a.id, b.path, a.name,
        a.ocr_text IS NULL, a.ai_objects IS NULL {ELIGIBLE} AND a.id > ?1
        AND ((?2 AND a.ocr_text IS NULL) OR (?3 AND a.ai_objects IS NULL))
        AND a.id % ?6 = ?5 ORDER BY a.id LIMIT ?4")).map_err(|e| e.to_string())?;
    stmt.query_map(rusqlite::params![after, ocr, objects, limit as i64, shard.0, shard.1], |row| {
        Ok((row.get(0)?, PathBuf::from(row.get::<_, String>(1)?).join(row.get::<_, String>(2)?),
            row.get(3)?, row.get(4)?))
    }).map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

fn save_result(conn: &Connection, id: i64, column: &str, value: &str) -> Result<usize, String> {
    // column is selected internally, never from a CLI argument. Recheck both
    // eligibility and NULL so another writer's completed results are preserved.
    conn.execute(&format!("UPDATE afiles SET {column} = ?1 WHERE id = ?2 AND {column} IS NULL
        AND folder_id IN (SELECT id FROM afolders WHERE COALESCE(ai_excluded,0) = 0)"),
        rusqlite::params![value, id]).map_err(|e| e.to_string())
}

fn run_bulk(command: &str, args: &[String]) -> Result<(), String> {
    let options = BulkOptions::parse(args)?;
    let db_path = get_current_db_path()?;
    let flags = if command == "ai-status" { rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY }
        else { rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE };
    let conn = Connection::open_with_flags(&db_path, flags).map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(10)).map_err(|e| e.to_string())?;
    println!("{}", json!({"event": "start", "db": db_path, "counts": counts(&conn)?,
        "workers": 1, "threads": options.threads, "pause_ms": options.pause_ms,
        "shard": format!("{}/{}", options.shard.0, options.shard.1)}));
    if command == "ai-status" || options.max == 0 { return Ok(()); }

    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true)
        .open(if options.shard.1 > 1 { format!("{db_path}.ai-cli.{}of{}.lock", options.shard.0, options.shard.1) } else { format!("{db_path}.ai-cli.lock") }).map_err(|e| e.to_string())?;
    lock.try_lock().map_err(|e| format!("another bulk CLI holds the library lock: {e}"))?;
    let do_ocr = command != "objects-all";
    let do_objects = command != "ocr-all";
    let models = lap_casa::t_advanced_ai::default_models_dir();
    let mut ocr = if do_ocr { Some(lap_casa::t_advanced_ai::OcrEngine::load_with_threads(&models, options.threads)?) } else { None };
    let mut objects = if do_objects { Some(lap_casa::t_advanced_ai::ObjectEngine::load_with_threads(&models, options.threads)?) } else { None };
    let mut after = i64::MIN;
    let (mut attempted, mut saved, mut failed) = (0usize, 0usize, 0usize);
    let started = std::time::Instant::now();
    while attempted < options.max {
        let files = pending(&conn, after, do_ocr, do_objects, (options.max - attempted).min(32), options.shard)?;
        if files.is_empty() { break; }
        for (id, path, needs_ocr, needs_objects) in files {
            after = id;
            let path = path.to_string_lossy();
            for column in ["ai_objects", "ocr_text"] {
                let result = match column {
                    "ai_objects" if needs_objects && do_objects => objects.as_mut().unwrap().detect(&path).map(|v| v.join(", ")),
                    "ocr_text" if needs_ocr && do_ocr => ocr.as_mut().unwrap().ocr_image(&path),
                    _ => continue,
                };
                match result {
                    Ok(value) => {
                        let changed = save_result(&conn, id, column, &value)?;
                        saved += changed;
                        println!("{}", json!({"event": "result", "id": id, "column": column,
                            "saved": changed, "bytes": value.len(), "path": path}));
                    }
                    Err(error) => {
                        failed += 1;
                        println!("{}", json!({"event": "error", "id": id, "column": column, "path": path, "error": error}));
                    }
                }
            }
            attempted += 1;
            std::thread::sleep(std::time::Duration::from_millis(options.pause_ms));
            if attempted % 100 == 0 {
                println!("{}", json!({"event": "progress", "attempted": attempted, "saved": saved,
                    "failed": failed, "elapsed_s": started.elapsed().as_secs_f64(), "last_id": after}));
            }
        }
    }
    println!("{}", json!({"event": "finish", "attempted": attempted, "saved": saved, "failed": failed,
        "elapsed_s": started.elapsed().as_secs_f64(), "counts": counts(&conn)?}));
    if failed > 0 { return Err(format!("{failed} operations failed and remain NULL; see error records")); }
    Ok(())
}

#[cfg(test)]
mod search_tests {
    use super::*;

    fn options(args: &[&str]) -> SearchOptions { SearchOptions::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap() }
    fn fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE afolders(id INTEGER PRIMARY KEY,path TEXT,is_excluded_from_search INTEGER);
            INSERT INTO afolders VALUES(1,'C:/photos/travel',0),(2,'C:/private',1),(3,'C:/photos/video',NULL);
            CREATE TABLE afiles(id INTEGER PRIMARY KEY,folder_id INTEGER,name TEXT,file_type INTEGER,created_at INTEGER,
                taken_date INTEGER,width INTEGER,height INTEGER,ai_tags TEXT,ai_objects TEXT,ocr_text TEXT,comments TEXT,
                geo_name TEXT,geo_admin1 TEXT,geo_cc TEXT,embeds BLOB);
            INSERT INTO afiles VALUES
                (1,1,'beach.jpg',1,1,1592179200,640,480,'scene: beach, place: Prague','person, boat, person','Ticket AB12','Holiday note','Prague','Bohemia','CZ',NULL),
                (2,1,'park.jpg',1,2,1609459200,800,600,'scene: park, place: London','dog, person','Welcome home','','London','England','GB',NULL),
                (3,2,'hidden.jpg',1,3,1700000000,640,480,'scene: beach, place: Prague','person','hidden','private','Prague','Bohemia','CZ',NULL),
                (4,1,'audio.mp3',4,4,1700000000,NULL,NULL,'scene: beach','person',NULL,NULL,NULL,NULL,NULL,NULL),
                (5,1,'raw.dng',3,99,NULL,NULL,NULL,'','',NULL,NULL,NULL,NULL,NULL,NULL),
                (6,3,'movie.mp4',2,6,1609372800,NULL,NULL,NULL,NULL,NULL,'100%_real',NULL,NULL,NULL,x'010203');
            CREATE TABLE persons(id INTEGER PRIMARY KEY,name TEXT);
            INSERT INTO persons VALUES(10,'Alice'),(20,'Bob'),(30,''),(40,NULL);
            CREATE TABLE faces(file_id INTEGER,person_id INTEGER);
            CREATE INDEX idx_faces_person_id ON faces(person_id);
            INSERT INTO faces VALUES(1,10),(1,10),(1,30),(2,10),(2,20);
            CREATE TABLE atags(id INTEGER PRIMARY KEY,name TEXT);
            INSERT INTO atags VALUES(1,'Vacation'),(2,'Receipt');
            CREATE TABLE afile_tags(file_id INTEGER,tag_id INTEGER);
            INSERT INTO afile_tags VALUES(1,1),(2,1),(2,2);").unwrap();
        conn.execute_batch("ALTER TABLE afiles ADD COLUMN e_make TEXT;
            ALTER TABLE afiles ADD COLUMN e_model TEXT;
            ALTER TABLE afiles ADD COLUMN e_lens_make TEXT;
            ALTER TABLE afiles ADD COLUMN e_lens_model TEXT;
            UPDATE afiles SET e_make='Canon',e_model='Canon PowerShot S110',e_lens_make='Canon',e_lens_model='EF 50mm' WHERE id IN (1,3,4);
            UPDATE afiles SET e_make='FUJIFILM',e_model='FinePix S3Pro',e_lens_make='Fujinon',e_lens_model='Zoom 100%_real' WHERE id=2;
            UPDATE afiles SET e_make='Canon',e_model='Canon PowerShot S110',taken_date=NULL WHERE id=5;").unwrap();
        for (id, vector) in [(1,vector(3.0,0.0)),(2,vector(0.0,1.0)),(3,vector(1.0,0.0)),(4,vector(1.0,0.0)),(5,vector(0.8,0.6))] {
            conn.execute("UPDATE afiles SET embeds=?1 WHERE id=?2",rusqlite::params![vector.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>(),id]).unwrap();
        }
        conn
    }
    fn vector(a: f32,b: f32) -> Vec<f32> { let mut v = vec![0.0;512]; v[0]=a; v[1]=b; v }
    fn ids(result: &serde_json::Value) -> Vec<i64> { result["results"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect() }

    #[test]
    fn camera_filters_hydration_and_timeline() {
        let conn=fixture();
        for (args,expected) in [
            (vec!["--camera","CANON"],vec![1,5]),(vec!["--make","fujifilm"],vec![2]),
            (vec!["--model","S110"],vec![1,5]),(vec!["--lens","Canon EF"],vec![1]),
            (vec!["--lens","%_"],vec![2]),(vec!["--make","Canon","--model","FinePix"],vec![]),
            (vec!["FUJIFILM"],vec![2]),(vec!["EF 50mm"],vec![1]),
        ] { assert_eq!(ids(&search_results(&conn,&options(&args),None).unwrap()),expected,"{args:?}"); }
        let photo=hydrate_search_result(&conn,1,None).unwrap();
        assert_eq!(photo["camera"],"Canon PowerShot S110"); assert_eq!(photo["lens"],"Canon EF 50mm");
        assert_eq!(hydrate_search_result(&conn,2,None).unwrap()["camera"],"FUJIFILM FinePix S3Pro");
        assert!(hydrate_search_result(&conn,6,None).unwrap()["camera"].is_null());
        assert_eq!(equipment_label(Some(" canon "),Some(" CANON S110 ")),Some("CANON S110".into()));
        let cameras=camera_results(&conn,"",50,true).unwrap();
        assert_eq!(cameras["total_matches"],2);
        assert_eq!(cameras["cameras"][0]["photo_count"],2);
        assert_eq!(cameras["cameras"][0]["first_year"],"2020");
        assert_eq!(cameras["cameras"][0]["last_year"],"2020");
        assert_eq!(cameras["by_year"][0]["year"],"2021");
        assert_eq!(cameras["by_year"].as_array().unwrap().len(),2);
        assert_eq!(camera_results(&conn,"%_",50,false).unwrap()["total_matches"],1);
        assert_eq!(camera_results(&conn,"Canon EF",50,false).unwrap()["total_matches"],1);
        assert!(camera_results(&conn,"",50,false).unwrap()["by_year"].as_array().unwrap().is_empty());
        assert_eq!(search_stats(&conn).unwrap()["top_cameras"],cameras["cameras"]);
        assert_eq!(search_stats(&conn).unwrap()["top_lenses"],cameras["lenses"]);
        for year in ["1989","2036"] {
            conn.execute("UPDATE afiles SET taken_date=?1 WHERE id=1",[date_bound(year,false).unwrap()]).unwrap();
            assert_eq!(camera_results(&conn,"Canon",50,true).unwrap()["by_year"],json!([]));
        }
        assert_eq!(options(&["--cameras","--by-year"]).limit,50);
        assert_eq!(options(&["--cameras","--limit","1"]).limit,1);
    }

    #[test]
    fn contact_sheet_and_write_only_handoff_are_headless_and_exact() {
        let conn=fixture();
        let options=options(&["--camera","Canon","--view-out","test.html","--show-only-write"]);
        assert!(!options.view && !options.show && options.show_only_write);
        let mut result=search_results(&conn,&options,None).unwrap();
        result["results"][0]["name"]=json!("<script>\"&.jpg");
        result["results"][0]["path"]=json!("C:/photos/a #é&.jpg");
        result["results"][0]["ocr_text"]=json!("<img onerror=alert(1)>");
        let html=contact_sheet(&result).unwrap();
        assert!(html.contains("&lt;script&gt;&quot;&amp;.jpg"));
        assert!(html.contains("&lt;img onerror=alert(1)&gt;"));
        assert!(html.contains("file:///C:/photos/a%20%23%C3%A9&amp;.jpg"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("Canon PowerShot S110"));
        let dir=env::temp_dir().join(format!("lap095-handoff-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        write_search_handoff(&dir,&options,&result).unwrap();
        let path=dir.join("external-search.json");
        let payload:serde_json::Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(payload["tokens"],json!([{"type":"ids","value":"1,5"}]));
        assert_eq!(payload["count"],2); assert_eq!(payload["focus"],false);
        assert_eq!(payload["text"],"camera:Canon");
        write_search_handoff(&dir,&options,&json!({"results":[]})).unwrap();
        let empty:serde_json::Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(empty["tokens"][0]["value"],""); assert_eq!(empty["count"],0);
        std::fs::remove_file(path).unwrap(); std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn plain_words_match_across_fields_and_dates_are_hard_constraints() {
        let conn = fixture();
        for (query, expected) in [
            ("2020 Prague Alice", vec![1]), ("2020-06-15 travel AB12", vec![1]),
            ("Alice Receipt", vec![2]), ("2021 Bohemia", vec![]),
            ("London boat", vec![]), ("2020-06", vec![1]),
            ("2020 beach.jpg", vec![1]), ("2020 hidden", vec![]),
        ] {
            assert_eq!(ids(&search_results(&conn,&options(&[query]),None).unwrap()), expected, "{query}");
        }
        // Metadata still works when the independent visual pass cannot load.
        let result = plain_search_results(&conn,&options(&["2020 Prague"]),std::path::Path::new("missing-models")).unwrap();
        assert_eq!(ids(&result),vec![1]);
        assert!(result["semantic_error"].is_string());
        conn.execute("UPDATE afolders SET path='C:/photos/2006_05_29_Switzerland' WHERE id=1",[]).unwrap();
        // Imported EXIF can contain an ingest date decades after the folder date.
        conn.execute("UPDATE afiles SET taken_date=?1 WHERE id=1",[date_bound("2025-09-26",false).unwrap()]).unwrap();
        for query in ["2006 Switzerland", "2006-05-29 Switzerland"] {
            assert!(ids(&search_results(&conn,&options(&[query]),None).unwrap()).contains(&1));
        }
        conn.execute("UPDATE afiles SET name='DSC_0681.jpg' WHERE id=1",[]).unwrap();
        conn.execute("UPDATE afiles SET name='IMG_3829.jpg' WHERE id=2",[]).unwrap();
        for (query,expected) in [("0681",vec![1]),("DSC 0681",vec![1]),("IMG 3829",vec![2])] {
            assert_eq!(ids(&search_results(&conn,&options(&[query]),None).unwrap()),expected);
        }
    }

    #[test]
    fn sql_filters_cover_every_field_and_combine_with_and() {
        let conn = fixture();
        for (args, expected) in [
            (vec!["beach.jpg"],vec![1]),(vec!["AB12"],vec![1]),(vec!["Holiday"],vec![1]),
            (vec!["Bohemia"],vec![1]),(vec!["CZ"],vec![1]),(vec!["travel"],vec![2,1,5]),
            (vec!["Alice"],vec![2,1]),(vec!["Receipt"],vec![2]),
            (vec!["--scene","BEACH"],vec![1]),(vec!["--place","Prague"],vec![1]),
            (vec!["--place","England"],vec![2]),(vec!["--location","GB"],vec![2]),
            (vec!["--object","person"],vec![2,1]),(vec!["--ocr","ticket"],vec![1]),
            (vec!["--person","10"],vec![2,1]),(vec!["--person","Bob"],vec![2]),
            (vec!["--tag","Vacation"],vec![2,1]),(vec!["--folder","video"],vec![6]),
            (vec!["--object","person,boat"],vec![1]),(vec!["--person","Alice","--person","Bob"],vec![2]),
            (vec!["--scene","beach","--place","Prague","--person","Alice","--ocr","AB12"],vec![1]),
            (vec!["--scene","park","--tag","Receipt","--object","boat"],vec![]),
            (vec!["100%_real"],vec![2,6]),(vec!["%"],vec![2,6]),(vec!["' OR 1=1 --"],vec![]),
        ] {
            let result = search_results(&conn,&options(&args),None).unwrap();
            assert_eq!(ids(&result),expected,"{args:?}");
            assert_eq!(result["total_matches"].as_u64().unwrap() as usize,expected.len());
        }
        let result = search_results(&conn,&options(&["--limit","1","--offset","1"]),None).unwrap();
        assert_eq!(ids(&result),vec![6]); assert_eq!(result["total_matches"],4);
        assert!(ids(&search_results(&conn,&options(&["--offset","999"]),None).unwrap()).is_empty());
        assert!(ids(&search_results(&conn,&options(&["--limit","0"]),None).unwrap()).is_empty());
    }

    #[test]
    fn dates_use_inclusive_utc_periods_and_numeric_timestamps() {
        let conn = fixture();
        for (args, expected) in [
            (vec!["--after","2020","--before","2020"],vec![6,1]),
            (vec!["--after","2020-06","--before","2020-06"],vec![1]),
            (vec!["--after","2020-06-15","--before","2020-06-15"],vec![1]),
            (vec!["--after","1592179200","--before","1592179200"],vec![1]),
            (vec!["--after","2021","--before","2020"],vec![]),
        ] { assert_eq!(ids(&search_results(&conn,&options(&args),None).unwrap()),expected,"{args:?}"); }
        assert_eq!(date_bound("2020-02",true).unwrap()-date_bound("2020-02",false).unwrap(),29*86400-1);
        assert_eq!(date_bound("-1",false).unwrap(),-1);
        for invalid in ["2021-02-29","2020-13","not-a-date","2020-02-31","2020-00-01"] { assert!(date_bound(invalid,false).is_err()); }
    }

    #[test]
    fn stats_parse_per_photo_labels_and_exclude_private_files() {
        let stats = search_stats(&fixture()).unwrap();
        assert_eq!(stats["total_files"],6); assert_eq!(stats["eligible_files"],4);
        assert_eq!(stats["with_embeds"],4); assert_eq!(stats["with_ai_tags"],2);
        assert_eq!(stats["with_ai_objects"],2); assert_eq!(stats["with_ocr_text"],2);
        assert_eq!(stats["named_persons"],2);
        assert_eq!(stats["top_scenes"],json!([{"name":"beach","count":1},{"name":"park","count":1}]));
        assert_eq!(stats["top_places"][0],json!({"name":"London","count":1}));
        assert_eq!(stats["top_objects"][0],json!({"name":"person","count":2}));
    }

    #[test]
    fn people_alias_listing_and_stats_count_eligible_photos_once() {
        let conn = fixture();
        conn.execute_batch("INSERT INTO persons VALUES(50,'Anna'),(60,'Zelda'),(70,'Clara');
            INSERT INTO faces VALUES(3,50),(4,60);").unwrap();
        let alias = options(&["--people","ALICE"]);
        assert_eq!(alias.filters,vec![("person".into(),"ALICE".into())]);
        assert_eq!(ids(&search_results(&conn,&alias,None).unwrap()),vec![2,1]);
        let listing = people_results(&conn,"",100,0).unwrap();
        assert_eq!(listing["total_matches"],5);
        assert_eq!(listing["people"],json!([
            {"id":10,"name":"Alice","photo_count":2,"face_count":3},
            {"id":20,"name":"Bob","photo_count":1,"face_count":1},
            {"id":50,"name":"Anna","photo_count":0,"face_count":0},
            {"id":70,"name":"Clara","photo_count":0,"face_count":0},
            {"id":60,"name":"Zelda","photo_count":0,"face_count":0}]));
        let filtered = people_results(&conn,"aLi",1,0).unwrap();
        assert_eq!(filtered["total_matches"],1); assert_eq!(filtered["people"][0]["name"],"Alice");
        assert_eq!(people_results(&conn,"",1,1).unwrap()["people"][0]["name"],"Bob");
        assert_eq!(people_results(&conn,"%",100,0).unwrap()["total_matches"],0);
        assert_eq!(options(&["--people-list","Alice"]).limit,100);
        assert_eq!(SearchOptions::parse_for(&[],true).unwrap().limit,100);
        for args in [vec!["--limit","2","--people-list"],vec!["--people-list","--limit","2"]] {
            assert_eq!(options(&args).limit,2);
        }
        for args in [vec!["--people-list","--stats"],vec!["--people-list","--person","Alice"]] {
            assert!(SearchOptions::parse(&args.into_iter().map(str::to_string).collect::<Vec<_>>()).is_err());
        }
        for id in 100..120 { conn.execute("INSERT INTO persons VALUES(?1,?2)",rusqlite::params![id,format!("Extra{id}")]).unwrap(); }
        let stats = search_stats(&conn).unwrap();
        assert_eq!(stats["top_persons"].as_array().unwrap().len(),15);
        assert_eq!(stats["top_persons"][0],json!({"id":10,"name":"Alice","photo_count":2}));
        assert!(stats["top_persons"][0].get("face_count").is_none());
    }

    #[test]
    fn result_hydration_is_structured_and_prefers_user_comments() {
        let conn = fixture();
        let result = hydrate_search_result(&conn,1,None).unwrap();
        assert_eq!(result["persons"],json!(["Alice"])); assert_eq!(result["tags"],json!(["Vacation"]));
        assert_eq!(result["scenes"],json!(["beach"])); assert_eq!(result["places"],json!(["Prague"]));
        assert_eq!(result["objects"],json!(["person","boat"]));
        assert_eq!(result["taken_iso"],"2020-06-15T00:00:00Z");
        assert_eq!(result["description"],"Holiday note"); assert_eq!(result["location"],"Prague, Bohemia, CZ");
        assert_eq!(result["width"],640); assert_eq!(result["height"],480);
        assert!(result["path"].as_str().unwrap().ends_with("beach.jpg")); assert!(result["clip_score"].is_null());
        assert!(hydrate_search_result(&conn,2,None).unwrap()["description"].as_str().unwrap().contains("park scene in London"));
        let empty = hydrate_search_result(&conn,5,None).unwrap();
        for key in ["taken_date","taken_iso","width","height","description","location"] { assert!(empty[key].is_null(),"{key}"); }
    }

    #[test]
    fn clip_ranks_normalized_embeddings_after_sql_filters_then_paginates() {
        let conn = fixture();
        let query = vector(1.0,0.0);
        let result = search_results(&conn,&options(&["--clip","beach","--limit","1","--offset","1"]),Some(&query)).unwrap();
        assert_eq!(result["total_matches"],2); assert_eq!(result["skipped_embeddings"],1);
        assert_eq!(ids(&result),vec![5]); assert_eq!(result["results"][0]["clip_score"],0.8);
        let filtered = search_results(&conn,&options(&["--clip","beach","--person","Alice","--min-score","0"]),Some(&query)).unwrap();
        assert_eq!(ids(&filtered),vec![1,2]); assert_eq!(filtered["skipped_embeddings"],0);
        let bytes = query.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>();
        conn.execute("UPDATE afiles SET embeds=?1 WHERE id=6",[bytes]).unwrap();
        assert_eq!(ids(&search_results(&conn,&options(&["--clip","beach"]),Some(&query)).unwrap()),vec![6,1,5]);
        assert!(search_results(&conn,&options(&["--clip","beach"]),Some(&[f32::NAN;512])).is_err());
    }

    #[test]
    fn parser_validates_flags_caps_limits_and_reports_missing_database() {
        let parsed = options(&["family","holiday","--limit=999","--object","person,boat","--object","car"]);
        assert_eq!(parsed.query,"family holiday"); assert_eq!(parsed.limit,500); assert_eq!(parsed.filters.len(),3);
        assert!(options(&["--stats"]).stats); assert!(options(&["--help"]).help);
        for args in [vec!["--nope"],vec!["--db"],vec!["--limit","-1"],vec!["--offset","-1"],vec!["--min-score","NaN"],vec!["--min-score","2"],vec!["--scene","beach,,park"],vec!["--after","2021-02-29"],vec!["--clip",""]] {
            assert!(SearchOptions::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).is_err(),"{args:?}");
        }
        let path = std::env::temp_dir().join(format!("lap094-{}.db",std::process::id()));
        assert!(resolve_search_db_path(Some(&path)).is_err()); assert!(open_search_db(&path).is_err()); assert!(!path.exists());
        {
            let db = Connection::open(&path).unwrap(); db.execute("CREATE TABLE unchanged(id INTEGER)",[]).unwrap();
        }
        let db = open_search_db(&path).unwrap();
        assert!(db.execute("INSERT INTO unchanged VALUES(1)",[]).is_err());
        assert_eq!(resolve_search_db_path(Some(&path)).unwrap(),path);
        drop(db); std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_and_saves_preserve_completed_and_excluded_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE afolders(id INTEGER PRIMARY KEY, path TEXT, ai_excluded INTEGER);
            CREATE TABLE afiles(id INTEGER PRIMARY KEY, folder_id INTEGER, name TEXT, file_type INTEGER, ocr_text TEXT, ai_objects TEXT);
            INSERT INTO afolders VALUES (1,'C:/photos',0),(2,'C:/private',1);
            INSERT INTO afiles VALUES (1,1,'bad.jpg',1,NULL,NULL),(2,1,'done.jpg',1,'text','chair'),
                (3,2,'excluded.jpg',1,NULL,NULL),(4,1,'movie.mp4',4,NULL,NULL),(5,1,'next.jpg',1,NULL,'person');").unwrap();
        let rows = pending(&conn, i64::MIN, true, true, 32, (0, 1)).unwrap();
        assert_eq!(rows.iter().map(|r| r.0).collect::<Vec<_>>(), vec![1,5]);
        // A failed item is skipped by the cursor but remains pending on restart.
        assert_eq!(pending(&conn, 1, true, true, 32, (0, 1)).unwrap()[0].0, 5);
        assert_eq!(pending(&conn, i64::MIN, true, true, 32, (0, 1)).unwrap()[0].0, 1);
        assert_eq!(save_result(&conn, 2, "ocr_text", "replacement").unwrap(), 0);
        assert_eq!(save_result(&conn, 3, "ocr_text", "excluded").unwrap(), 0);
        assert_eq!(save_result(&conn, 5, "ocr_text", "").unwrap(), 1);
        assert_eq!(pending(&conn, 1, true, true, 32, (0, 1)).unwrap().len(), 0);
        assert_eq!(counts(&conn).unwrap()["ocr_pending"], 1);
    }

    #[test]
    fn rejects_unbounded_threads_and_invalid_max() {
        for args in [vec!["oops"], vec!["--threads", "0"], vec!["--threads", "99"], vec!["--pause-ms"]] {
            assert!(BulkOptions::parse(&args.into_iter().map(String::from).collect::<Vec<_>>()).is_err());
        }
    }
}
