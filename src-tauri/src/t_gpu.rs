//! App-wide ONNX acceleration policy and observable session loading (LAP-104).
use ort::session::{Session, builder::SessionBuilder};
use serde::Serialize;
use tauri::{Emitter, Manager};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::{Path, PathBuf}, sync::{Mutex, OnceLock}};

#[derive(Clone, Serialize)]
pub struct Hardware {
    pub name: Option<String>,
    pub vram_mb: Option<u64>,
    pub driver: Option<String>,
    pub cuda: bool,
    pub reason: String,
    pub missing: Vec<String>,
}
#[derive(Clone, Serialize)]
pub struct EngineStatus { pub provider: String, pub reason: String }
static HARDWARE: OnceLock<Mutex<Hardware>> = OnceLock::new();
static ENGINES: Mutex<BTreeMap<String, EngineStatus>> = Mutex::new(BTreeMap::new());
static LIVE: Mutex<BTreeMap<u64, (String, EngineStatus)>> = Mutex::new(BTreeMap::new());
static NEXT_SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Session ownership is also status ownership: dropping an engine removes its live report.
pub struct GpuSession { session: Session, id: u64 }
impl std::ops::Deref for GpuSession { type Target = Session; fn deref(&self) -> &Session { &self.session } }
impl std::ops::DerefMut for GpuSession { fn deref_mut(&mut self) -> &mut Session { &mut self.session } }
impl Drop for GpuSession { fn drop(&mut self) { if let Ok(mut live) = LIVE.lock() { live.remove(&self.id); } } }
fn tracked(session: Session, name: &str, provider: &str, reason: String) -> GpuSession {
    let id = NEXT_SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    ENGINES.lock().unwrap().remove(name);
    LIVE.lock().unwrap().insert(id, (name.into(), EngineStatus { provider: provider.into(), reason }));
    GpuSession { session, id }
}
fn reports() -> BTreeMap<String, EngineStatus> {
    let mut reports = ENGINES.lock().unwrap().clone();
    for (name, status) in LIVE.lock().unwrap().values() {
        match reports.get_mut(name) {
            Some(previous) if previous.provider != status.provider && previous.provider != "unavailable" => {
                previous.provider = "mixed".into();
                if !status.reason.is_empty() { previous.reason = status.reason.clone(); }
            }
            _ => { reports.insert(name.clone(), status.clone()); }
        }
    }
    reports
}
const DLLS: &[&str] = &["cudart64_12.dll", "cublasLt64_12.dll", "cublas64_12.dll", "cufft64_11.dll", "cudnn64_9.dll"];

pub fn runtime_dir() -> Result<PathBuf, String> {
    Ok(crate::t_config::get_app_data_dir()?.join("models/gpu-runtime"))
}

#[cfg(windows)]
fn load_runtime() -> Vec<String> {
    use windows::{core::PCWSTR, Win32::System::LibraryLoader::{LoadLibraryExW, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, AddDllDirectory, SetDefaultDllDirectories}};
    let dir = runtime_dir().ok().map(|d| d.join("current"));
    if let Some(dir) = dir.as_ref().filter(|d| d.is_dir()) {
        let wide: Vec<u16> = dir.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
        // Keep the directory cookie and DLL handles for the process lifetime. ORT loads cuDNN
        // sub-libraries lazily; removing the directory after probing would break inference.
        unsafe {
            let _ = AddDllDirectory(PCWSTR(wide.as_ptr()));
            let _ = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
        }
    }
    DLLS.iter().filter_map(|name| {
        let local = dir.as_ref().map(|d| d.join(name)).filter(|p| p.exists());
        let path = local.clone().unwrap_or_else(|| PathBuf::from(name));
        let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
        let flags = if local.is_some() { LOAD_LIBRARY_SEARCH_DEFAULT_DIRS | LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR } else { LOAD_LIBRARY_SEARCH_DEFAULT_DIRS };
        unsafe { LoadLibraryExW(PCWSTR(wide.as_ptr()), None, flags) }.err().map(|e| format!("{name}: {e}"))
    }).collect()
}
#[cfg(not(windows))]
fn load_runtime() -> Vec<String> { vec!["GPU add-on is supported on Windows".into()] }

fn probe() -> Hardware {
    let mut h = Hardware { name: None, vram_mb: None, driver: None, cuda: false, reason: "no NVIDIA GPU".into(), missing: vec![] };
    if std::env::var("LAP_SELECT_CPU").as_deref() == Ok("1") || std::env::var("LAP_GPU_NO_GPU").as_deref() == Ok("1") { return h; }
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
        if let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() {
            let mut index = 0;
            while let Ok(adapter) = factory.EnumAdapters1(index) {
                if let Ok(desc) = adapter.GetDesc1() {
                    if desc.VendorId == 0x10de {
                        let end = desc.Description.iter().position(|c| *c == 0).unwrap_or(desc.Description.len());
                        h.name = Some(String::from_utf16_lossy(&desc.Description[..end]));
                        h.vram_mb = Some((desc.DedicatedVideoMemory / (1024 * 1024)) as u64);
                        break;
                    }
                }
                index += 1;
            }
        }
    }
    let mut command = std::process::Command::new("nvidia-smi");
    command.args(["--query-gpu=name,memory.total,driver_version", "--format=csv,noheader,nounits"]);
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
    if let Ok(output) = command.output() {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            let fields: Vec<_> = text.lines().next().unwrap_or("").split(',').map(str::trim).collect();
            if fields.len() == 3 {
                h.name = Some(fields[0].into()); h.vram_mb = fields[1].parse().ok(); h.driver = Some(fields[2].into());
            }
        }
    }
    if h.name.is_none() { return h; }
    if h.driver.is_none() { h.reason = "NVIDIA driver unavailable".into(); return h; }
    if !cfg!(feature = "gpu-providers") { h.reason = "GPU providers are not included in this build".into(); return h; }
    h.missing = load_runtime();
    h.cuda = h.missing.is_empty();
    h.reason = if h.cuda { String::new() } else { "GPU add-on is missing or could not load".into() };
    h
}
pub fn hardware() -> Hardware { HARDWARE.get_or_init(|| Mutex::new(probe())).lock().unwrap().clone() }
pub fn refresh() -> Hardware {
    let h = probe();
    *HARDWARE.get_or_init(|| Mutex::new(h.clone())).lock().unwrap() = h.clone();
    h
}
fn record(name: &str, provider: &str, reason: String) {
    ENGINES.lock().unwrap().insert(name.into(), EngineStatus { provider: provider.into(), reason });
}
pub fn engine(name: &str) -> Option<EngineStatus> { reports().remove(name) }

/// The only provider selection site. A failed CUDA load is reported even when CPU is allowed.
/// Strict diagnostic/background callers fail instead of silently changing their execution policy.
pub fn load(path: &Path, require_cuda: bool, builder: impl Fn() -> Result<SessionBuilder, String>) -> Result<GpuSession, String> {
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let h = hardware();
    let mut reason = h.reason;
    #[cfg(feature = "gpu-providers")]
    if h.cuda {
        let attempt = (|| {
            let b = builder()?.with_execution_providers([ort::execution_providers::CUDAExecutionProvider::default().build().error_on_failure()]).map_err(|e| e.to_string())?;
            b.commit_from_file(path).map_err(|e| e.to_string())
        })();
        match attempt {
            Ok(session) => { return Ok(tracked(session, &name, "cuda", String::new())); }
            Err(e) => { reason = e; }
        }
    }
    if require_cuda {
        record(&name, "unavailable", reason.clone());
        return Err(reason);
    }
    match builder()?.commit_from_file(path).map_err(|e| e.to_string()) {
        Ok(session) => { Ok(tracked(session, &name, "cpu", reason)) }
        Err(e) => { record(&name, "unavailable", e.clone()); Err(e) }
    }
}

#[tauri::command]
pub async fn gpu_status() -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let h = hardware();
        let mut engines = reports();
        for name in ["sam2_tiny_encoder.onnx", "sam2_tiny_decoder.onnx", "det_500m.onnx", "w600k_mbf.onnx", "vision_model.onnx", "text_model.onnx", "yolov10n.onnx", "ocr_det.onnx", "ocr_cls.onnx", "ocr_rec.onnx"] {
            engines.entry(name.into()).or_insert_with(|| EngineStatus { provider: "not_loaded".into(), reason: if h.cuda { "Loads on demand".into() } else { h.reason.clone() } });
        }
        let prompt = h.name.is_some() && h.driver.is_some() && !h.cuda && !h.missing.is_empty() && cfg!(feature = "gpu-providers") && runtime_dir().map(|d| !d.join("prompt-dismissed").exists()).unwrap_or(false);
        serde_json::json!({"hardware": h, "engines": engines, "prompt": prompt})
    }).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn gpu_dismiss_prompt() -> Result<(), String> {
    let dir = runtime_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("prompt-dismissed"), b"1").map_err(|e| e.to_string())
}

// Official NVIDIA manifests, pinned on 2026-11-02: CUDA 12.8.1 and cuDNN 9.8.0.
// Downloaded directly from NVIDIA, never bundled in the GPL application archive.
const PACKAGES: &[(&str, &str, u64)] = &[
    ("cuda/redist/cuda_cudart/windows-x86_64/cuda_cudart-windows-x86_64-12.8.90-archive.zip", "4a39058fd8519444a81cfc7ae055d136f48d1a31ffa41ae255b35b2edd61e13b", 3037735),
    ("cuda/redist/libcublas/windows-x86_64/libcublas-windows-x86_64-12.8.4.1-archive.zip", "57a470112cec7e112c95253dde8b3c7184d795dbd92b0bde77a4cb7f8c94c8aa", 563660944),
    ("cuda/redist/libcufft/windows-x86_64/libcufft-windows-x86_64-11.3.3.83-archive.zip", "cc6e0ba958cf23387b462017a24464c72bd901549046133f3d1ebcc3d7444c90", 190568498),
    ("cudnn/redist/cudnn/windows-x86_64/cudnn-windows-x86_64-9.8.0.87_cuda12-archive.zip", "d8a23705e3884b137b7e05449fb2b61bfa524e7cfc3fda80743d633f423c6ce4", 675349654),
];
static INSTALLING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
struct InstallGuard;
impl Drop for InstallGuard { fn drop(&mut self) { INSTALLING.store(false, std::sync::atomic::Ordering::Release); } }
struct StagingGuard(PathBuf);
impl Drop for StagingGuard {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}
fn progress(app: &tauri::AppHandle, phase: &str, bytes: u64) {
    let _ = app.emit("gpu-install-progress", serde_json::json!({"phase": phase, "bytes": bytes, "total": PACKAGES.iter().map(|p| p.2).sum::<u64>()}));
}

fn unpack(archive: &Path, dest: &Path, package: usize) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() { continue; }
        let path = entry.enclosed_name().ok_or("Unsafe path in NVIDIA archive")?.to_path_buf();
        let name = path.file_name().ok_or("Missing archive filename")?.to_string_lossy().into_owned();
        let lower = name.to_ascii_lowercase();
        let dll = lower.ends_with(".dll") && path.components().any(|c| c.as_os_str() == "bin");
        let notice = lower.contains("license") || lower.contains("eula") || lower.contains("notice");
        if !dll && !notice { continue; }
        let target = if dll { dest.join(name) } else { dest.join(format!("{package}-{name}")) };
        let mut file = std::fs::File::create(target).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut file).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn gpu_install(app: tauri::AppHandle, accept_nvidia_terms: bool) -> Result<(), String> {
    if !accept_nvidia_terms { return Err("Accept the NVIDIA CUDA and cuDNN terms before downloading".into()); }
    if !cfg!(windows) || hardware().name.is_none() { return Err("No supported NVIDIA GPU".into()); }
    if INSTALLING.swap(true, std::sync::atomic::Ordering::AcqRel) { return Err("GPU add-on installation is already running".into()); }
    let _guard = InstallGuard;
    let result = install(&app).await;
    if let Err(ref e) = result { progress(&app, &format!("Failed: {e}"), 0); }
    result
}

// Retain bytes in the app-owned package file across firewall windows and app restarts.
// Only network failures retry: disk, protocol and checksum errors remain actionable errors.
async fn download_archive(
    client: &reqwest::Client, url: &str, archive: &Path, expected: &str, size: u64,
    retry_delay: std::time::Duration, notify: impl Fn(&str, u64),
) -> Result<(), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut file = tokio::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true)
        .open(archive).await.map_err(|e| e.to_string())?;
    let mut received = file.metadata().await.map_err(|e| e.to_string())?.len();
    if received > size {
        file.set_len(0).await.map_err(|e| e.to_string())?;
        received = 0;
    }
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if n == 0 { break; }
        hash.update(&buffer[..n]);
    }
    notify("Downloading", received);
    while received < size {
        let mut request = client.get(url).header(reqwest::header::ACCEPT_ENCODING, "identity");
        if received > 0 { request = request.header(reqwest::header::RANGE, format!("bytes={received}-")); }
        let response = request.send().await;
        let mut response = match response {
            Ok(response) if response.status().is_success() => response,
            Ok(response) if !(response.status().is_server_error() || matches!(response.status().as_u16(), 408 | 429)) => {
                return Err(format!("NVIDIA download returned HTTP {}", response.status()));
            }
            _ => {
                notify("Waiting for network access — approve the firewall prompt; retrying automatically", received);
                tokio::time::sleep(retry_delay).await;
                continue;
            }
        };
        if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            let wanted = format!("bytes {received}-{}/{size}", size - 1);
            if response.headers().get(reqwest::header::CONTENT_RANGE).and_then(|v| v.to_str().ok()) != Some(wanted.as_str()) {
                return Err("Invalid NVIDIA download resume range; nothing was activated".into());
            }
        } else if response.status() == reqwest::StatusCode::OK {
            // A server may ignore Range. Restart safely instead of appending duplicate bytes.
            use tokio::io::AsyncSeekExt;
            file.set_len(0).await.map_err(|e| e.to_string())?;
            file.rewind().await.map_err(|e| e.to_string())?;
            received = 0;
            hash = Sha256::new();
        } else {
            return Err(format!("Unexpected NVIDIA download HTTP {}", response.status()));
        }
        let mut last = std::time::Instant::now();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if received + chunk.len() as u64 > size { return Err("NVIDIA archive exceeds its pinned size".into()); }
                    file.write_all(&chunk).await.map_err(|e| e.to_string())?;
                    hash.update(&chunk);
                    received += chunk.len() as u64;
                    if last.elapsed().as_millis() > 150 { notify("Downloading", received); last = std::time::Instant::now(); }
                }
                Ok(None) | Err(_) => break,
            }
        }
        file.flush().await.map_err(|e| e.to_string())?;
        if received < size {
            notify("Waiting for network access — approve the firewall prompt; retrying automatically", received);
            tokio::time::sleep(retry_delay).await;
        }
    }
    if format!("{:x}", hash.finalize()) != expected {
        // Discard corrupt bytes so the next explicit attempt can recover.
        file.set_len(0).await.map_err(|e| e.to_string())?;
        return Err("NVIDIA archive checksum mismatch; nothing was activated. Retry the download.".into());
    }
    Ok(())
}

async fn install(app: &tauri::AppHandle) -> Result<(), String> {
    let root = runtime_dir()?;
    let staging = root.join(format!("staging-{}", chrono::Utc::now().timestamp_millis()));
    let _cleanup = StagingGuard(staging.clone());
    let current = root.join("current");
    // Never overwrite DLLs mapped by this process. Retry warm-up if an installation already works.
    if !hardware().cuda {
        tokio::fs::create_dir_all(&staging).await.map_err(|e| e.to_string())?;
        let client = reqwest::Client::builder().https_only(true)
            .connect_timeout(std::time::Duration::from_secs(10))
            .read_timeout(std::time::Duration::from_secs(30))
            .build().map_err(|e| e.to_string())?;
        let mut bytes = 0;
        for (index, (url, expected, size)) in PACKAGES.iter().enumerate() {
            let archive = root.join(format!("package-{index}.zip"));
            download_archive(&client, &format!("https://developer.download.nvidia.com/compute/{url}"),
                &archive, expected, *size, std::time::Duration::from_secs(5),
                |phase, received| progress(app, phase, bytes + received)).await?;
            bytes += size;
            progress(app, "Verifying and extracting", bytes);
            let dest = staging.clone(); let source = archive.clone();
            tauri::async_runtime::spawn_blocking(move || unpack(&source, &dest, index)).await.map_err(|e| e.to_string())??;
        }
        for dll in DLLS { if !staging.join(dll).is_file() { return Err(format!("Archive missing {dll}")); } }
        if current.exists() {
            // A broken previous installation is kept for diagnosis. Never delete mapped files.
            let backup = root.join(format!("previous-{}", chrono::Utc::now().timestamp_millis()));
            tokio::fs::rename(&current, backup).await.map_err(|e| format!("Cannot replace loaded runtime: {e}. Close and reopen LapCasa, then repair."))?;
        }
        tokio::fs::rename(&staging, &current).await.map_err(|e| e.to_string())?;
        if !refresh().cuda { return Err(hardware().reason); }
        // Keep verified packages across interruptions; remove only after activation succeeds.
        for index in 0..PACKAGES.len() { let _ = tokio::fs::remove_file(root.join(format!("package-{index}.zip"))).await; }
    }
    progress(app, "Preparing AI tools — the first warm-up can take a minute", PACKAGES.iter().map(|p| p.2).sum());
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let models = app.path().resource_dir().map_err(|e| e.to_string())?.join("models");
        if crate::t_models::available("selection", &models) { crate::t_edit_select::reset_and_warm(&crate::t_models::resolve("selection", &models))?; }
        if crate::t_models::available("people", &models) {
            let state = app.state::<crate::t_face::FaceState>();
            let mut faces = state.0.lock().map_err(|e| e.to_string())?;
            faces.unload_models(); faces.load_models_from_dir(&crate::t_models::resolve("people", &models))?;
        }
        if crate::t_models::available("search", &models) {
            let state = app.state::<crate::t_ai::AiState>();
            let mut ai = state.0.lock().map_err(|e| e.to_string())?;
            let model = ai.model_status(&app).active_model;
            ai.unload_models(); ai.load_models(&app)?;
            ai.set_text_model(&app, crate::t_ai::ImageSearchTextModel::from_i64(model))?;
        }
        crate::t_advanced_ai::prepare_gpu_engines(&models)?;
        gpu_dismiss_prompt()?;
        progress(&app, "Ready", PACKAGES.iter().map(|p| p.2).sum());
        let _ = app.emit("gpu-ready", ());
        Ok::<(), String>(())
    }).await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn download_server(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/package", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let mut requests = vec![];
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                requests.push(String::from_utf8(request).unwrap().to_ascii_lowercase());
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, thread)
    }
    async fn test_download(tag: &str, initial: &[u8], responses: Vec<String>, expected: &str) -> (Result<(), String>, Vec<String>, Vec<u8>) {
        let root = std::env::temp_dir().join(format!("lap104-download-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let archive = root.join("package.zip");
        std::fs::write(&archive, initial).unwrap();
        let (url, server) = download_server(responses);
        let client = reqwest::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(2)).build().unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), download_archive(
            &client, &url, &archive, expected, 6, std::time::Duration::from_millis(1), |_, _| {},
        )).await.unwrap();
        let bytes = std::fs::read(&archive).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        (result, server.join().unwrap(), bytes)
    }
    #[tokio::test]
    async fn download_retries_service_failure_and_resumes_interrupted_body() {
        let hash = format!("{:x}", Sha256::digest(b"abcdef"));
        let (result, requests, bytes) = test_download("interrupted", b"", vec![
            "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabc".into(),
            "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\nConnection: close\r\n\r\ndef".into(),
        ], &hash).await;
        result.unwrap();
        assert!(requests[2].contains("range: bytes=3-"));
        assert_eq!(bytes, b"abcdef");
    }
    #[tokio::test]
    async fn download_resumes_after_restart_and_handles_ignored_range() {
        let hash = format!("{:x}", Sha256::digest(b"abcdef"));
        for (tag, response) in [
            ("restart", "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\nConnection: close\r\n\r\ndef"),
            ("ignored", "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabcdef"),
        ] {
            let (result, requests, bytes) = test_download(tag, b"abc", vec![response.into()], &hash).await;
            result.unwrap();
            assert!(requests[0].contains("range: bytes=3-"));
            assert_eq!(bytes, b"abcdef");
        }
        let (result, requests, _) = test_download("cached", b"abcdef", vec![], &hash).await;
        result.unwrap();
        assert!(requests.is_empty());
    }
    #[tokio::test]
    async fn download_rejects_bad_ranges_and_checksums() {
        let hash = format!("{:x}", Sha256::digest(b"abcdef"));
        let (result, _, bytes) = test_download("bad-range", b"abc", vec![
            "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 0-2/6\r\nConnection: close\r\n\r\ndef".into(),
        ], &hash).await;
        assert!(result.unwrap_err().contains("resume range"));
        assert_eq!(bytes, b"abc");
        let (result, _, bytes) = test_download("checksum", b"abcxyz", vec![], &hash).await;
        assert!(result.unwrap_err().contains("checksum mismatch"));
        assert!(bytes.is_empty());
    }
    fn fixture(tag: &str, entries: &[(&str, &[u8])]) -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("lap104-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(root.join("out")).unwrap();
        let archive = root.join("archive.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        for (name, data) in entries {
            writer.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
        (root.clone(), archive, root.join("out"))
    }
    #[test]
    fn extraction_preserves_notices_and_omits_headers_and_installers() {
        let (root, archive, dest) = fixture("extract", &[
            ("pkg/bin/cudnn64_9.dll", b"runtime"), ("pkg/LICENSE", b"terms"),
            ("pkg/include/header.h", b"header"), ("pkg/bin/setup.exe", b"installer"),
        ]);
        unpack(&archive, &dest, 3).unwrap();
        assert_eq!(std::fs::read(dest.join("cudnn64_9.dll")).unwrap(), b"runtime");
        assert_eq!(std::fs::read(dest.join("3-LICENSE")).unwrap(), b"terms");
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn extraction_rejects_archive_path_traversal() {
        let (root, archive, dest) = fixture("traversal", &[("../bin/escaped.dll", b"bad")]);
        assert!(unpack(&archive, &dest, 0).unwrap_err().contains("Unsafe path"));
        assert!(!root.join("escaped.dll").exists());
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}
