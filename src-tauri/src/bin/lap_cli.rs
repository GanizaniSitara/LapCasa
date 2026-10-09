use rusqlite::Connection;
use serde_json::json;
use std::env;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: lap-cli <search QUERY | ocr PATH | objects PATH | ai-profile PATH PROFILE_DIR | ai-status | ai-all | ocr-all | objects-all> [MAX] [--threads N] [--pause-ms N]\n       lap-cli face-thumbs --db PATH --cache PATH [--size 512] [--aspect 0.8333333333333334] [--workers 2] [--pause-ms 25] [--status]");
        std::process::exit(1);
    }

    let command = &args[1];

    match command.as_str() {
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
        "search" => {
            if args.len() < 3 {
                eprintln!("Usage: lap-cli search <query>");
                std::process::exit(1);
            }
            let query = &args[2];
            search_db(query);
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

// Explicit paths prevent a maintenance command from touching the wrong library.
fn run_photo_thumbs(args: &[String]) -> Result<(), String> {
    use std::io::Write;
    let (mut db,mut path,mut journal)=(None,None,None);
    let (mut workers,mut pause_ms)=(2usize,25u64);
    let mut args=args.iter();
    while let Some(flag)=args.next() {
        match flag.as_str() {
            "--help"=>{println!("lap-cli photo-thumbs --db PATH --cache PATH --journal PATH [--workers 1..4] [--pause-ms 25]\nClose the app first. Upgrade orientation versions, retain unaffected bytes, regenerate PRIMARY mismatches. Completed entries are skipped on restart. Journal only changed rows/entries (64MB cap); Windows requires 20GB free before each batch.");return Ok(());}
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

fn search_db(query: &str) {
    let db_path = match get_current_db_path() {
        Ok(p) => p,
        Err(e) => {
            let res = json!({ "error": e });
            println!("{}", res.to_string());
            return;
        }
    };

    let conn = match Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            let res = json!({ "error": format!("Failed to open DB: {}", e) });
            println!("{}", res.to_string());
            return;
        }
    };

    let sql = "
        SELECT id, name, folder_id, ocr_text 
        FROM afiles 
        WHERE name LIKE ?1 OR ocr_text LIKE ?1 
        LIMIT 50
    ";

    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(e) => {
            let res = json!({ "error": format!("Failed to prepare statement: {}", e) });
            println!("{}", res.to_string());
            return;
        }
    };

    let search_term = format!("%{}%", query);

    let rows = match stmt.query_map([&search_term], |row| {
        Ok(json!({
            "id": row.get::<_, i64>(0)?,
            "name": row.get::<_, String>(1)?,
            "folder_id": row.get::<_, Option<i64>>(2)?,
            "ocr_text": row.get::<_, Option<String>>(3)?,
        }))
    }) {
        Ok(r) => r,
        Err(e) => {
            let res = json!({ "error": format!("Failed to execute query: {}", e) });
            println!("{}", res.to_string());
            return;
        }
    };

    let mut results = Vec::new();
    for row in rows {
        if let Ok(r) = row {
            results.push(r);
        }
    }

    println!("{}", serde_json::to_string(&results).unwrap());
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

// Face suggestions on an explicit library (never the configured one by
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
