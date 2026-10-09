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

pub fn background_enabled() -> bool {
    BACKGROUND_ENABLED.load(std::sync::atomic::Ordering::Acquire)
}

pub fn set_background_enabled(enabled: bool) {
    let enabled = enabled && !cfg!(feature = "vm-rig");
    BACKGROUND_ENABLED.store(enabled, std::sync::atomic::Ordering::Release);
    if !enabled {
        set_current(None);
    }
}

// Progress corner: the photo being processed now and what is left.
static CURRENT: std::sync::Mutex<Option<(i64, String)>> = std::sync::Mutex::new(None);
static PENDING: std::sync::Mutex<Option<(Instant, i64, i64)>> = std::sync::Mutex::new(None);

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
}

pub fn ai_progress() -> Result<AiProgress, String> {
    let (objects_pending, ocr_pending) = {
        let mut pending = PENDING.lock().map_err(|e| e.to_string())?;
        match *pending {
            Some((at, objects, ocr)) if at.elapsed() < Duration::from_secs(10) => (objects, ocr),
            _ => {
                let conn = t_sqlite::open_conn()?;
                let (objects, ocr): (i64, i64) = conn.query_row(
                    "SELECT COALESCE(SUM(a.ai_objects IS NULL), 0), COALESCE(SUM(a.ocr_text IS NULL), 0)
                     FROM afiles a JOIN afolders b ON a.folder_id = b.id
                     WHERE COALESCE(b.ai_excluded, 0) = 0 AND a.file_type IN (1, 2, 3)",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                ).map_err(|e| e.to_string())?;
                *pending = Some((Instant::now(), objects, ocr));
                (objects, ocr)
            }
        }
    };
    let current = CURRENT.lock().map_err(|e| e.to_string())?.clone();
    Ok(AiProgress {
        enabled: background_enabled(),
        file_id: current.as_ref().map(|c| c.0),
        path: current.map(|c| c.1),
        objects_pending,
        ocr_pending,
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
    while background_enabled() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(200));
    }
}

pub fn start_ai_worker(app: AppHandle) {
    if !background_enabled() || WORKER_STARTED.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return;
    }
    thread::spawn(move || {
        let resource_dir = app.path().resource_dir().unwrap_or_else(|_| PathBuf::from("."));
        let models_dir = resource_dir.join("models");
        let mut engine: Option<OcrEngine> = None;
        let mut objects: Option<ObjectEngine> = None;
        let mut next_object_attempt = Instant::now();
        loop {
            if !background_enabled() {
                engine = None;
                objects = None;
                thread::sleep(Duration::from_millis(200));
                continue;
            }
            if engine.is_none() {
                match OcrEngine::load(&models_dir) {
                    Ok(e) => engine = Some(e),
                    Err(e) => {
                        eprintln!("Advanced AI worker: OCR models unavailable ({}); retrying in 60s", e);
                        worker_sleep(Duration::from_secs(60));
                        continue;
                    }
                }
            }
            if objects.is_none() && Instant::now() >= next_object_attempt {
                match ObjectEngine::load(&models_dir) {
                    Ok(e) => objects = Some(e),
                    Err(e) => {
                        eprintln!("Advanced AI worker: object model unavailable: {}", e);
                        next_object_attempt = Instant::now() + Duration::from_secs(60);
                    }
                }
            }
            if !background_enabled() {
                continue;
            }
            let processed = match t_sqlite::open_conn() {
                Ok(conn) => (|| -> Result<usize, String> {
                    let ocr = ocr_pending(&conn, engine.as_mut().unwrap(), WORKER_BATCH, |_, _| {})?;
                    let detected = if background_enabled() {
                        if let Some(objects) = objects.as_mut() {
                            objects_pending(&conn, objects, WORKER_BATCH)?
                        } else {
                            0
                        }
                    } else {
                        0
                    };
                    Ok(ocr + detected)
                })(),
                Err(e) => Err(e),
            };
            match processed {
                Ok(0) => {
                    set_current(None);
                    worker_sleep(Duration::from_secs(10));
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("Error in advanced AI worker: {}", e);
                    worker_sleep(Duration::from_secs(10));
                }
            }
        }
    });
}

/// Populate searchable COCO object names for images that have not been scanned.
pub fn objects_pending(conn: &Connection, engine: &mut ObjectEngine, limit: usize) -> Result<usize, String> {
    let mut stmt = conn.prepare("SELECT a.id, b.path, a.name FROM afiles a JOIN afolders b ON a.folder_id = b.id WHERE a.ai_objects IS NULL AND COALESCE(b.ai_excluded, 0) = 0 AND a.file_type IN (1, 2, 3) LIMIT ?1")
        .map_err(|e| e.to_string())?;
    let files: Vec<(i64, String, String)> = stmt.query_map(params![limit as i64], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|e| e.to_string())?.filter_map(Result::ok).collect();
    for (id, folder, name) in &files {
        let path = Path::new(folder).join(name);
        set_current(Some((*id, path.to_string_lossy().into_owned())));
        let labels = engine.detect(&path.to_string_lossy()).unwrap_or_default();
        conn.execute("UPDATE afiles SET ai_objects = ?1 WHERE id = ?2", params![labels.join(", "), id])
            .map_err(|e| e.to_string())?;
    }
    Ok(files.len())
}

// YOLOv10n COCO ONNX weights: https://huggingface.co/onnx-community/yolov10n
// Derived from THU-MIG/yolov10 (AGPL-3.0).
pub struct ObjectEngine { session: ort::session::Session }

impl ObjectEngine {
    pub fn load(models_dir: &Path) -> Result<Self, String> {
        Self::load_configured(models_dir, 2, false, None)
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
        #[cfg(feature = "gpu-providers")]
        let cuda = ort::execution_providers::CUDAExecutionProvider::default().build();
        #[cfg(feature = "gpu-providers")]
        let cuda = if require_cuda { cuda.error_on_failure() } else { cuda };
        let mut builder = ort::session::Session::builder().map_err(|e| e.to_string())?
            .with_intra_threads(threads).map_err(|e| e.to_string())?
            .with_inter_threads(1).map_err(|e| e.to_string())?
            .with_parallel_execution(false).map_err(|e| e.to_string())?;
        #[cfg(feature = "gpu-providers")]
        { builder = builder.with_execution_providers([cuda]).map_err(|e| e.to_string())?; }
        #[cfg(not(feature = "gpu-providers"))]
        if require_cuda { return Err("GPU providers are not included in this build".into()); }
        if let Some(dir) = profile_dir {
            builder = builder.with_profiling(dir.join("objects")).map_err(|e| e.to_string())?;
        }
        let session = builder.commit_from_file(path).map_err(|e| e.to_string())?;
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
/// result (or "" on failure, so the file is not retried forever) back to the
/// database. Returns the number of files handled. `progress(done, path)` is
/// called after each file.
pub fn ocr_pending<F: FnMut(usize, &str)>(
    conn: &Connection,
    engine: &mut OcrEngine,
    limit: usize,
    mut progress: F,
) -> Result<usize, String> {
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

    for (i, (id, folder_path, name)) in files.iter().enumerate() {
        let full_path = Path::new(folder_path).join(name);
        let full_path = full_path.to_string_lossy();
        set_current(Some((*id, full_path.to_string())));
        let text = engine.ocr_image(&full_path).unwrap_or_default();
        let _ = conn.execute("UPDATE afiles SET ocr_text = ?1 WHERE id = ?2", params![text, id]);
        progress(i + 1, &full_path);
    }

    Ok(files.len())
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
    let models_dir = models_dir_opt.unwrap_or_else(default_models_dir);
    let mut engine = OcrEngine::load(&models_dir)?;
    engine.ocr_image(image_path)
}

/// PaddleOCR detection + recognition sessions on the CUDA execution provider.
/// Load once and reuse: session creation is the expensive part.
pub struct OcrEngine {
    det: ort::session::Session,
    cls: ort::session::Session,
    rec: ort::session::Session,
    vocab: Vec<char>,
}

impl OcrEngine {
    pub fn load(models_dir: &Path) -> Result<Self, String> {
        Self::load_configured(models_dir, 2, false, None)
    }

    pub fn load_with_threads(models_dir: &Path, threads: usize) -> Result<Self, String> {
        Self::load_configured(models_dir, threads, true, None)
    }

    pub fn load_profiled(models_dir: &Path, threads: usize, profile_dir: &Path) -> Result<Self, String> {
        Self::load_configured(models_dir, threads, true, Some(profile_dir))
    }

    fn load_configured(models_dir: &Path, threads: usize, require_cuda: bool, profile_dir: Option<&Path>) -> Result<Self, String> {
        let build = |file: &str| -> Result<ort::session::Session, String> {
            let path = models_dir.join(file);
            if !path.exists() {
                return Err(format!("missing model {}", path.display()));
            }
            #[cfg(feature = "gpu-providers")]
            let cuda = ort::execution_providers::CUDAExecutionProvider::default().build();
            #[cfg(feature = "gpu-providers")]
            let cuda = if require_cuda { cuda.error_on_failure() } else { cuda };
            let mut builder = ort::session::Session::builder()
                .map_err(|e| e.to_string())?
                .with_intra_threads(threads).map_err(|e| e.to_string())?
                .with_inter_threads(1).map_err(|e| e.to_string())?
                .with_parallel_execution(false).map_err(|e| e.to_string())?;
            #[cfg(feature = "gpu-providers")]
            { builder = builder.with_execution_providers([cuda]).map_err(|e| e.to_string())?; }
            #[cfg(not(feature = "gpu-providers"))]
            if require_cuda { return Err("GPU providers are not included in this build".into()); }
            if let Some(dir) = profile_dir {
                builder = builder.with_profiling(dir.join(file)).map_err(|e| e.to_string())?;
            }
            builder.commit_from_file(path).map_err(|e| e.to_string())
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
            if self.is_upside_down(&crop)? {
                crop = crop.rotate180();
            }
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
