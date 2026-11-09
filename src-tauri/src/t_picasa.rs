//! Settings bridge to the verified Picasa importer. No automatic import.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager};

static RUNNING: AtomicBool = AtomicBool::new(false);

static SIDECAR_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Edit one photo section while retaining unrelated lines, sections and keys.
pub fn update_picasa_ini_entry(folder: &Path, filename: &str, updates: &[(&str, Option<&str>)]) -> Result<(), String> {
    use std::io::Write;
    if filename.is_empty() || filename.contains(['\r', '\n', '[', ']']) {
        return Err("Invalid Picasa section name".into());
    }
    for (key, value) in updates {
        if key.trim().is_empty() || key.contains(['\r', '\n', '=', '[', ']', ';', '#'])
            || value.is_some_and(|v| v.contains(['\r', '\n'])) {
            return Err("Invalid Picasa key or value".into());
        }
    }
    let _guard = SIDECAR_WRITE_LOCK.lock().map_err(|e| e.to_string())?;
    let path = folder.join(".picasa.ini");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_owned).collect();
    let section = format!("[{filename}]");
    for (key, value) in updates {
        let mut inside = false;
        let mut section_end = None;
        let mut found = false;
        lines.retain_mut(|line| {
            let trimmed = line.trim().trim_start_matches('\u{feff}');
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                inside = trimmed == section;
            }
            if inside && !trimmed.starts_with([';', '#'])
                && trimmed.split_once('=').is_some_and(|(k,_)| k.trim().eq_ignore_ascii_case(key.trim())) {
                if found || value.is_none() { return false; }
                *line = format!("{}={}{}", key.trim(), value.unwrap(), newline);
                found = true;
            }
            true
        });
        if found || value.is_none() { continue; }
        inside = false;
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim().trim_start_matches('\u{feff}');
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                if inside { section_end = Some(index); break; }
                inside = trimmed == section;
            }
        }
        let index = section_end.unwrap_or(lines.len());
        if !inside && section_end.is_none() {
            if let Some(last) = lines.last_mut() {
                if !last.ends_with('\n') { last.push_str(newline); }
            }
            lines.push(format!("{section}{newline}"));
        } else if index > 0 && !lines[index-1].ends_with('\n') {
            lines[index-1].push_str(newline);
        }
        let index = if !inside && section_end.is_none() { lines.len() } else { index };
        lines.insert(index, format!("{}={}{}", key.trim(), value.unwrap(), newline));
    }
    let staged = folder.join(format!(".picasa.ini.{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&staged)
            .map_err(|e| e.to_string())?;
        file.write_all(lines.concat().as_bytes()).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        // rename replaces the existing file atomically on Windows and Unix.
        fs::rename(&staged, &path).map_err(|e| e.to_string())
    })();
    let _ = fs::remove_file(staged);
    result
}
struct Busy;
impl Busy {
    fn acquire() -> Result<Self, String> {
        RUNNING.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "A Picasa import operation is already running".to_string())?;
        Ok(Self)
    }
}
impl Drop for Busy { fn drop(&mut self) { RUNNING.store(false, Ordering::Release); } }

#[derive(Clone, Serialize, Deserialize)]
pub struct RootMapping { pub source: String, pub target: String }
#[derive(Serialize, Deserialize)]
struct Request { database: String, contacts: String, mappings: Vec<RootMapping>, coordinates: String }

fn package_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let packaged = app.path().resource_dir().map_err(|e| e.to_string())?.join("picasa-import");
    if packaged.join("picasa_import/desktop.py").is_file() { return Ok(packaged); }
    #[cfg(debug_assertions)]
    {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../importers/picasa");
        if source.join("picasa_import/desktop.py").is_file() { return Ok(source); }
    }
    Err("Picasa importer files are missing from this application build".into())
}

fn python() -> Result<(String, Vec<String>), String> {
    let mut choices: Vec<(String, Vec<String>)> = Vec::new();
    if let Ok(path) = std::env::var("LAP_PICASA_PYTHON") { choices.push((path, vec![])); }
    #[cfg(windows)]
    {
        choices.push(("py".into(), vec!["-3".into()]));
    }
    choices.push(("python3".into(), vec![]));
    choices.push(("python".into(), vec![]));
    for (exe, args) in choices {
        let mut command = Command::new(&exe);
        command.args(&args).args(["-c", "import sys; from PIL import Image; assert sys.version_info >= (3,11)"]);
        #[cfg(windows)]
        { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
        if command.output().is_ok_and(|out| out.status.success()) { return Ok((exe, args)); }
    }
    Err("Picasa importing is unavailable: Python 3.11 or later with Pillow is required on this computer".into())
}

fn jobs_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("picasa-import"))
}
fn job_path(app: &tauri::AppHandle, token: &str) -> Result<PathBuf, String> {
    uuid::Uuid::parse_str(token).map_err(|_| "Invalid Picasa preview ID".to_string())?;
    let root = jobs_root(app)?.join(token);
    if !root.join("request.json").is_file() { return Err("Picasa preview was not found; preview again".into()); }
    Ok(root)
}

fn run(app: &tauri::AppHandle, mode: &str, job: &Path, token: &str) -> Result<Value, String> {
    let package = package_root(app)?;
    let (exe, args) = python()?;
    let mut command = Command::new(exe);
    command.args(args).args(["-s", "-c", "import runpy,sys; sys.path.insert(0,sys.argv.pop(1)); runpy.run_module('picasa_import.desktop',run_name='__main__')"])
        .arg(package).arg(mode).arg(job)
        .env("PYTHONUTF8", "1").env("PYTHONUNBUFFERED", "1")
        .stdout(Stdio::piped()).stderr(Stdio::piped());
    if mode == "apply" { command.arg("--desktop-user-confirmed"); }
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
    let mut child = command.spawn().map_err(|e| format!("Cannot start Picasa importer: {e}"))?;
    let mut stderr = child.stderr.take().ok_or("Cannot read importer errors")?;
    let errors = std::thread::spawn(move || { let mut text = String::new(); let _ = stderr.read_to_string(&mut text); text });
    let stdout = child.stdout.take().ok_or("Cannot read importer output")?;
    let mut result = None;
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if let Ok(value) = serde_json::from_str::<Value>(&line) {
            if value["event"] == "result" { result = Some(value); }
            else { let _ = app.emit("picasa-import-progress", json!({"token": token, "progress": value})); }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let error = errors.join().unwrap_or_default();
    if !status.success() {
        if let Ok(value) = serde_json::from_str::<Value>(error.trim()) {
            if let Some(message) = value["error"].as_str() { return Err(message.to_string()); }
        }
        return Err(if error.trim().is_empty() { "Picasa importer stopped unexpectedly".into() } else { error.trim().into() });
    }
    result.ok_or_else(|| "Picasa importer returned no result".into())
}

#[tauri::command]
pub async fn picasa_import_defaults(app: tauri::AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        package_root(&app)?;
        let runtime = python().err();
        let database = crate::t_storage::get_current_db_path()?;
        let conn = rusqlite::Connection::open_with_flags(&database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT path FROM albums ORDER BY id").map_err(|e| e.to_string())?;
        let roots = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
        let contacts = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
            .map(|p| p.join("Google/Picasa2/contacts/contacts.xml"))
            .filter(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(json!({"contacts": contacts, "targetRoots": roots, "unavailable": runtime}))
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn picasa_import_preview(app: tauri::AppHandle, contacts: String, mappings: Vec<RootMapping>, token: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = Busy::acquire()?;
        uuid::Uuid::parse_str(&token).map_err(|_| "Invalid Picasa preview ID".to_string())?;
        if mappings.is_empty() { return Err("Choose a Picasa photo folder and its matching library folder".into()); }
        let database = crate::t_storage::get_current_db_path()?;
        let request = Request { database: database.clone(), contacts, mappings, coordinates: "raw".into() };
        let job = jobs_root(&app)?.join(&token);
        fs::create_dir_all(job.parent().ok_or("Invalid import directory")?).map_err(|e| e.to_string())?;
        fs::create_dir(&job).map_err(|e| e.to_string())?;
        fs::write(job.join("request.json"), serde_json::to_vec(&request).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let mut result = run(&app, "preview", &job, &token)?;
        if crate::t_storage::get_current_db_path()? != database { return Err("Library changed during preview; preview the selected library again".into()); }
        result["token"] = json!(token);
        Ok(result)
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn picasa_import_apply(app: tauri::AppHandle, token: String, thumbnail_size: Option<u32>, tile_aspect: Option<f64>) -> Result<Value, String> {
    let size = thumbnail_size.unwrap_or(512);
    let aspect = tile_aspect.unwrap_or(5.0 / 6.0);
    if ![256,512,1024].contains(&size) || !(0.5..=2.0).contains(&aspect) { return Err("Invalid face thumbnail profile".into()); }
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = Busy::acquire()?;
        let _library = crate::t_cmds::FILE_REFRESH_LIBRARY_LOCK.lock().map_err(|e| e.to_string())?;
        if crate::t_advanced_ai::background_enabled() { return Err("Turn off Process library in background before importing people".into()); }
        let job = job_path(&app, &token)?;
        let request: Request = serde_json::from_slice(&fs::read(job.join("request.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        if request.database != crate::t_storage::get_current_db_path()? { return Err("The selected library differs from the preview; preview again".into()); }
        let mut result = run(&app, "apply", &job, &token)?;
        // Finish the persistent named-face cache before exposing imported people.
        // The library lock prevents a switch while these crops are generated.
        let prepared = (|| {
            let cache = crate::t_thumb_cache::FaceCache::current()?;
            let library_id = Path::new(&request.database).file_stem().and_then(|s| s.to_str()).ok_or("invalid library filename")?;
            crate::t_face_thumb::pre_generate(Path::new(&request.database), library_id, &cache,
                size, aspect, 2, 25, false, |stats| {
                    let _ = app.emit("picasa-import-progress", json!({"token":token,
                        "progress":{"event":"face-thumbnails", "stats":stats}}));
                })
        })();
        // Import has committed already. A cache failure must not report that the
        // labels failed to import, and completed crops remain resumable.
        match prepared {
            Ok(stats) => result["faceThumbnails"] = serde_json::to_value(stats).map_err(|e| e.to_string())?,
            Err(error) => result["faceThumbnailError"] = json!(error),
        }
        let _ = app.emit("picasa-import-finished", json!({"token": token}));
        let _ = app.emit("refresh-content", ());
        Ok(result)
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn picasa_import_open_report(app: tauri::AppHandle, token: String) -> Result<(), String> {
    opener::open(job_path(&app, &token)?).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sidecar_round_trip_preserves_sections_and_faces() {
        let root = std::env::temp_dir().join(format!("lap089-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let path = root.join(".picasa.ini");
        let prefix = "[Picasa]\r\nname=Holiday\r\n[.album:abc]\r\nname=Family\r\n";
        let suffix = "[other.jpg]\r\nstar=yes\r\n";
        fs::write(&path,format!("{prefix}[photo.jpg]\r\nfaces=rect64(1234),abc\r\nstar=no\r\ncaption=old\r\nbackuphash=123\r\n{suffix}")).unwrap();
        update_picasa_ini_entry(&root,"photo.jpg",&[("star",Some("yes")),("caption",Some("Holiday = fun")),("filters",Some("finetune2=1,0,0,0,0,0"))]).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(prefix));
        assert!(text.ends_with(suffix));
        for line in ["faces=rect64(1234),abc", "star=yes", "caption=Holiday = fun", "filters=finetune2=1,0,0,0,0,0", "backuphash=123"] {
            assert!(text.contains(&format!("{line}\r\n")),"{line}");
        }
        update_picasa_ini_entry(&root,"photo.jpg",&[("filters",None),("backuphash",None)]).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("filters="));
        assert!(!text.contains("backuphash="));
        assert!(text.contains("faces=rect64(1234),abc"));
        update_picasa_ini_entry(&root,"new.jpg",&[("star",Some("yes"))]).unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("[new.jpg]\r\nstar=yes\r\n"));
        fs::remove_dir_all(root).unwrap();
    }
}
