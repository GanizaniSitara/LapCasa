use crate::t_sqlite;

use ndarray::Array4;
use rusqlite::{params, Connection};

use image::{DynamicImage, GenericImageView, ImageDecoder, ImageReader, Rgb, RgbImage};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use std::time::Instant;
use tauri::AppHandle;
use tauri::Manager;

// PaddleOCR en_PP-OCRv4 recogniser. Output class 0 is the CTC blank; class i
// (i >= 1) is the (i-1)th entry of en_dict.txt followed by the space appended
// by use_space_char. en_dict.txt is ASCII 48..=126 then 33..=47.
const DET_MAX_SIDE: u32 = 960;
const DET_THRESH: f32 = 0.3;
const DET_BOX_THRESH: f32 = 0.6;
const DET_UNCLIP_RATIO: f32 = 1.5;
const CLS_HEIGHT: u32 = 48;
const CLS_WIDTH: u32 = 192;
const CLS_THRESH: f32 = 0.9;
const REC_HEIGHT: u32 = 48;
const REC_MIN_WIDTH: u32 = 320;
const REC_DROP_SCORE: f32 = 0.5;
const WORKER_BATCH: usize = 1;
static BACKGROUND_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static WORKER_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static CONFIG_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CUDA_AVAILABLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static GPU_ERRORS: std::sync::Mutex<[Option<String>; 3]> = std::sync::Mutex::new([None, None, None]);
// Add-on installation prepares these before announcing Ready. The worker takes ownership
// on its next pass, so the first user operation never pays session/kernel warm-up costs.
static PREPARED: std::sync::Mutex<(Option<SceneEngine>, Option<ObjectEngine>, Option<OcrEngine>)> = std::sync::Mutex::new((None, None, None));

pub fn prepare_gpu_engines(models: &Path) -> Result<(), String> {
    let scenes = if crate::t_models::available("search", models) { Some(SceneEngine::load(&crate::t_models::resolve("search", models))?) } else { None };
    let objects = if crate::t_models::available("objects", models) { Some(ObjectEngine::load(&crate::t_models::resolve("objects", models))?) } else { None };
    let ocr = if crate::t_models::available("ocr", models) { Some(OcrEngine::load(&crate::t_models::resolve("ocr", models))?) } else { None };
    *PREPARED.lock().map_err(|e| e.to_string())? = (scenes, objects, ocr);
    experimental_config_changed();
    Ok(())
}
thread_local! { static IN_WORKER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

#[derive(Clone, Copy)]
enum Stage { Scenes = 0, Objects = 1, Ocr = 2 }

fn opted_in(config: &crate::t_config::AppConfig, background: bool, stage: Stage) -> bool {
    background && config.process_library_in_background && match stage {
        Stage::Scenes => config.ai_scene_detection_enabled,
        Stage::Objects => config.ai_object_detection_enabled,
        Stage::Ocr => config.ai_ocr_enabled,
    }
}

fn stage_enabled(stage: Stage) -> bool {
    background_enabled() && crate::t_config::load_app_config()
        .map(|config| opted_in(&config, background_enabled(), stage)).unwrap_or(false)
}

fn checkpoint(stage: Stage) -> Result<(), String> {
    crate::scheduler::cooperative_yield_blocking();
    if IN_WORKER.with(|worker| worker.get()) && !stage_enabled(stage) {
        return Err("Experimental detection paused".into());
    }
    Ok(())
}

pub fn experimental_config_changed() {
    CONFIG_EPOCH.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    if let Ok(mut pending) = PENDING.lock() { *pending = None; }
}

fn set_stage_error(stage: Stage, error: Option<String>) {
    if let Ok(mut errors) = GPU_ERRORS.lock() { errors[stage as usize] = error; }
}

#[cfg(windows)]
fn lower_worker_priority() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> *mut std::ffi::c_void;
        fn SetThreadPriority(thread: *mut std::ffi::c_void, priority: i32) -> i32;
    }
    // Only this background thread; never change the process or desktop priority.
    unsafe { SetThreadPriority(GetCurrentThread(), -2); }
}
#[cfg(not(windows))]
fn lower_worker_priority() {}

pub fn background_enabled() -> bool {
    BACKGROUND_ENABLED.load(std::sync::atomic::Ordering::Acquire)
}

pub fn set_background_enabled(enabled: bool) {
    let enabled = enabled && !cfg!(feature = "vm-rig");
    BACKGROUND_ENABLED.store(enabled, std::sync::atomic::Ordering::Release);
    experimental_config_changed();
    if !enabled {
        set_current(None);
    }
}

// Picasa-style progress corner: the photo being processed now and what is left.
static CURRENT: std::sync::Mutex<Option<(i64, String)>> = std::sync::Mutex::new(None);
static PENDING: std::sync::Mutex<Option<(Instant, i64, i64, i64)>> = std::sync::Mutex::new(None);

fn set_current(file: Option<(i64, String)>) {
    if let Ok(mut current) = CURRENT.lock() {
        *current = file;
    }
}

#[derive(serde::Serialize)]
pub struct AiProgress {
    pub enabled: bool,
    pub file_id: Option<i64>,
    pub path: Option<String>,
    pub objects_pending: i64,
    pub ocr_pending: i64,
    pub scenes_pending: i64,
    pub descriptions_pending: i64,
    pub cuda_available: bool,
    pub gpu_status: Option<String>,
}

pub fn ai_progress() -> Result<AiProgress, String> {
    let (objects_pending, ocr_pending, scenes_pending) = {
        let mut pending = PENDING.lock().map_err(|e| e.to_string())?;
        match *pending {
            Some((at, objects, ocr, scenes)) if at.elapsed() < Duration::from_secs(10) => (objects, ocr, scenes),
            _ => {
                let conn = t_sqlite::open_conn()?;
                let (objects, ocr, scenes): (i64, i64, i64) = conn.query_row(
                    "SELECT COALESCE(SUM(a.ai_objects IS NULL), 0), COALESCE(SUM(a.ocr_text IS NULL), 0),
                            COALESCE(SUM(a.embeds IS NOT NULL AND a.ai_tags IS NULL), 0)
                     FROM afiles a JOIN afolders b ON a.folder_id = b.id
                     WHERE COALESCE(b.ai_excluded, 0) = 0 AND a.file_type IN (1, 2, 3)",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                ).map_err(|e| e.to_string())?;
                *pending = Some((Instant::now(), objects, ocr, scenes));
                (objects, ocr, scenes)
            }
        }
    };
    let current = CURRENT.lock().map_err(|e| e.to_string())?.clone();
    let errors = GPU_ERRORS.lock().map_err(|e| e.to_string())?.iter().flatten().cloned().collect::<Vec<_>>();
    let cuda_available = CUDA_AVAILABLE.load(std::sync::atomic::Ordering::Acquire);
    Ok(AiProgress {
        enabled: background_enabled(),
        file_id: current.as_ref().map(|c| c.0),
        path: current.map(|c| c.1),
        objects_pending,
        ocr_pending,
        scenes_pending,
        descriptions_pending: 0, // Summaries are synthesized on demand from existing detections.
        cuda_available,
        gpu_status: Some(if !errors.is_empty() { errors.join(" · ") }
            else if cuda_available { "CUDA ready; opted-in detection runs only while idle.".into() }
            else { "CUDA has not been initialized. Detection is opt-in and GPU-only.".into() }),
    })
}
const OBJECT_SCORE: f32 = 0.4;
const COCO_LABELS: [&str; 80] = [
    "person", "bicycle", "car", "motorcycle", "airplane", "bus", "train", "truck", "boat", "traffic light",
    "fire hydrant", "stop sign", "parking meter", "bench", "bird", "cat", "dog", "horse", "sheep", "cow",
    "elephant", "bear", "zebra", "giraffe", "backpack", "umbrella", "handbag", "tie", "suitcase", "frisbee",
    "skis", "snowboard", "sports ball", "kite", "baseball bat", "baseball glove", "skateboard", "surfboard", "tennis racket", "bottle",
    "wine glass", "cup", "fork", "knife", "spoon", "bowl", "banana", "apple", "sandwich", "orange",
    "broccoli", "carrot", "hot dog", "pizza", "donut", "cake", "chair", "couch", "potted plant", "bed",
    "dining table", "toilet", "tv", "laptop", "mouse", "remote", "keyboard", "cell phone", "microwave", "oven",
    "toaster", "sink", "refrigerator", "book", "clock", "vase", "scissors", "teddy bear", "hair drier", "toothbrush",
];

fn debug() -> bool {
    std::env::var_os("LAP_OCR_DEBUG").is_some()
}

fn build_vocab() -> Vec<char> {
    (48u8..=126).chain(33u8..=47).map(|b| b as char).chain(std::iter::once(' ')).collect()
}

fn worker_sleep(duration: Duration) {
    let deadline = Instant::now() + duration;
    let epoch = CONFIG_EPOCH.load(std::sync::atomic::Ordering::Acquire);
    while background_enabled() && Instant::now() < deadline && epoch == CONFIG_EPOCH.load(std::sync::atomic::Ordering::Acquire) {
        thread::sleep(deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(100)));
    }
}

/// Stages initialize independently and back off on failure without CPU fallback.
fn run_stage<T>(
    stage: Stage, engine: &mut Option<T>, retry_at: &mut Instant,
    load: impl FnOnce() -> Result<T, String>, run: impl FnOnce(&mut T) -> Result<usize, String>,
) -> usize {
    if !stage_enabled(stage) {
        *engine = None;
        set_stage_error(stage, None);
        return 0;
    }
    if Instant::now() < *retry_at { return 0; }
    let result = (|| {
        checkpoint(stage)?;
        if engine.is_none() {
            *engine = Some(load()?);
            CUDA_AVAILABLE.store(true, std::sync::atomic::Ordering::Release);
        }
        checkpoint(stage)?;
        run(engine.as_mut().unwrap())
    })();
    set_current(None);
    match result {
        Ok(count) => { set_stage_error(stage, None); count }
        Err(error) => {
            *engine = None;
            if stage_enabled(stage) {
                set_stage_error(stage, Some(format!("{} paused (GPU-only): {}", match stage {
                    Stage::Scenes => "Scene detection", Stage::Objects => "Object detection", Stage::Ocr => "OCR",
                }, error)));
                *retry_at = Instant::now() + Duration::from_secs(60);
            }
            0
        }
    }
}

pub fn start_ai_worker(app: AppHandle) {
    if !background_enabled() || WORKER_STARTED.swap(true, std::sync::atomic::Ordering::AcqRel) { return; }
    thread::spawn(move || {
        lower_worker_priority();
        IN_WORKER.with(|worker| worker.set(true));
        let resource_dir = app.path().resource_dir().unwrap_or_else(|_| PathBuf::from("."));
        let models_dir = resource_dir.join("models");
        let mut ocr = None;
        let mut objects = None;
        let mut scenes = None;
        let mut retry = [Instant::now(); 3];
        let mut epoch = CONFIG_EPOCH.load(std::sync::atomic::Ordering::Acquire);
        loop {
            let updated = CONFIG_EPOCH.load(std::sync::atomic::Ordering::Acquire);
            if updated != epoch { ocr = None; objects = None; scenes = None; retry = [Instant::now(); 3]; epoch = updated; }
            if !background_enabled() {
                ocr = None; objects = None; scenes = None;
                set_current(None);
                thread::sleep(Duration::from_millis(200));
                continue;
            }
            let mut processed = 0;
            processed += run_stage(Stage::Scenes, &mut scenes, &mut retry[0],
                || PREPARED.lock().unwrap().0.take().map(Ok).unwrap_or_else(|| SceneEngine::load(&crate::t_models::resolve("search", &models_dir))),
                |engine| { let conn = t_sqlite::open_conn()?; scenes_pending(&conn, engine, WORKER_BATCH) });
            processed += run_stage(Stage::Objects, &mut objects, &mut retry[1],
                || PREPARED.lock().unwrap().1.take().map(Ok).unwrap_or_else(|| ObjectEngine::load(&crate::t_models::resolve("objects", &models_dir))),
                |engine| { let conn = t_sqlite::open_conn()?; objects_pending(&conn, engine, WORKER_BATCH) });
            processed += run_stage(Stage::Ocr, &mut ocr, &mut retry[2],
                || PREPARED.lock().unwrap().2.take().map(Ok).unwrap_or_else(|| OcrEngine::load(&crate::t_models::resolve("ocr", &models_dir))),
                |engine| { let conn = t_sqlite::open_conn()?; ocr_pending(&conn, engine, WORKER_BATCH, |_, _| {}) });
            if processed == 0 { worker_sleep(Duration::from_secs(2)); }
        }
    });
}

/// Populate searchable COCO object names for images that have not been scanned.
pub fn objects_pending(conn: &Connection, engine: &mut ObjectEngine, limit: usize) -> Result<usize, String> {
    if !stage_enabled(Stage::Objects) { return Ok(0); }
    let mut stmt = conn.prepare("SELECT a.id, b.path, a.name FROM afiles a JOIN afolders b ON a.folder_id = b.id WHERE a.ai_objects IS NULL AND COALESCE(b.ai_excluded, 0) = 0 AND a.file_type IN (1, 2, 3) LIMIT ?1")
        .map_err(|e| e.to_string())?;
    let files: Vec<(i64, String, String)> = stmt.query_map(params![limit as i64], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|e| e.to_string())?.filter_map(Result::ok).collect();
    let mut processed = 0;
    for (id, folder, name) in &files {
        checkpoint(Stage::Objects)?;
        if !stage_enabled(Stage::Objects) { break; }
        let path = Path::new(folder).join(name);
        set_current(Some((*id, path.to_string_lossy().into_owned())));
        let labels = engine.detect(&path.to_string_lossy())?;
        if !stage_enabled(Stage::Objects) { break; }
        conn.execute("UPDATE afiles SET ai_objects = ?1 WHERE id = ?2", params![labels.join(", "), id])
            .map_err(|e| e.to_string())?;
        processed += 1;
        set_current(None);
        worker_sleep(Duration::from_millis(250));
    }
    Ok(processed)
}

const SCENES: &[&str] = &[
    "a beach",
    "the sea from a boat",
    "a swimming pool",
    "a mountain landscape",
    "a snowy landscape",
    "skiing",
    "a lake",
    "a forest",
    "a garden",
    "a park",
    "a field in the countryside",
    "a desert",
    "a waterfall",
    "a sunset",
    "a city street",
    "a city skyline",
    "an old town square",
    "a church interior",
    "a castle",
    "a museum",
    "an airport",
    "a train",
    "a car interior",
    "a restaurant",
    "a bar or nightclub",
    "a kitchen",
    "a living room",
    "a bedroom",
    "an office",
    "schoolwork or a classroom",
    "a playground",
    "a zoo",
    "an aquarium",
    "a concert",
    "a wedding",
    "a birthday party with a cake",
    "a christmas tree",
    "a sports stadium",
    "a golf course",
    "a tennis court",
    "a football match",
    "a gym",
    "underwater",
    "fireworks",
    "a hospital",
    "a shop or supermarket",
    "a farm with animals",
    "a dog",
    "a cat",
    "food on a plate",
    "a document or screenshot",
    "lego bricks",
];
const PLACES: &[(&str, &str)] = &[
    ("Prague", "Prague"),
    ("Prague Castle", "Prague"),
    ("Charles Bridge in Prague", "Prague"),
    ("Brno", "Brno"),
    ("Vienna", "Vienna"),
    ("Budapest", "Budapest"),
    ("Berlin", "Berlin"),
    ("the Brandenburg Gate in Berlin", "Berlin"),
    ("Munich", "Munich"),
    ("Paris", "Paris"),
    ("the Eiffel Tower in Paris", "Paris"),
    ("London", "London"),
    ("Big Ben and the Houses of Parliament in London", "London"),
    ("Tower Bridge in London", "London"),
    ("Canary Wharf in London", "London"),
    ("Edinburgh", "Edinburgh"),
    ("Rome", "Rome"),
    ("the Colosseum in Rome", "Rome"),
    ("Venice", "Venice"),
    ("Florence", "Florence"),
    ("Barcelona", "Barcelona"),
    ("Madrid", "Madrid"),
    ("Lisbon", "Lisbon"),
    ("Amsterdam", "Amsterdam"),
    ("the Swiss Alps", "Alps"),
    ("Greece", "Greece"),
    ("Croatia", "Croatia"),
    ("New York City", "New York"),
    ("the Statue of Liberty", "New York"),
    ("San Francisco", "San Francisco"),
    ("the Golden Gate Bridge", "San Francisco"),
    ("Las Vegas", "Las Vegas"),
    ("Los Angeles", "Los Angeles"),
    ("Mexico", "Mexico"),
    ("Mayan ruins in Mexico", "Mexico"),
    ("Thailand", "Thailand"),
    ("a Thai temple", "Thailand"),
    ("Tokyo", "Tokyo"),
    ("Hong Kong", "Hong Kong"),
    ("Singapore", "Singapore"),
    ("Dubai", "Dubai"),
    ("Egypt", "Egypt"),
    ("the pyramids of Giza", "Egypt"),
    ("Disneyland", "Disneyland"),
];
const OTHER: &[&str] = &[
    "a photo",
    "a snapshot of people",
    "a close-up portrait of a person",
    "a group of people indoors",
    "a blurry photo",
    "a photo of a room",
    "a photo taken at home",
    "a grand old building",
    "a stately home",
    "a river in a town",
    "old stone ruins",
];

const SCENE_MIN_PROB: f32 = 0.55;
const SCENE_MIN_SIM: f32 = 0.24;
const PLACE_MIN_PROB: f32 = 0.70;
const PLACE_MIN_SIM: f32 = 0.26;
const LOGIT_SCALE: f32 = 100.0;
const EMBEDDING_DIM: usize = 512;

fn normalize(mut values: Vec<f32>) -> Result<Vec<f32>, String> {
    if values.len() != EMBEDDING_DIM || values.iter().any(|v| !v.is_finite()) {
        return Err("Invalid CLIP embedding: expected 512 finite floats".into());
    }
    let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON { return Err("Invalid zero CLIP embedding".into()); }
    for value in &mut values { *value /= norm; }
    Ok(values)
}

fn decode_embedding(blob: &[u8]) -> Result<Vec<f32>, String> {
    if blob.len() != EMBEDDING_DIM * 4 { return Err("Invalid CLIP embedding byte length".into()); }
    normalize(blob.chunks_exact(4).map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap())).collect())
}

/// Decode a stored CLIP image embedding without opening the original photo.
pub fn decode_clip_embedding(blob: &[u8]) -> Result<Vec<f32>, String> {
    decode_embedding(blob)
}

/// Foreground CLI query encoding uses the shared acceleration policy.
pub fn encode_clip_text_query(models_dir: &Path, query: &str) -> Result<Vec<f32>, String> {
    encode_clip_text_query_profiled(models_dir, query).map(|(vector,_)| vector)
}

pub fn encode_clip_text_query_profiled(models_dir: &Path, query: &str) -> Result<(Vec<f32>, f64), String> {
    encode_clip_text_query_cancellable(models_dir, query, &|| false)
}

pub fn encode_clip_text_query_cancellable(models_dir: &Path, query: &str, cancelled: &impl Fn() -> bool) -> Result<(Vec<f32>, f64), String> {
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    struct Encoder {
        path: std::path::PathBuf,
        used: Instant,
        tokenizer: tokenizers::Tokenizer,
        session: crate::t_gpu::GpuSession,
    }
    static ENCODER: OnceLock<Mutex<Option<Encoder>>> = OnceLock::new();
    let cache = ENCODER.get_or_init(|| {
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(60));
            if let Some(cache) = ENCODER.get() {
                if let Ok(mut entry) = cache.try_lock() {
                    if entry.as_ref().is_some_and(|e| e.used.elapsed() >= Duration::from_secs(600)) {
                        *entry = None;
                    }
                }
            }
        });
        Mutex::new(None)
    });
    let query = query.trim();
    if query.is_empty() { return Err("CLIP query must not be empty".into()); }
    let mut cache = cache.lock().map_err(|e| e.to_string())?;
    if cancelled() { return Err("Search cancelled".into()); }
    let mut load_ms = 0.0;
    if cache.as_ref().is_none_or(|e| e.path != models_dir) {
        let started = Instant::now();
        let tokenizer = tokenizers::Tokenizer::from_file(models_dir.join("tokenizer.json")).map_err(|e| e.to_string())?;
        let session = crate::t_gpu::load(&models_dir.join("text_model.onnx"), false, || ort::session::Session::builder().map_err(|e| e.to_string())?
            .with_intra_threads(1).map_err(|e| e.to_string())?
            .with_inter_threads(1).map_err(|e| e.to_string())?
            .with_intra_op_spinning(false).map_err(|e| e.to_string())?
            .with_inter_op_spinning(false).map_err(|e| e.to_string()))?;
        load_ms = started.elapsed().as_secs_f64()*1000.0;
        println!("Search text model loaded in {:.1} ms; file {} bytes; idle eviction 600s", started.elapsed().as_secs_f64()*1000.0,
            std::fs::metadata(models_dir.join("text_model.onnx")).map(|m| m.len()).unwrap_or(0));
        *cache = Some(Encoder { path: models_dir.to_owned(), used: Instant::now(), tokenizer, session });
    }
    let encoder = cache.as_mut().unwrap();
    encoder.used = Instant::now();
    let prompts = if query.to_ascii_lowercase().starts_with("a photo of ") {
        vec![query.to_string()]
    } else { vec![query.to_string(), format!("a photo of {}", query)] };
    let mut mean = vec![0.0; EMBEDDING_DIM];
    for prompt in &prompts {
        if cancelled() { return Err("Search cancelled".into()); }
        let encoded = encoder.tokenizer.encode(prompt.as_str(), true).map_err(|e| e.to_string())?;
        let ids = ndarray::Array2::from_shape_vec((1, encoded.get_ids().len()), encoded.get_ids().iter().map(|id| *id as i64).collect()).map_err(|e| e.to_string())?;
        let input = ort::value::Value::from_array(ids).map_err(|e| e.to_string())?;
        let output = encoder.session.run(ort::inputs!["input_ids" => input]).map_err(|e| e.to_string())?;
        let (_, values) = output[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        for (sum, value) in mean.iter_mut().zip(normalize(values.to_vec())?) { *sum += value / prompts.len() as f32; }
    }
    normalize(mean).map(|vector| (vector, load_ms))
}

fn classify(image: &[f32], labels: &[Vec<f32>], other: &[Vec<f32>], min_prob: f32, min_sim: f32) -> Option<usize> {
    let sims: Vec<f32> = labels.iter().chain(other).map(|row| image.iter().zip(row).map(|(a, b)| a * b).sum()).collect();
    if sims.is_empty() { return None; }
    let mut best = 0;
    for i in 1..sims.len() { if sims[i] > sims[best] { best = i; } }
    let denominator: f32 = sims.iter().map(|sim| ((sim - sims[best]) * LOGIT_SCALE).exp()).sum();
    (best < labels.len() && 1.0 / denominator >= min_prob && sims[best] >= min_sim).then_some(best)
}

fn tag_text(label: &str) -> &str {
    ["a ", "an ", "the "].iter().find_map(|prefix| label.strip_prefix(*prefix)).unwrap_or(label)
}

/// Owns only cached label vectors after CUDA text-encoder initialization.
pub struct SceneEngine {
    scenes: Vec<Vec<f32>>,
    places: Vec<Vec<f32>>,
    other: Vec<Vec<f32>>,
}

impl SceneEngine {
    pub fn load(models_dir: &Path) -> Result<Self, String> {
        #[cfg(not(feature = "gpu-providers"))]
        { let _ = models_dir; return Err("GPU providers are not included in this build".into()); }
        #[cfg(feature = "gpu-providers")]
        {
            checkpoint(Stage::Scenes)?;
            let mut session = crate::t_gpu::load(&models_dir.join("text_model.onnx"), true, || ort::session::Session::builder().map_err(|e| e.to_string())?
                .with_intra_threads(1).map_err(|e| e.to_string())?
                .with_inter_threads(1).map_err(|e| e.to_string())?
                .with_parallel_execution(false).map_err(|e| e.to_string())?
                .with_intra_op_spinning(false).map_err(|e| e.to_string())?
                .with_inter_op_spinning(false).map_err(|e| e.to_string()))?;
            let tokenizer = tokenizers::Tokenizer::from_file(models_dir.join("tokenizer.json")).map_err(|e| e.to_string())?;
            let mut matrix = |labels: &[&str], templates: &[&str]| -> Result<Vec<Vec<f32>>, String> {
                labels.iter().map(|label| {
                    let mut mean = vec![0.0f32; EMBEDDING_DIM];
                    for template in templates {
                        checkpoint(Stage::Scenes)?;
                        let text = template.replace("{}", label);
                        let encoded = tokenizer.encode(text, true).map_err(|e| e.to_string())?;
                        let ids = ndarray::Array2::from_shape_vec((1, encoded.get_ids().len()), encoded.get_ids().iter().map(|id| *id as i64).collect()).map_err(|e| e.to_string())?;
                        let input = ort::value::Value::from_array(ids).map_err(|e| e.to_string())?;
                        let output = session.run(ort::inputs!["input_ids" => input]).map_err(|e| e.to_string())?;
                        let (_, values) = output[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
                        let normalized = normalize(values.to_vec())?;
                        for (sum, value) in mean.iter_mut().zip(normalized) { *sum += value / templates.len() as f32; }
                    }
                    normalize(mean)
                }).collect()
            };
            let scenes = matrix(SCENES, &["a photo of {}", "a picture of {}"])?;
            let places = matrix(&PLACES.iter().map(|(description, _)| *description).collect::<Vec<_>>(), &["a photo taken in {}", "a travel photo of {}"])?;
            let other = matrix(OTHER, &["{}"])?;
            Ok(Self { scenes, places, other })
        }
    }

    fn tags(&self, blob: &[u8]) -> Result<String, String> {
        let image = decode_embedding(blob)?;
        let mut tags = Vec::new();
        if let Some(index) = classify(&image, &self.scenes, &self.other, SCENE_MIN_PROB, SCENE_MIN_SIM) {
            tags.push(format!("scene: {}", tag_text(SCENES[index])));
        }
        if let Some(index) = classify(&image, &self.places, &self.other, PLACE_MIN_PROB, PLACE_MIN_SIM) {
            tags.push(format!("place: {}", PLACES[index].1));
        }
        Ok(tags.join(", "))
    }
}

pub fn scenes_pending(conn: &Connection, engine: &SceneEngine, limit: usize) -> Result<usize, String> {
    process_scene_rows(conn, engine, limit, || {
        checkpoint(Stage::Scenes)?;
        Ok(stage_enabled(Stage::Scenes))
    })
}

fn process_scene_rows(conn: &Connection, engine: &SceneEngine, limit: usize, allowed: impl Fn() -> Result<bool, String>) -> Result<usize, String> {
    if !allowed()? { return Ok(0); }
    let mut stmt = conn.prepare("SELECT a.id, a.embeds FROM afiles a JOIN afolders b ON a.folder_id = b.id
        WHERE a.embeds IS NOT NULL AND a.ai_tags IS NULL AND COALESCE(b.ai_excluded, 0) = 0
        AND a.file_type IN (1, 2, 3) LIMIT ?1").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([limit as i64], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)))
        .map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    let mut processed = 0;
    for (id, blob) in rows {
        if !allowed()? { break; }
        let tags = engine.tags(&blob)?;
        if !allowed()? { break; }
        conn.execute("UPDATE afiles SET ai_tags = ?1 WHERE id = ?2 AND ai_tags IS NULL", params![tags, id]).map_err(|e| e.to_string())?;
        processed += 1;
        worker_sleep(Duration::from_millis(250));
    }
    Ok(processed)
}

pub fn ai_description(ai_tags: Option<&str>, ai_objects: Option<&str>, ocr_text: Option<&str>) -> Option<String> {
    let mut scene = None;
    let mut place = None;
    for tag in ai_tags.unwrap_or("").split(',').map(str::trim) {
        if let Some(value) = tag.strip_prefix("scene:").map(str::trim).filter(|s| !s.is_empty()) { scene = Some(value); }
        if let Some(value) = tag.strip_prefix("place:").map(str::trim).filter(|s| !s.is_empty()) { place = Some(value); }
    }
    let mut parts = Vec::new();
    match (scene, place) {
        (Some(scene), Some(place)) => parts.push(format!("{} scene in {}", scene, place)),
        (Some(scene), None) => parts.push(format!("{} scene", scene)),
        (None, Some(place)) => parts.push(format!("Photo in {}", place)),
        _ => {}
    }
    let objects = ai_objects.unwrap_or("").split(',').map(str::trim).filter(|s| !s.is_empty()).take(5).collect::<Vec<_>>();
    if !objects.is_empty() { parts.push(format!("Objects: {}", objects.join(", "))); }
    let text = ocr_text.unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
    if !text.is_empty() { parts.push(format!("Text: {}{}", text.chars().take(120).collect::<String>(), if text.chars().count() > 120 { "…" } else { "" })); }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

// YOLOv10n COCO ONNX weights: https://huggingface.co/onnx-community/yolov10n
// Derived from THU-MIG/yolov10 (AGPL-3.0).
pub struct ObjectEngine { session: crate::t_gpu::GpuSession }

impl ObjectEngine {
    pub fn load(models_dir: &Path) -> Result<Self, String> {
        Self::load_configured(models_dir, 1, true, None)
    }

    pub fn load_with_threads(models_dir: &Path, threads: usize) -> Result<Self, String> {
        Self::load_configured(models_dir, threads, true, None)
    }

    pub fn load_profiled(models_dir: &Path, threads: usize, profile_dir: &Path) -> Result<Self, String> {
        Self::load_configured(models_dir, threads, true, Some(profile_dir))
    }

    fn load_configured(models_dir: &Path, threads: usize, require_cuda: bool, profile_dir: Option<&Path>) -> Result<Self, String> {
        let path = models_dir.join("yolov10n.onnx");
        if !path.exists() { return Err(format!("missing model {}", path.display())); }
        let session = crate::t_gpu::load(&path, require_cuda, || {
        let mut builder = ort::session::Session::builder().map_err(|e| e.to_string())?
            .with_intra_threads(threads).map_err(|e| e.to_string())?
            .with_inter_threads(1).map_err(|e| e.to_string())?
            .with_parallel_execution(false).map_err(|e| e.to_string())?;
        if require_cuda {
            builder = builder.with_intra_op_spinning(false).map_err(|e| e.to_string())?
                .with_inter_op_spinning(false).map_err(|e| e.to_string())?;
        }
        if let Some(dir) = profile_dir {
            builder = builder.with_profiling(dir.join("objects")).map_err(|e| e.to_string())?;
        }
        Ok(builder)
        })?;
        Ok(Self { session })
    }

    pub fn end_profiling(&mut self) -> Result<String, String> {
        self.session.end_profiling().map_err(|e| e.to_string())
    }

    pub fn detect(&mut self, image_path: &str) -> Result<Vec<String>, String> {
        let img = open_oriented(image_path)?.to_rgb8();
        let (w, h) = img.dimensions();
        if w == 0 || h == 0 { return Ok(Vec::new()); }
        let scale = 640.0 / w.max(h) as f32;
        let rw = ((w as f32 * scale).round() as u32).max(1);
        let rh = ((h as f32 * scale).round() as u32).max(1);
        let resized = image::imageops::resize(&img, rw, rh, image::imageops::FilterType::Triangle);
        let mut canvas = RgbImage::from_pixel(640, 640, Rgb([114, 114, 114]));
        image::imageops::overlay(&mut canvas, &resized, ((640 - rw) / 2) as i64, ((640 - rh) / 2) as i64);
        let mut tensor = Array4::<f32>::zeros((1, 3, 640, 640));
        for (x, y, p) in canvas.enumerate_pixels() {
            for c in 0..3 { tensor[[0, c, y as usize, x as usize]] = p[c] as f32 / 255.0; }
        }
        let value = ort::value::Value::from_array(tensor).map_err(|e| e.to_string())?;
        checkpoint(Stage::Objects)?;
        let outputs = self.session.run(ort::inputs![value]).map_err(|e| e.to_string())?;
        let (shape, values) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        if shape.len() != 3 || shape[2] != 6 { return Err(format!("unexpected YOLO output shape: {:?}", shape)); }
        let mut labels = Vec::new();
        for detection in values.chunks_exact(6) {
            let class = detection[5] as usize;
            if detection[4] >= OBJECT_SCORE && detection[5] >= 0.0 {
                if let Some(label) = COCO_LABELS.get(class) {
                    if !labels.contains(&label.to_string()) { labels.push(label.to_string()); }
                }
            }
        }
        Ok(labels)
    }
}

/// Run OCR over up to `limit` files that have no `ocr_text` yet, writing the
/// result (including a genuine empty detection) back to the
/// database. Returns the number of files handled. `progress(done, path)` is
/// called after each file.
pub fn ocr_pending<F: FnMut(usize, &str)>(
    conn: &Connection,
    engine: &mut OcrEngine,
    limit: usize,
    mut progress: F,
) -> Result<usize, String> {
    if !stage_enabled(Stage::Ocr) { return Ok(0); }
    let query = "
        SELECT a.id, b.path, a.name
        FROM afiles a
        JOIN afolders b ON a.folder_id = b.id
        WHERE a.ocr_text IS NULL
          AND COALESCE(b.ai_excluded, 0) = 0
          AND a.file_type IN (1, 2, 3)
        LIMIT ?1
    ";

    let mut stmt = conn.prepare(query).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![limit as i64], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })
        .map_err(|e| e.to_string())?;
    let files: Vec<(i64, String, String)> = rows.filter_map(Result::ok).collect();

    let mut processed = 0;
    for (i, (id, folder_path, name)) in files.iter().enumerate() {
        checkpoint(Stage::Ocr)?;
        if !stage_enabled(Stage::Ocr) { break; }
        let full_path = Path::new(folder_path).join(name);
        let full_path = full_path.to_string_lossy();
        set_current(Some((*id, full_path.to_string())));
        let text = engine.ocr_image(&full_path)?;
        if !stage_enabled(Stage::Ocr) { break; }
        conn.execute("UPDATE afiles SET ocr_text = ?1 WHERE id = ?2", params![text, id]).map_err(|e| e.to_string())?;
        progress(i + 1, &full_path);
        processed += 1;
        set_current(None);
        worker_sleep(Duration::from_millis(250));
    }

    Ok(processed)
}

/// Default location of the OCR models: `models/` next to the executable, or
/// a `models/` directory in the working directory.
pub fn default_models_dir() -> PathBuf {
    if Path::new("models/ocr_det.onnx").exists() {
        return PathBuf::from("models");
    }
    std::env::current_exe()
        .map(|p| p.parent().unwrap().join("models"))
        .unwrap_or_else(|_| PathBuf::from("models"))
}

/// Convenience wrapper: load the models and OCR one image.
pub fn run_ocr_pipeline(image_path: &str, models_dir_opt: Option<PathBuf>) -> Result<String, String> {
    let models_dir = models_dir_opt.unwrap_or_else(|| crate::t_models::resolve("ocr", &default_models_dir()));
    let mut engine = OcrEngine::load(&models_dir)?;
    engine.ocr_image(image_path)
}

/// PaddleOCR detection + recognition sessions on the CUDA execution provider.
/// Load once and reuse: session creation is the expensive part.
pub struct OcrEngine {
    det: crate::t_gpu::GpuSession,
    cls: crate::t_gpu::GpuSession,
    rec: crate::t_gpu::GpuSession,
    vocab: Vec<char>,
}

impl OcrEngine {
    pub fn load(models_dir: &Path) -> Result<Self, String> {
        Self::load_configured(models_dir, 1, true, None)
    }

    pub fn load_with_threads(models_dir: &Path, threads: usize) -> Result<Self, String> {
        Self::load_configured(models_dir, threads, true, None)
    }

    pub fn load_profiled(models_dir: &Path, threads: usize, profile_dir: &Path) -> Result<Self, String> {
        Self::load_configured(models_dir, threads, true, Some(profile_dir))
    }

    fn load_configured(models_dir: &Path, threads: usize, require_cuda: bool, profile_dir: Option<&Path>) -> Result<Self, String> {
        let build = |file: &str| -> Result<crate::t_gpu::GpuSession, String> {
            checkpoint(Stage::Ocr)?;
            let path = models_dir.join(file);
            if !path.exists() {
                return Err(format!("missing model {}", path.display()));
            }
            crate::t_gpu::load(&path, require_cuda, || {
            let mut builder = ort::session::Session::builder()
                .map_err(|e| e.to_string())?
                .with_intra_threads(threads).map_err(|e| e.to_string())?
                .with_inter_threads(1).map_err(|e| e.to_string())?
                .with_parallel_execution(false).map_err(|e| e.to_string())?;
            if require_cuda {
                builder = builder.with_intra_op_spinning(false).map_err(|e| e.to_string())?
                    .with_inter_op_spinning(false).map_err(|e| e.to_string())?;
            }
            if let Some(dir) = profile_dir {
                builder = builder.with_profiling(dir.join(file)).map_err(|e| e.to_string())?;
            }
            Ok(builder)
            })
        };
        Ok(Self {
            det: build("ocr_det.onnx")?,
            cls: build("ocr_cls.onnx")?,
            rec: build("ocr_rec.onnx")?,
            vocab: build_vocab(),
        })
    }

    pub fn end_profiling(&mut self) -> Result<Vec<String>, String> {
        [&mut self.det, &mut self.cls, &mut self.rec].into_iter()
            .map(|session| session.end_profiling().map_err(|e| e.to_string())).collect()
    }

    pub fn ocr_image(&mut self, image_path: &str) -> Result<String, String> {
        let img = open_oriented(image_path)?;
        let (orig_w, orig_h) = img.dimensions();
        if orig_w == 0 || orig_h == 0 {
            return Ok(String::new());
        }

        checkpoint(Stage::Ocr)?;
        let boxes = self.detect(&img)?;
        if debug() {
            eprintln!("{}: {}x{}, {} text boxes", image_path, orig_w, orig_h, boxes.len());
        }

        let mut lines = Vec::new();
        for (x0, y0, x1, y1) in boxes {
            let mut crop = img.crop_imm(x0, y0, (x1 - x0).max(1), (y1 - y0).max(1));
            // Paddle get_rotate_crop_image: a tall box is vertical text, turn
            // it 90° counter-clockwise, then let the angle classifier fix 180°.
            if crop.height() as f32 >= crop.width() as f32 * 1.5 {
                crop = crop.rotate270();
            }
            checkpoint(Stage::Ocr)?;
            if self.is_upside_down(&crop)? {
                crop = crop.rotate180();
            }
            checkpoint(Stage::Ocr)?;
            if let Some(text) = self.recognise(&crop)? {
                lines.push(text);
            }
        }
        Ok(lines.join("\n"))
    }

    /// DBNet text detection. Returns boxes in original-image pixel coordinates,
    /// ordered top-to-bottom then left-to-right.
    fn detect(&mut self, img: &DynamicImage) -> Result<Vec<(u32, u32, u32, u32)>, String> {
        let (orig_w, orig_h) = img.dimensions();

        // Paddle DetResizeForTest, limit_type=max, limit_side_len=960: shrink so
        // the longest side is <= 960, then round each side to a multiple of 32.
        let ratio = (DET_MAX_SIDE as f32 / orig_w.max(orig_h) as f32).min(1.0);
        let new_w = (((orig_w as f32 * ratio) / 32.0).round() as u32 * 32).max(32);
        let new_h = (((orig_h as f32 * ratio) / 32.0).round() as u32 * 32).max(32);

        let resized = img.resize_exact(new_w, new_h, image::imageops::FilterType::Triangle).to_rgb8();

        // Paddle normalises the cv2 (BGR) image channel-wise with mean
        // [0.485, 0.456, 0.406] / std [0.229, 0.224, 0.225], so B gets 0.485.
        let mut det_tensor = Array4::<f32>::zeros((1, 3, new_h as usize, new_w as usize));
        for (x, y, p) in resized.enumerate_pixels() {
            det_tensor[[0, 0, y as usize, x as usize]] = (p[2] as f32 / 255.0 - 0.485) / 0.229;
            det_tensor[[0, 1, y as usize, x as usize]] = (p[1] as f32 / 255.0 - 0.456) / 0.224;
            det_tensor[[0, 2, y as usize, x as usize]] = (p[0] as f32 / 255.0 - 0.406) / 0.225;
        }

        let det_value = ort::value::Value::from_array(det_tensor).map_err(|e| e.to_string())?;
        let outputs = self.det.run(ort::inputs![det_value]).map_err(|e| e.to_string())?;
        let (_, prob) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;

        let w = new_w as usize;
        let h = new_h as usize;
        let binary: Vec<bool> = prob.iter().map(|&v| v > DET_THRESH).collect();

        // Connected components (8-neighbour BFS) over the binarised map.
        let mut boxes = Vec::new();
        let mut visited = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                let idx = y * w + x;
                if !binary[idx] || visited[idx] {
                    continue;
                }
                let mut q = vec![(x, y)];
                visited[idx] = true;
                let (mut min_x, mut max_x, mut min_y, mut max_y) = (x, x, y, y);
                let mut score_sum = 0.0f32;
                let mut head = 0;
                while head < q.len() {
                    let (cx, cy) = q[head];
                    head += 1;
                    min_x = min_x.min(cx);
                    max_x = max_x.max(cx);
                    min_y = min_y.min(cy);
                    max_y = max_y.max(cy);
                    score_sum += prob[cy * w + cx];
                    for &(dx, dy) in &[(-1i32, 0i32), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)] {
                        let nx = cx as i32 + dx;
                        let ny = cy as i32 + dy;
                        if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                            continue;
                        }
                        let n = ny as usize * w + nx as usize;
                        if binary[n] && !visited[n] {
                            visited[n] = true;
                            q.push((nx as usize, ny as usize));
                        }
                    }
                }
                let bw = (max_x - min_x) as f32;
                let bh = (max_y - min_y) as f32;
                let mean_score = score_sum / q.len() as f32;
                if debug() {
                    eprintln!("det component {}x{} at ({},{}) score {:.2}", bw, bh, min_x, min_y, mean_score);
                }
                if bw <= 2.0 || bh <= 2.0 || mean_score < DET_BOX_THRESH {
                    continue;
                }
                // Paddle unclip: offset the shrunk DB region outwards by
                // area * ratio / perimeter so the crop covers the glyph edges.
                let d = bw * bh * DET_UNCLIP_RATIO / (2.0 * (bw + bh));
                let sx = orig_w as f32 / new_w as f32;
                let sy = orig_h as f32 / new_h as f32;
                let x0 = ((min_x as f32 - d) * sx).max(0.0) as u32;
                let y0 = ((min_y as f32 - d) * sy).max(0.0) as u32;
                let x1 = (((max_x as f32 + d) * sx) as u32).min(orig_w);
                let y1 = (((max_y as f32 + d) * sy) as u32).min(orig_h);
                boxes.push(((x0, y0, x1, y1), min_y));
            }
        }

        // Reading order: rows of roughly one text height (in detection-map
        // pixels), left to right.
        boxes.sort_by_key(|(b, det_y)| (det_y / 15, b.0));
        Ok(boxes.into_iter().map(|(b, _)| b).collect())
    }

    /// Paddle text-angle classifier (labels 0 / 180). True when the crop is
    /// upside down with confidence above cls_thresh.
    fn is_upside_down(&mut self, crop: &DynamicImage) -> Result<bool, String> {
        let (cw, ch) = crop.dimensions();
        let resized_w = ((CLS_HEIGHT as f32 * cw as f32 / ch as f32).ceil() as u32)
            .clamp(1, CLS_WIDTH);
        let resized = crop
            .resize_exact(resized_w, CLS_HEIGHT, image::imageops::FilterType::Triangle)
            .to_rgb8();
        let mut tensor = Array4::<f32>::zeros((1, 3, CLS_HEIGHT as usize, CLS_WIDTH as usize));
        for (x, y, p) in resized.enumerate_pixels() {
            tensor[[0, 0, y as usize, x as usize]] = p[2] as f32 / 127.5 - 1.0;
            tensor[[0, 1, y as usize, x as usize]] = p[1] as f32 / 127.5 - 1.0;
            tensor[[0, 2, y as usize, x as usize]] = p[0] as f32 / 127.5 - 1.0;
        }
        let value = ort::value::Value::from_array(tensor).map_err(|e| e.to_string())?;
        let outputs = self.cls.run(ort::inputs![value]).map_err(|e| e.to_string())?;
        let (_, probs) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        Ok(probs.len() >= 2 && probs[1] > probs[0] && probs[1] > CLS_THRESH)
    }

    /// CRNN recognition of one text crop. Returns None when the mean character
    /// confidence is below Paddle's drop_score or nothing was decoded.
    fn recognise(&mut self, crop: &DynamicImage) -> Result<Option<String>, String> {
        let (cw, ch) = crop.dimensions();
        let resized_w = ((REC_HEIGHT as f32 * cw as f32 / ch as f32).ceil() as u32).max(1);
        let tensor_w = resized_w.max(REC_MIN_WIDTH);
        let resized = crop
            .resize_exact(resized_w, REC_HEIGHT, image::imageops::FilterType::Triangle)
            .to_rgb8();

        // Normalise to [-1, 1], BGR order, zero-pad on the right (Paddle pads
        // after normalisation, so the pad value is 0).
        let mut rec_tensor = Array4::<f32>::zeros((1, 3, REC_HEIGHT as usize, tensor_w as usize));
        for (x, y, p) in resized.enumerate_pixels() {
            rec_tensor[[0, 0, y as usize, x as usize]] = p[2] as f32 / 127.5 - 1.0;
            rec_tensor[[0, 1, y as usize, x as usize]] = p[1] as f32 / 127.5 - 1.0;
            rec_tensor[[0, 2, y as usize, x as usize]] = p[0] as f32 / 127.5 - 1.0;
        }

        let rec_value = ort::value::Value::from_array(rec_tensor).map_err(|e| e.to_string())?;
        let outputs = self.rec.run(ort::inputs![rec_value]).map_err(|e| e.to_string())?;
        let (shape, probs) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        let seq_len = shape[1] as usize;
        let num_classes = shape[2] as usize;

        // Greedy CTC decode: argmax per step, drop blanks (class 0) and repeats.
        let mut text = String::new();
        let mut conf_sum = 0.0f32;
        let mut last_idx = usize::MAX;
        for t in 0..seq_len {
            let step = &probs[t * num_classes..(t + 1) * num_classes];
            let (max_idx, max_prob) = step
                .iter()
                .enumerate()
                .fold((0, f32::MIN), |acc, (i, &p)| if p > acc.1 { (i, p) } else { acc });
            if max_idx != 0 && max_idx != last_idx {
                if let Some(&c) = self.vocab.get(max_idx - 1) {
                    text.push(c);
                    conf_sum += max_prob;
                }
            }
            last_idx = max_idx;
        }

        let n = text.chars().count();
        let mean_conf = if n == 0 { 0.0 } else { conf_sum / n as f32 };
        if debug() {
            eprintln!("rec {}x{} -> {:?} conf {:.2}", cw, ch, text, mean_conf);
        }
        if n == 0 || mean_conf < REC_DROP_SCORE {
            return Ok(None);
        }
        Ok(Some(text.trim().to_string()).filter(|s| !s.is_empty()))
    }
}

/// Open an image and apply its EXIF orientation, so phone photos stored
/// sideways are OCR'd the way they are displayed.
fn open_oriented(path: &str) -> Result<DynamicImage, String> {
    let mut decoder = ImageReader::open(path)
        .map_err(|_| "Failed to open image".to_string())?
        .with_guessed_format()
        .map_err(|_| "Failed to open image".to_string())?
        .into_decoder()
        .map_err(|_| "Failed to decode image".to_string())?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(|_| "Failed to decode image".to_string())?;
    img.apply_orientation(orientation);
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basis(index: usize) -> Vec<f32> {
        let mut vector = vec![0.0; EMBEDDING_DIM]; vector[index] = 1.0; vector
    }
    fn blob(vector: &[f32]) -> Vec<u8> { vector.iter().flat_map(|value| value.to_le_bytes()).collect() }
    fn engine() -> SceneEngine {
        SceneEngine { scenes: vec![basis(0)], places: vec![basis(0)], other: vec![basis(1)] }
    }

    #[test]
    fn every_stage_requires_both_global_and_specific_opt_in() {
        let mut config = crate::t_config::AppConfig::default();
        config.process_library_in_background = true;
        for stage in [Stage::Scenes, Stage::Objects, Stage::Ocr] { assert!(!opted_in(&config, true, stage)); }
        config.ai_description_enabled = true; // On-demand summaries never opt in a scan.
        for stage in [Stage::Scenes, Stage::Objects, Stage::Ocr] { assert!(!opted_in(&config, true, stage)); }
        config.ai_scene_detection_enabled = true;
        assert!(opted_in(&config, true, Stage::Scenes));
        assert!(!opted_in(&config, false, Stage::Scenes));
        assert!(!opted_in(&config, true, Stage::Objects));
        config.ai_object_detection_enabled = true;
        config.ai_ocr_enabled = true;
        for stage in [Stage::Scenes, Stage::Objects, Stage::Ocr] {
            assert!(opted_in(&config, true, stage));
            assert!(!opted_in(&config, false, stage));
        }
        config.process_library_in_background = false;
        for stage in [Stage::Scenes, Stage::Objects, Stage::Ocr] { assert!(!opted_in(&config, true, stage)); }
    }

    #[test]
    fn embeddings_are_little_endian_normalized_and_validated() {
        let mut vector = basis(0); vector[0] = 3.0; vector[1] = 4.0;
        let decoded = decode_embedding(&blob(&vector)).unwrap();
        assert!((decoded[0] - 0.6).abs() < 1e-6 && (decoded[1] - 0.8).abs() < 1e-6);
        assert!(decode_embedding(&[0; 7]).is_err());
        assert!(decode_embedding(&vec![0; 2048]).is_err());
        vector[0] = f32::NAN; assert!(decode_embedding(&blob(&vector)).is_err());
        vector[0] = f32::INFINITY; assert!(decode_embedding(&blob(&vector)).is_err());
    }

    #[test]
    fn classifier_requires_probability_similarity_and_beating_other() {
        let image = basis(0);
        assert_eq!(classify(&image, &[basis(0)], &[basis(1)], 0.55, 0.24), Some(0));
        assert_eq!(classify(&image, &[basis(1)], &[basis(0)], 0.55, 0.24), None);
        assert_eq!(classify(&image, &[basis(0), basis(0)], &[basis(1)], 0.55, 0.24), None);
        let mut weak = basis(1); weak[0] = 0.20;
        assert_eq!(classify(&image, &[weak], &[basis(2)], 0.55, 0.24), None);
        let mut label = basis(1); label[0] = 0.50;
        let mut other = basis(1); other[0] = 0.494; // softmax probability about 0.646
        assert_eq!(classify(&image, &[label.clone()], &[other.clone()], SCENE_MIN_PROB, SCENE_MIN_SIM), Some(0));
        assert_eq!(classify(&image, &[label], &[other], PLACE_MIN_PROB, PLACE_MIN_SIM), None);
        assert_eq!(engine().tags(&blob(&basis(0))).unwrap(), "scene: beach, place: Prague");
        assert_eq!(engine().tags(&blob(&basis(1))).unwrap(), "");
        // Exact label inventory from lapcasa-scene-tags.py, including city aliases.
        assert_eq!((SCENES.len(), PLACES.len(), OTHER.len()), (52, 44, 11));
        assert_eq!(PLACES[2], ("Charles Bridge in Prague", "Prague"));
        assert_eq!(tag_text("an airport"), "airport");
    }

    #[test]
    fn scene_database_scan_preserves_exclusions_existing_results_and_no_matches() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE afolders(id INTEGER PRIMARY KEY, ai_excluded INTEGER);
            CREATE TABLE afiles(id INTEGER PRIMARY KEY, folder_id INTEGER, file_type INTEGER, embeds BLOB, ai_tags TEXT);
            INSERT INTO afolders VALUES(1,0),(2,1);
            INSERT INTO afiles VALUES(1,1,1,NULL,NULL),(2,1,2,NULL,NULL),(3,2,1,NULL,NULL),
                (4,1,4,NULL,NULL),(5,1,1,NULL,'existing'),(6,1,3,NULL,NULL),(7,1,1,NULL,NULL);").unwrap();
        for id in 1..=6 { conn.execute("UPDATE afiles SET embeds=?1 WHERE id=?2", params![blob(&basis(if id == 2 {1} else {0})), id]).unwrap(); }
        assert_eq!(process_scene_rows(&conn, &engine(), 10, || Ok(false)).unwrap(), 0);
        assert_eq!(process_scene_rows(&conn, &engine(), 10, || Ok(true)).unwrap(), 3);
        let tags = |id| conn.query_row("SELECT ai_tags FROM afiles WHERE id=?1", [id], |row| row.get::<_, Option<String>>(0)).unwrap();
        assert_eq!(tags(1).as_deref(), Some("scene: beach, place: Prague"));
        assert_eq!(tags(2).as_deref(), Some(""));
        assert_eq!(tags(3), None); assert_eq!(tags(4), None);
        assert_eq!(tags(5).as_deref(), Some("existing"));
        assert_eq!(tags(6).as_deref(), Some("scene: beach, place: Prague"));
        assert_eq!(tags(7), None);
        assert_eq!(process_scene_rows(&conn, &engine(), 10, || Ok(true)).unwrap(), 0);
        conn.execute("UPDATE afiles SET ai_tags=NULL, embeds=x'010203' WHERE id=1", []).unwrap();
        assert!(process_scene_rows(&conn, &engine(), 10, || Ok(true)).is_err());
        assert_eq!(tags(1), None); // Invalid data/errors never become successful empty scans.
    }

    #[test]
    fn description_uses_existing_results_and_bounds_text() {
        assert_eq!(ai_description(None, Some(" , "), Some(" \n ")), None);
        assert_eq!(ai_description(Some("scene: beach, place: Prague"), Some("person, boat"), Some("Hello\nworld")).as_deref(),
            Some("beach scene in Prague · Objects: person, boat · Text: Hello world"));
        let description = ai_description(None, None, Some(&"á".repeat(200))).unwrap();
        assert!(description.ends_with('…')); assert_eq!(description.chars().count(), 127);
    }
}
