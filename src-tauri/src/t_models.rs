//! Optional, explicitly consented model packs. No network or installation at startup.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::{Path, PathBuf}, sync::{Arc, Mutex, OnceLock, atomic::{AtomicBool, Ordering}}};
use tauri::{Emitter, Manager};
use tokio::io::AsyncWriteExt;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ModelFile {
    name: String,
    url: String,
    bytes: u64,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Pack {
    id: String,
    label: String,
    purpose: String,
    provider: String,
    licence: String,
    licence_url: String,
    licence_text: String,
    restriction: String,
    unavailable: String,
    files: Vec<ModelFile>,
}

impl Pack {
    // Bind acceptance to both the displayed terms and the precise download set.
    fn revision(&self) -> String {
        format!("{:x}", Sha256::digest(serde_json::to_vec(self).expect("model catalogue serialises")))
    }

    fn consent(&self, accepted: bool, revision: &str) -> Result<(), String> {
        if !accepted || revision != self.revision() {
            return Err("Review and accept the current model terms before downloading".into());
        }
        if !self.unavailable.is_empty() { return Err(self.unavailable.clone()); }
        if self.files.is_empty() || self.files.iter().any(|f| {
            !f.url.starts_with("https://huggingface.co/") || f.name.contains(['/', '\\'])
                || f.name.starts_with('.') || f.bytes == 0 || f.sha256.len() != 64
        }) { return Err("Model download source is not verified".into()); }
        Ok(())
    }
}

fn catalogue() -> &'static [Pack] {
    static PACKS: OnceLock<Vec<Pack>> = OnceLock::new();
    PACKS.get_or_init(|| serde_json::from_str(include_str!("model_catalog.json")).expect("fixed model catalogue"))
}

fn pack(id: &str) -> Result<&'static Pack, String> {
    catalogue().iter().find(|p| p.id == id).ok_or_else(|| "Unknown model pack".into())
}

fn root() -> Result<PathBuf, String> {
    Ok(crate::t_config::get_app_data_dir()?.join("models/packs"))
}

fn installed_dir(root: &Path, pack: &Pack) -> PathBuf {
    root.join(&pack.id).join(pack.revision())
}

#[derive(Deserialize, Serialize)]
struct Receipt {
    revision: String,
    accepted: bool,
    accepted_at: String,
    provider: String,
    licence_url: String,
}

fn files_present(pack: &Pack, dir: &Path) -> bool {
    pack.files.iter().all(|f| std::fs::metadata(dir.join(&f.name)).is_ok_and(|m| m.is_file() && m.len() == f.bytes))
}

fn bundled_present(pack: &Pack, dir: &Path) -> bool {
    // Retain the exact orientation-classifier export shipped by existing builds.
    // New downloads use the pinned provider export instead.
    pack.files.iter().all(|f| {
        let path = dir.join(&f.name);
        let Ok(meta) = std::fs::metadata(&path) else { return false };
        if !meta.is_file() { return false; }
        if meta.len() == f.bytes { return true; }
        if pack.id == "ocr" && f.name == "ocr_cls.onnx" && meta.len() == 571423 {
            return std::fs::read(path).is_ok_and(|bytes| format!("{:x}", Sha256::digest(bytes)) == "e263cad54b85e6dd6ce5aed66154785682a221be2216300ce321e58a9d022296");
        }
        false
    })
}

fn installed(pack: &Pack, dir: &Path) -> bool {
    let Ok(bytes) = std::fs::read(dir.join("licence-receipt.json")) else { return false };
    let Ok(receipt) = serde_json::from_slice::<Receipt>(&bytes) else { return false };
    receipt.accepted && receipt.revision == pack.revision() && files_present(pack, dir)
}

/// Preserve existing bundled/dev models. A model-free public bundle uses only
/// an installed, consented pack. Never search another user's profile or fetch here.
pub fn resolve(id: &str, bundled: &Path) -> PathBuf {
    let Ok(pack) = pack(id) else { return bundled.to_owned() };
    if bundled_present(pack, bundled) { return bundled.to_owned(); }
    if let Ok(root) = root() {
        let dir = installed_dir(&root, pack);
        if installed(pack, &dir) { return dir; }
    }
    bundled.to_owned()
}

pub fn app_dir(app: &tauri::AppHandle, id: &str) -> PathBuf {
    let bundled = app.path().resource_dir().unwrap_or_else(|_| PathBuf::from(".")).join("models");
    resolve(id, &bundled)
}

pub fn available(id: &str, bundled: &Path) -> bool {
    pack(id).is_ok_and(|p| bundled_present(p, &resolve(id, bundled)))
}

pub fn missing(id: &str) -> String {
    let label = pack(id).map(|p| p.label.as_str()).unwrap_or("AI");
    format!("{label} models are not installed. Open Settings > Search > Model downloads.")
}

#[derive(Serialize)]
pub struct PackStatus {
    #[serde(flatten)]
    pack: Pack,
    revision: String,
    total_bytes: u64,
    installed: bool,
    bundled: bool,
    downloading: bool,
}

struct Active {
    id: String,
    request_id: String,
    cancelled: Arc<AtomicBool>,
    committed: bool,
}
static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);

struct DownloadGuard;
impl Drop for DownloadGuard {
    fn drop(&mut self) { *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = None; }
}

#[tauri::command]
pub async fn model_packs(app: tauri::AppHandle) -> Result<Vec<PackStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = root()?;
        let bundled_dir = app.path().resource_dir().map_err(|e| e.to_string())?.join("models");
        let active = ACTIVE.lock().map_err(|e| e.to_string())?;
        Ok(catalogue().iter().map(|p| {
            let bundled = bundled_present(p, &bundled_dir);
            PackStatus {
                pack: p.clone(), revision: p.revision(), total_bytes: p.files.iter().map(|f| f.bytes).sum(),
                installed: bundled || installed(p, &installed_dir(&root, p)), bundled,
                downloading: active.as_ref().is_some_and(|a| a.id == p.id),
            }
        }).collect())
    }).await.map_err(|e| e.to_string())?
}

#[derive(Clone, Serialize)]
struct Progress<'a> { id: &'a str, request_id: &'a str, phase: &'a str, bytes: u64, total: u64 }

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) { Err("Model download cancelled".into()) } else { Ok(()) }
}

async fn download_file(
    client: &reqwest::Client, file: &ModelFile, dir: &Path, cancel: &AtomicBool,
    notify: &impl Fn(u64),
) -> Result<(), String> {
    check_cancel(cancel)?;
    let mut response = tokio::select! {
        result = client.get(&file.url).header(reqwest::header::ACCEPT_ENCODING, "identity").send() => result.map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?,
        _ = cancelled(cancel) => return Err("Model download cancelled".into()),
    };
    if response.status() != reqwest::StatusCode::OK || response.content_length().is_some_and(|n| n != file.bytes) {
        return Err(format!("Unexpected size or response for {}", file.name));
    }
    let mut output = tokio::fs::OpenOptions::new().write(true).create_new(true)
        .open(dir.join(&file.name)).await.map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut received = 0u64;
    let mut last = std::time::Instant::now();
    loop {
        let chunk = tokio::select! {
            result = response.chunk() => result.map_err(|e| e.to_string())?,
            _ = cancelled(cancel) => return Err("Model download cancelled".into()),
        };
        let Some(chunk) = chunk else { break };
        check_cancel(cancel)?;
        received += chunk.len() as u64;
        if received > file.bytes { return Err(format!("{} exceeds its verified size", file.name)); }
        output.write_all(&chunk).await.map_err(|e| e.to_string())?;
        digest.update(&chunk);
        if last.elapsed().as_millis() >= 150 { notify(received); last = std::time::Instant::now(); }
    }
    if received != file.bytes || format!("{:x}", digest.finalize()) != file.sha256 {
        return Err(format!("{} failed size or checksum verification; nothing was activated", file.name));
    }
    output.flush().await.map_err(|e| e.to_string())?;
    output.sync_all().await.map_err(|e| e.to_string())?;
    notify(received);
    Ok(())
}

async fn cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) { tokio::time::sleep(std::time::Duration::from_millis(100)).await; }
}

// All files and their licence receipt become visible as one immutable pack.
// A failed or cancelled attempt never modifies a previously installed pack.
async fn stage_pack(
    root: &Path, pack: &Pack, client: &reqwest::Client, cancel: &AtomicBool,
    notify: impl Fn(&str, u64),
) -> Result<PathBuf, String> {
    let parent = root.join(&pack.id);
    tokio::fs::create_dir_all(&parent).await.map_err(|e| e.to_string())?;
    let staging = parent.join(format!(".download-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir(&staging).await.map_err(|e| e.to_string())?;
    let result = async {
        let mut bytes = 0;
        for file in &pack.files {
            download_file(client, file, &staging, cancel, &|received| notify("Downloading", bytes + received)).await?;
            bytes += file.bytes;
        }
        check_cancel(cancel)?;
        notify("Verifying", bytes);
        let receipt = Receipt {
            revision: pack.revision(), accepted: true, accepted_at: chrono::Utc::now().to_rfc3339(),
            provider: pack.provider.clone(), licence_url: pack.licence_url.clone(),
        };
        tokio::fs::write(staging.join("licence.txt"), &pack.licence_text).await.map_err(|e| e.to_string())?;
        tokio::fs::write(staging.join("licence-receipt.json"), serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())?;
        check_cancel(cancel)?;
        Ok(staging.clone())
    }.await;
    if result.is_err() { let _ = tokio::fs::remove_dir_all(&staging).await; }
    result
}

fn publish_pack(root: &Path, pack: &Pack, staging: &Path, cancel: &AtomicBool) -> Result<(), String> {
    check_cancel(cancel)?;
    // The OS releases this lock if a process exits, including during repair.
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .open(root.join(&pack.id).join(".publish.lock")).map_err(|e| e.to_string())?;
    lock.try_lock().map_err(|_| "Another LapCasa process is installing this model pack. Retry when it finishes.".to_string())?;
    let destination = installed_dir(root, pack);
    // Another process may have finished the same pack while we downloaded it.
    if installed(pack, &destination) { return Ok(()); }
    let previous = destination.with_file_name(format!("previous-{}", uuid::Uuid::new_v4()));
    let had_previous = destination.exists();
    if had_previous {
        std::fs::rename(&destination, &previous).map_err(|e| format!("Cannot repair this model pack while it is in use: {e}. Close other LapCasa windows and retry."))?;
    }
    if let Err(error) = std::fs::rename(staging, &destination) {
        if had_previous && !destination.exists() { let _ = std::fs::rename(&previous, &destination); }
        return Err(error.to_string());
    }
    Ok(())
}

fn publish_active(root: &Path, pack: &Pack, staging: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let mut active = ACTIVE.lock().map_err(|e| e.to_string())?;
    publish_pack(root, pack, staging, cancel)?;
    if let Some(active) = active.as_mut() { active.committed = true; }
    Ok(())
}

#[tauri::command]
pub async fn model_download(app: tauri::AppHandle, id: String, request_id: String, accepted: bool, revision: String) -> Result<(), String> {
    let pack = pack(&id)?.clone();
    pack.consent(accepted, &revision)?; // No network, files or activation before consent.
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut active = ACTIVE.lock().map_err(|e| e.to_string())?;
        if active.is_some() { return Err("Another model download is already running".into()); }
        *active = Some(Active { id: id.clone(), request_id: request_id.clone(), cancelled: cancel.clone(), committed: false });
    }
    let _guard = DownloadGuard;
    let root = root()?;
    let destination = installed_dir(&root, &pack);
    if installed(&pack, &destination) { return Ok(()); }
    let client = reqwest::Client::builder().https_only(true)
        .connect_timeout(std::time::Duration::from_secs(15))
        .read_timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build().map_err(|e| e.to_string())?;
    let total = pack.files.iter().map(|f| f.bytes).sum();
    let notify = |phase: &str, bytes: u64| { let _ = app.emit("model-download-progress", Progress { id: &id, request_id: &request_id, phase, bytes, total }); };
    notify("Downloading", 0);
    let staging = stage_pack(&root, &pack, &client, &cancel, &notify).await?;
    // Serialize final publication with Cancel; accepting Cancel always prevents activation.
    let publish = publish_active(&root, &pack, &staging, &cancel);
    // Also discard our verified staging folder if another process installed first.
    if staging.exists() { let _ = tokio::fs::remove_dir_all(&staging).await; }
    if let Err(error) = publish {
        return Err(error);
    }
    notify("Installed", total);
    if id == "selection" { crate::t_edit_select::models_installed(); }
    Ok(())
}

#[tauri::command]
pub fn model_cancel(id: String, request_id: String) -> Result<bool, String> {
    let active = ACTIVE.lock().map_err(|e| e.to_string())?;
    if let Some(active) = active.as_ref().filter(|a| a.id == id && a.request_id == request_id && !a.committed) {
        active.cancelled.store(true, Ordering::Release);
        return Ok(true);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    static REQUEST_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn fixture() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lap-model-tests-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        dir
    }

    fn example(url: String, content: &[u8]) -> Pack {
        Pack { id: "example".into(), label: "Example".into(), purpose: "Test".into(), provider: "Synthetic".into(),
            licence: "Test licence".into(), licence_url: "https://example.invalid/terms".into(), licence_text: "Test terms".into(),
            restriction: String::new(), unavailable: String::new(), files: vec![ModelFile {
                name: "model.onnx".into(), url, bytes: content.len() as u64, sha256: format!("{:x}", Sha256::digest(content)),
            }] }
    }

    fn server(body: &'static [u8]) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/model", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 2048]; let _ = stream.read(&mut request);
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            stream.write_all(body).unwrap();
        });
        (url, handle)
    }

    #[test]
    fn acceptance_is_bound_to_exact_terms_and_files() {
        let mut p = example("https://huggingface.co/example/model".into(), b"model");
        let revision = p.revision();
        assert!(p.consent(false, &revision).is_err());
        assert!(p.consent(true, "old-terms").is_err());
        assert!(p.consent(true, &revision).is_ok());
        p.licence_text.push_str(" changed");
        assert!(p.consent(true, &revision).is_err());
        let revision = p.revision(); p.files[0].sha256 = "0".repeat(64);
        assert!(p.consent(true, &revision).is_err());
    }

    #[test]
    fn catalogue_has_only_fixed_packs_and_explicit_restrictions() {
        assert_eq!(catalogue().len(), 5);
        assert!(pack("../outside").is_err());
        assert!(pack("people").unwrap().restriction.contains("Non-commercial research"));
        for p in catalogue().iter().filter(|p| p.unavailable.is_empty()) { p.consent(true, &p.revision()).unwrap(); }
    }

    #[test]
    fn stale_cancel_cannot_cancel_a_later_download() {
        let _serial = REQUEST_TEST_LOCK.lock().unwrap();
        let token = Arc::new(AtomicBool::new(false));
        *ACTIVE.lock().unwrap() = Some(Active { id: "people".into(), request_id: "current".into(), cancelled: token.clone(), committed: false });
        let _guard = DownloadGuard;
        assert!(!model_cancel("people".into(), "previous".into()).unwrap());
        assert!(!model_cancel("search".into(), "current".into()).unwrap());
        assert!(!token.load(Ordering::Acquire));
        assert!(model_cancel("people".into(), "current".into()).unwrap());
        assert!(token.load(Ordering::Acquire));
    }

    #[test]
    fn cancellation_is_accepted_only_before_publication() {
        let _serial = REQUEST_TEST_LOCK.lock().unwrap();
        let root = fixture(); let p = example("https://huggingface.co/example/model".into(), b"valid model");
        let staging = root.join("staged"); std::fs::create_dir(&staging).unwrap();
        std::fs::write(staging.join("model.onnx"), b"valid model").unwrap();
        std::fs::write(staging.join("licence-receipt.json"), serde_json::to_vec(&Receipt {
            revision: p.revision(), accepted: true, accepted_at: "test".into(), provider: p.provider.clone(), licence_url: p.licence_url.clone(),
        }).unwrap()).unwrap();
        std::fs::create_dir(root.join(&p.id)).unwrap();
        let token = Arc::new(AtomicBool::new(false));
        *ACTIVE.lock().unwrap() = Some(Active { id: p.id.clone(), request_id: "current".into(), cancelled: token.clone(), committed: false });
        let _guard = DownloadGuard;
        assert!(model_cancel(p.id.clone(), "current".into()).unwrap());
        assert!(publish_active(&root, &p, &staging, &token).is_err());
        assert!(!installed_dir(&root, &p).exists());
        token.store(false, Ordering::Release);
        publish_active(&root, &p, &staging, &token).unwrap();
        assert!(installed(&p, &installed_dir(&root, &p)));
        assert!(!model_cancel(p.id.clone(), "current".into()).unwrap());
        assert!(!token.load(Ordering::Acquire));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn verified_pack_is_staged_whole_with_a_licence_receipt() {
        let root = fixture(); let (url, server) = server(b"valid model");
        let p = example(url, b"valid model"); let cancel = AtomicBool::new(false);
        let staged = stage_pack(&root, &p, &reqwest::Client::new(), &cancel, |_, _| {}).await.unwrap();
        assert!(!installed_dir(&root, &p).exists());
        assert!(installed(&p, &staged));
        assert_eq!(std::fs::read(staged.join("model.onnx")).unwrap(), b"valid model");
        assert!(staged.join("licence.txt").exists());
        server.join().unwrap(); std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn corrupt_or_truncated_download_never_activates_or_replaces_a_pack() {
        for bad in [b"wrong model".as_slice(), b"short".as_slice()] {
            let root = fixture(); let (url, server) = server(bad);
            let p = example(url, b"valid model");
            let existing = root.join("previous-version"); std::fs::write(&existing, b"previous good bytes").unwrap();
            assert!(stage_pack(&root, &p, &reqwest::Client::new(), &AtomicBool::new(false), |_, _| {}).await.is_err());
            assert!(!installed_dir(&root, &p).exists());
            assert_eq!(std::fs::read(existing).unwrap(), b"previous good bytes");
            assert_eq!(std::fs::read_dir(root.join(&p.id)).unwrap().count(), 0);
            server.join().unwrap(); std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn cancellation_does_not_contact_server_or_leave_a_partial_pack() {
        let root = fixture(); let p = example("http://127.0.0.1:1/never".into(), b"valid model");
        let cancel = AtomicBool::new(true);
        let result = stage_pack(&root, &p, &reqwest::Client::new(), &cancel, |_, _| {}).await;
        assert!(result.unwrap_err().contains("cancelled"));
        assert!(!installed_dir(&root, &p).exists());
        assert_eq!(std::fs::read_dir(root.join(&p.id)).unwrap().count(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn cancelling_a_stalled_response_cleans_the_download() {
        let root = fixture();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let p = example(format!("http://{}/model", listener.local_addr().unwrap()), b"valid model");
        let (sent, ready) = tokio::sync::oneshot::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 2048]; let _ = stream.read(&mut request);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nv").unwrap();
            let _ = sent.send(());
            std::thread::sleep(std::time::Duration::from_millis(400));
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        tokio::spawn(async move { let _ = ready.await; token.store(true, Ordering::Release); });
        let result = stage_pack(&root, &p, &reqwest::Client::new(), &cancel, |_, _| {}).await;
        assert!(result.unwrap_err().contains("cancelled"));
        assert!(!installed_dir(&root, &p).exists());
        assert_eq!(std::fs::read_dir(root.join(&p.id)).unwrap().count(), 0);
        server.join().unwrap(); std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn second_file_failure_does_not_publish_a_mixed_pack() {
        let root = fixture();
        let (url, first) = server(b"valid model");
        let mut p = example(url, b"valid model");
        let (url, second) = server(b"wrong model");
        let mut other = p.files[0].clone(); other.name = "second.onnx".into(); other.url = url;
        p.files.push(other);
        assert!(stage_pack(&root, &p, &reqwest::Client::new(), &AtomicBool::new(false), |_, _| {}).await.is_err());
        assert!(!installed_dir(&root, &p).exists());
        assert_eq!(std::fs::read_dir(root.join(&p.id)).unwrap().count(), 0);
        first.join().unwrap(); second.join().unwrap(); std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn repair_preserves_incomplete_files_until_verified_replacement_is_ready() {
        let root = fixture(); let (url, server) = server(b"valid model");
        let p = example(url, b"valid model");
        let destination = installed_dir(&root, &p);
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(destination.join("old-partial"), b"keep for inspection").unwrap();
        let staged = stage_pack(&root, &p, &reqwest::Client::new(), &AtomicBool::new(false), |_, _| {}).await.unwrap();
        assert!(destination.join("old-partial").exists());
        assert!(publish_pack(&root, &p, &staged, &AtomicBool::new(true)).is_err());
        assert!(destination.join("old-partial").exists());
        publish_pack(&root, &p, &staged, &AtomicBool::new(false)).unwrap();
        assert!(installed(&p, &destination));
        let previous = std::fs::read_dir(root.join(&p.id)).unwrap().map(|e| e.unwrap().path())
            .find(|p| p.file_name().unwrap().to_string_lossy().starts_with("previous-")).unwrap();
        assert_eq!(std::fs::read(previous.join("old-partial")).unwrap(), b"keep for inspection");
        server.join().unwrap(); std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incomplete_or_unconsented_pack_is_not_usable() {
        let root = fixture(); let p = example("https://huggingface.co/example/model".into(), b"valid model");
        std::fs::write(root.join("model.onnx"), b"valid model").unwrap();
        assert!(!installed(&p, &root));
        std::fs::write(root.join("licence-receipt.json"), serde_json::to_vec(&Receipt {
            revision: p.revision(), accepted: false, accepted_at: "test".into(), provider: p.provider.clone(), licence_url: p.licence_url.clone(),
        }).unwrap()).unwrap();
        assert!(!installed(&p, &root));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn publication_is_serialized_across_processes() {
        const CHILD_ROOT: &str = "LAP_MODEL_PUBLICATION_TEST_ROOT";
        let p = example("https://huggingface.co/example/model".into(), b"valid model");
        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            let root = PathBuf::from(root);
            let result = publish_pack(&root, &p, &root.join("staged"), &AtomicBool::new(false));
            assert!(result.unwrap_err().contains("Another LapCasa process"));
            assert!(installed(&p, &installed_dir(&root, &p)));
            return;
        }
        let root = fixture();
        let destination = installed_dir(&root, &p);
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(destination.join("model.onnx"), b"valid model").unwrap();
        std::fs::write(destination.join("licence-receipt.json"), serde_json::to_vec(&Receipt {
            revision: p.revision(), accepted: true, accepted_at: "test".into(), provider: p.provider.clone(), licence_url: p.licence_url.clone(),
        }).unwrap()).unwrap();
        let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false)
            .open(root.join(&p.id).join(".publish.lock")).unwrap();
        lock.try_lock().unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "t_models::tests::publication_is_serialized_across_processes", "--nocapture"])
            .env(CHILD_ROOT, &root).output().unwrap();
        assert!(child.status.success(), "{}{}", String::from_utf8_lossy(&child.stdout), String::from_utf8_lossy(&child.stderr));
        assert!(installed(&p, &destination));
        drop(lock);
        // The waiting process rechecks under the lock and preserves the winner.
        publish_pack(&root, &p, &root.join("staged"), &AtomicBool::new(false)).unwrap();
        assert_eq!(std::fs::read_dir(root.join(&p.id)).unwrap().count(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }
}
