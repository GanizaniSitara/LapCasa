//! Content-aware selection for effect masks (LAP-103): the photo offers its own regions.
//!
//! One small promptable segmentation model (SAM2 hiera-tiny, Apache-2.0, two ONNX files in the
//! app's models folder) does all of it. The photo's editor preview is encoded once, when mask mode
//! opens, and kept; every selection is then a decoder call on that encoding (milliseconds on CUDA,
//! tens of milliseconds on the CPU):
//!   * click to select: positive and negative points the user clicks;
//!   * Sky: positive seeds where the top of the photo looks like sky, faces as negatives;
//!   * Subject / People: the library's face boxes, a point on each face and one on its torso;
//!   * Background: the Subject mask, applied inverted (a flag on the saved mask, see t_edit_mask).
//! The result is a soft 8-bit mask (the model's logits through a sigmoid, so hair and foliage get
//! a real transition, not a polygon) in the preview's frame, which `t_edit_mask::save` stores at
//! the photo's size. Without the model files or a GPU the app still runs: `status` says why the
//! buttons are off, and everything falls back to the CPU execution provider.
use image::{DynamicImage, GrayImage, RgbImage};
use ndarray::{Array1, Array2, Array3, Array4};
use ort::{inputs, value::Value};
use crate::t_gpu::GpuSession as Session;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

pub const ENCODER_FILE: &str = "sam2_tiny_encoder.onnx";
pub const DECODER_FILE: &str = "sam2_tiny_decoder.onnx";
const SIDE: u32 = 1024;      // the encoder's square input; the photo sits top-left, longest edge SIDE
const LOGIT_SIDE: usize = 256; // the decoder's mask grid over that square
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Point {
    /// 0..1 of the preview's width and height.
    pub x: f32,
    pub y: f32,
    /// 1 = inside the selection, 0 = outside.
    pub label: u8,
}

#[derive(Debug, Serialize)]
pub struct Selection {
    /// 8-bit grey PNG, white = selected, at `width` x `height` (the encoder's frame, longest edge 1024).
    pub png: String,
    pub width: u32,
    pub height: u32,
    pub iou: f32,
    pub ms: u128,
    pub points: Vec<Point>,
}

struct Engine {
    encoder: Session,
    decoder: Session,
    cuda: bool,
    /// Why CUDA was not used (the CPU path then), for the status report.
    cuda_error: Option<String>,
}

struct Encoded {
    file_id: i64,
    /// Preview size the points refer to, and the encoder-frame size of the photo inside the square.
    preview: (u32, u32),
    frame: (u32, u32),
    preview_rgb: RgbImage,
    embeddings: Array4<f32>,
    high_res_0: Array4<f32>,
    high_res_1: Array4<f32>,
}

static ENGINE: Mutex<Option<Result<Engine, String>>> = Mutex::new(None);

/// A newly installed model can recover a previous missing-model result lazily.
pub fn models_installed() {
    if let Ok(mut engine) = ENGINE.lock() {
        if engine.as_ref().is_some_and(|result| result.is_err()) { *engine = None; }
    }
}
static ENCODED: Mutex<Option<Encoded>> = Mutex::new(None);
/// One encode at a time: a second request for the same photo waits and then finds it cached
/// (mask mode opening and a region button fire together). Never held while ENGINE or ENCODED is.
static ENCODING: Mutex<()> = Mutex::new(());

pub fn reset_and_warm(models: &Path) -> Result<(), String> {
    {
        let _encoding = ENCODING.lock().map_err(|e| e.to_string())?;
        let mut encoded = ENCODED.lock().map_err(|e| e.to_string())?;
        let mut engine = ENGINE.lock().map_err(|e| e.to_string())?;
        *encoded = None;
        *engine = None;
    }
    warm(models)?;
    if status(models)["cuda"] != true { return Err("Selection could not use CUDA; see AI acceleration status".into()); }
    Ok(())
}

/// Warm sessions and CUDA kernels without replacing the current photo's embedding.
pub fn warm(models: &Path) -> Result<(), String> {
    let preview = RgbImage::from_pixel(640, 480, image::Rgb([128, 128, 128]));
    let (tensor, frame) = pixel_values(&preview);
    with_engine(models, |engine| {
        let value = Value::from_array(tensor).map_err(|e| e.to_string())?;
        let outputs = engine.encoder.run(inputs!["pixel_values" => value]).map_err(|e| e.to_string())?;
        let enc = Encoded { file_id: -104, preview: (640, 480), frame, preview_rgb: preview,
            embeddings: owned4(&outputs, "image_embeddings")?, high_res_0: owned4(&outputs, "high_res_feats_0")?, high_res_1: owned4(&outputs, "high_res_feats_1")? };
        drop(outputs);
        let points = [Point { x: 0.5, y: 0.5, label: 1 }];
        let first = decode(engine, &enc, &points, None)?;
        decode(engine, &enc, &points, Some(&first.logits))?;
        Ok(())
    })
}

fn model_files(models_dir: &Path) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let (e, d) = (models_dir.join(ENCODER_FILE), models_dir.join(DECODER_FILE));
    let missing: Vec<String> = [&e, &d].iter().filter(|p| !p.exists()).map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    if missing.is_empty() { Ok((e, d)) } else {
        Err(format!("Selection model not installed: {} missing from {} (scripts/download_models adds it)", missing.join(" and "), models_dir.display()))
    }
}

/// Whether content selection can work here, and why not if it cannot (the buttons' tooltip).
pub fn status(models_dir: &Path) -> Json {
    match model_files(models_dir) {
        Err(reason) => json!({"available": false, "reason": reason, "cuda": false}),
        Ok(_) => {
            let loaded = ENGINE.lock().map(|g| g.as_ref().map(|r| r.as_ref().map(|e| (e.cuda, e.cuda_error.clone())).map_err(|e| e.clone()))).unwrap_or(None);
            match loaded {
                Some(Err(reason)) => json!({"available": false, "reason": reason, "cuda": false}),
                Some(Ok((cuda, cuda_error))) => json!({"available": true, "reason": "", "cuda": cuda, "loaded": true, "cuda_error": cuda_error}),
                None => json!({"available": true, "reason": "", "cuda": false, "loaded": false}),
            }
        }
    }
}

fn build_session(path: &Path, threads: usize, cuda: bool) -> Result<Session, String> {
    crate::t_gpu::load(path, cuda, || ort::session::Session::builder().map_err(|e| e.to_string())?
        .with_intra_threads(threads).map_err(|e| e.to_string())?
        .with_inter_threads(1).map_err(|e| e.to_string())?
        .with_parallel_execution(false).map_err(|e| e.to_string()))
}

impl Engine {
    /// CUDA when the runtime can use it, the CPU otherwise (the first click then takes about two
    /// seconds for the encoder instead of a tenth of one).
    fn load(models_dir: &Path) -> Result<Self, String> {
        let (enc, dec) = model_files(models_dir)?;
        let threads = if crate::t_gpu::hardware().cuda { 1 } else {
            std::thread::available_parallelism().map(|n| (n.get() / 2).clamp(1, 8)).unwrap_or(2)
        };
        // LAP_SELECT_CPU=1 forces the CPU path (the acceptance run times both).
        let encoder = build_session(&enc, threads, false)?;
        let decoder = build_session(&dec, threads, false)?;
        let reports = [crate::t_gpu::engine(ENCODER_FILE), crate::t_gpu::engine(DECODER_FILE)];
        let cuda = reports.iter().all(|r| r.as_ref().is_some_and(|s| s.provider == "cuda"));
        let cuda_error = reports.into_iter().flatten().find(|s| s.provider != "cuda").map(|s| s.reason);
        Ok(Self { encoder, decoder, cuda, cuda_error })
    }
}

fn with_engine<T>(models_dir: &Path, f: impl FnOnce(&mut Engine) -> Result<T, String>) -> Result<T, String> {
    let mut guard = ENGINE.lock().map_err(|e| e.to_string())?;
    if guard.is_none() { *guard = Some(Engine::load(models_dir)); }
    match guard.as_mut().unwrap() {
        Ok(engine) => f(engine),
        Err(e) => Err(e.clone()),
    }
}

/// The encoder's frame for a preview: longest edge SIDE, top-left of the square.
fn frame_of(w: u32, h: u32) -> (u32, u32, f32) {
    let scale = SIDE as f32 / w.max(h) as f32;
    (((w as f32 * scale).round() as u32).clamp(1, SIDE), ((h as f32 * scale).round() as u32).clamp(1, SIDE), scale)
}

fn pixel_values(preview: &RgbImage) -> (Array4<f32>, (u32, u32)) {
    let (w, h) = preview.dimensions();
    let (fw, fh, _) = frame_of(w, h);
    let small = image::imageops::resize(preview, fw, fh, image::imageops::FilterType::Triangle);
    let mut t = Array4::<f32>::zeros((1, 3, SIDE as usize, SIDE as usize));
    for (x, y, p) in small.enumerate_pixels() {
        for c in 0..3 { t[[0, c, y as usize, x as usize]] = (p[c] as f32 / 255.0 - MEAN[c]) / STD[c]; }
    }
    (t, (fw, fh))
}

fn owned4(outputs: &ort::session::SessionOutputs, name: &str) -> Result<Array4<f32>, String> {
    let (shape, data) = outputs[name].try_extract_tensor::<f32>().map_err(|e| format!("{name}: {e}"))?;
    if shape.len() != 4 { return Err(format!("{name}: unexpected shape {shape:?}")); }
    let dims = (shape[0] as usize, shape[1] as usize, shape[2] as usize, shape[3] as usize);
    Array4::from_shape_vec(dims, data.to_vec()).map_err(|e| e.to_string())
}

/// Encode `file_id`'s preview (oriented, as the editor shows it) once; later calls for the same
/// photo return at once. Returns the preview size and whether CUDA did the work.
pub fn prepare(models_dir: &Path, file_id: i64, preview_jpeg: &[u8]) -> Result<Json, String> {
    let _one_at_a_time = ENCODING.lock().map_err(|e| e.to_string())?;
    if let Some(enc) = ENCODED.lock().map_err(|e| e.to_string())?.as_ref() {
        if enc.file_id == file_id {
            return Ok(json!({"width": enc.preview.0, "height": enc.preview.1, "cached": true}));
        }
    }
    let preview = image::load_from_memory(preview_jpeg).map_err(|e| format!("Preview did not decode: {e}"))?.to_rgb8();
    let (w, h) = preview.dimensions();
    if w == 0 || h == 0 { return Err("Empty preview".into()); }
    let started = Instant::now();
    let (tensor, frame) = pixel_values(&preview);
    let (embeddings, high_res_0, high_res_1, cuda) = with_engine(models_dir, |engine| {
        let value = Value::from_array(tensor).map_err(|e| e.to_string())?;
        let outputs = engine.encoder.run(inputs!["pixel_values" => value]).map_err(|e| format!("encoder: {e}"))?;
        Ok((owned4(&outputs, "image_embeddings")?, owned4(&outputs, "high_res_feats_0")?, owned4(&outputs, "high_res_feats_1")?, engine.cuda))
    })?;
    let ms = started.elapsed().as_millis();
    *ENCODED.lock().map_err(|e| e.to_string())? = Some(Encoded { file_id, preview: (w, h), frame, preview_rgb: preview, embeddings, high_res_0, high_res_1 });
    Ok(json!({"width": w, "height": h, "cached": false, "ms": ms, "cuda": cuda}))
}

/// Forget the encoding (the photo changed or the editor closed).
pub fn forget(file_id: Option<i64>) {
    if let Ok(mut guard) = ENCODED.lock() {
        if file_id.map_or(true, |id| guard.as_ref().map_or(false, |e| e.file_id == id)) { *guard = None; }
    }
}

struct Decoded { logits: Vec<f32>, iou: f32 }

fn decode(engine: &mut Engine, enc: &Encoded, points: &[Point], previous: Option<&[f32]>) -> Result<Decoded, String> {
    let n = points.len().max(1);
    let (_, _, scale) = frame_of(enc.preview.0, enc.preview.1);
    let mut coords: Vec<f32> = Vec::with_capacity(n * 2);
    let mut labels: Vec<f32> = Vec::with_capacity(n);
    for p in points {
        coords.push(p.x.clamp(0.0, 1.0) * enc.preview.0 as f32 * scale);
        coords.push(p.y.clamp(0.0, 1.0) * enc.preview.1 as f32 * scale);
        labels.push(if p.label == 0 { 0.0 } else { 1.0 });
    }
    if points.is_empty() { coords.extend([0.0, 0.0]); labels.push(-1.0); }
    let pts = Value::from_array(Array3::from_shape_vec((1, n, 2), coords).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let lbl = Value::from_array(Array2::from_shape_vec((1, n), labels).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let (mask_in, has_mask) = match previous {
        Some(prev) => (Array4::from_shape_vec((1, 1, LOGIT_SIDE, LOGIT_SIDE), prev.to_vec()).map_err(|e| e.to_string())?, 1.0f32),
        None => (Array4::<f32>::zeros((1, 1, LOGIT_SIDE, LOGIT_SIDE)), 0.0f32),
    };
    let mask_in = Value::from_array(mask_in).map_err(|e| e.to_string())?;
    let has = Value::from_array(Array1::<f32>::from_vec(vec![has_mask])).map_err(|e| e.to_string())?;
    let emb = Value::from_array(enc.embeddings.clone()).map_err(|e| e.to_string())?;
    let hr0 = Value::from_array(enc.high_res_0.clone()).map_err(|e| e.to_string())?;
    let hr1 = Value::from_array(enc.high_res_1.clone()).map_err(|e| e.to_string())?;
    let outputs = engine.decoder.run(inputs![
        "image_embeddings" => emb, "high_res_feats_0" => hr0, "high_res_feats_1" => hr1,
        "input_points" => pts, "input_labels" => lbl, "input_masks" => mask_in, "has_input_masks" => has,
    ]).map_err(|e| format!("decoder: {e}"))?;
    let (shape, masks) = outputs["pred_masks"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
    let (_, ious) = outputs["iou_scores"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
    let cell = LOGIT_SIDE * LOGIT_SIDE;
    if shape.len() < 3 || shape[shape.len() - 1] as usize != LOGIT_SIDE || shape[shape.len() - 2] as usize != LOGIT_SIDE {
        return Err(format!("unexpected mask shape {shape:?}"));
    }
    let candidates = masks.len() / cell;
    let best = (0..candidates.min(ious.len())).max_by(|a, b| ious[*a].total_cmp(&ious[*b])).ok_or("no mask")?;
    Ok(Decoded { logits: masks[best * cell..(best + 1) * cell].to_vec(), iou: ious[best] })
}

/// Logits on the 256 grid, sampled bilinearly over the photo's frame, through a sigmoid: 0 = out,
/// 255 = in, a real ramp where the model itself is unsure (hair, foliage, soft edges).
fn soft_mask(logits: &[f32], frame: (u32, u32)) -> GrayImage {
    let (fw, fh) = frame;
    let step = SIDE as f32 / LOGIT_SIDE as f32;
    let at = |x: i32, y: i32| logits[(y.clamp(0, LOGIT_SIDE as i32 - 1) as usize) * LOGIT_SIDE + x.clamp(0, LOGIT_SIDE as i32 - 1) as usize];
    GrayImage::from_fn(fw, fh, |x, y| {
        let u = (x as f32 + 0.5) / step - 0.5;
        let v = (y as f32 + 0.5) / step - 0.5;
        let (x0, y0) = (u.floor(), v.floor());
        let (fx, fy) = (u - x0, v - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1, y0) * fx;
        let bottom = at(x0, y0 + 1) * (1.0 - fx) + at(x0 + 1, y0 + 1) * fx;
        let l = top * (1.0 - fy) + bottom * fy;
        let v = (255.0 / (1.0 + (-l).exp())).round() as u8;
        // Below 6% or above 94% the model is sure: snap, so a face interior is solid and a far
        // background exactly untouched; the ramp between stays as it is.
        image::Luma([if v < 16 { 0 } else if v > 239 { 255 } else { v }])
    })
}

fn to_png_data_url(mask: &GrayImage) -> Result<String, String> {
    use base64::Engine as _;
    let mut png = Vec::new();
    DynamicImage::ImageLuma8(mask.clone()).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
}

fn select_with(models_dir: &Path, file_id: i64, points: Vec<Point>) -> Result<Selection, String> {
    let started = Instant::now();
    let guard = ENCODED.lock().map_err(|e| e.to_string())?;
    let enc = guard.as_ref().filter(|e| e.file_id == file_id).ok_or("Photo not prepared for selection")?;
    let (logits, iou) = with_engine(models_dir, |engine| {
        let first = decode(engine, enc, &points, None)?;
        // One refinement pass with the first mask as a prompt sharpens the boundary (the model's
        // own iterative use); kept unless it made the prediction clearly worse.
        let second = decode(engine, enc, &points, Some(&first.logits))?;
        Ok(if second.iou >= first.iou - 0.05 { (second.logits, second.iou) } else { (first.logits, first.iou) })
    })?;
    let mask = soft_mask(&logits, enc.frame);
    Ok(Selection { png: to_png_data_url(&mask)?, width: enc.frame.0, height: enc.frame.1, iou, ms: started.elapsed().as_millis(), points })
}

/// Click to select: the mask for the clicked points (label 1 inside, 0 outside).
pub fn select_points(models_dir: &Path, file_id: i64, points: Vec<Point>) -> Result<Selection, String> {
    if points.is_empty() { return Err("Click the photo to select something".into()); }
    select_with(models_dir, file_id, points)
}

/// Sky seeds: where the top of the picture is bright, flat and bluish (or near white), as
/// fractions of the picture; empty when nothing up there looks like sky.
pub fn sky_seeds(preview: &RgbImage) -> Vec<(f32, f32)> {
    let (w, h) = preview.dimensions();
    let sw = 256u32.min(w);
    let sh = ((h as f32 * sw as f32 / w as f32).round() as u32).max(1);
    let small = image::imageops::resize(preview, sw, sh, image::imageops::FilterType::Triangle);
    let mut found: Vec<(f32, f32, f32)> = Vec::new();
    for fy in [0.04f32, 0.12, 0.22] {
        for fx in [0.08f32, 0.3, 0.5, 0.7, 0.92] {
            let (cx, cy) = ((fx * sw as f32) as i32, (fy * sh as f32) as i32);
            let (mut sum, mut sq, mut n) = ([0.0f32; 3], [0.0f32; 3], 0.0f32);
            for y in (cy - 4).max(0)..=(cy + 4).min(sh as i32 - 1) {
                for x in (cx - 4).max(0)..=(cx + 4).min(sw as i32 - 1) {
                    let p = small.get_pixel(x as u32, y as u32);
                    for c in 0..3 { sum[c] += p[c] as f32; sq[c] += (p[c] as f32) * (p[c] as f32); }
                    n += 1.0;
                }
            }
            if n == 0.0 { continue; }
            let mean = [sum[0] / n, sum[1] / n, sum[2] / n];
            let bright = (mean[0] + mean[1] + mean[2]) / 3.0;
            // flat: the average per-channel standard deviation over the patch is small
            let std = (0..3).map(|c| (sq[c] / n - mean[c] * mean[c]).max(0.0).sqrt()).sum::<f32>() / 3.0;
            let blue = mean[2] - mean[0];
            if bright > 100.0 && std < 14.0 && (blue > 8.0 || (bright > 170.0 && blue.abs() < 25.0)) {
                found.push((fx, fy, bright));
            }
        }
    }
    found.sort_by(|a, b| b.2.total_cmp(&a.2));
    found.into_iter().take(4).map(|(x, y, _)| (x, y)).collect()
}

/// Prompts for the people in the picture from their face boxes (fractions of the picture): a
/// point on each face and one below it on the torso, largest faces first.
pub fn people_prompts(faces: &[[f32; 4]]) -> Vec<Point> {
    let mut faces: Vec<[f32; 4]> = faces.iter().copied().filter(|f| f[2] > 0.0 && f[3] > 0.0).collect();
    faces.sort_by(|a, b| (b[2] * b[3]).total_cmp(&(a[2] * a[3])));
    let mut out = Vec::new();
    for f in faces.iter().take(3) {
        let (cx, cy) = (f[0] + f[2] / 2.0, f[1] + f[3] / 2.0);
        out.push(Point { x: cx, y: cy, label: 1 });
        let torso = (cy + 1.6 * f[3]).min(0.95);
        if torso > cy + 0.15 * f[3] { out.push(Point { x: cx, y: torso, label: 1 }); }
    }
    out
}

/// One-click regions. `faces` are the library's face boxes in the preview's frame as fractions.
pub fn select_region(models_dir: &Path, file_id: i64, kind: &str, faces: &[[f32; 4]]) -> Result<Selection, String> {
    let points = match kind {
        "sky" => {
            let seeds = {
                let guard = ENCODED.lock().map_err(|e| e.to_string())?;
                let enc = guard.as_ref().filter(|e| e.file_id == file_id).ok_or("Photo not prepared for selection")?;
                sky_seeds(&enc.preview_rgb)
            };
            let mut points: Vec<Point> = if seeds.is_empty() {
                [0.25f32, 0.5, 0.75].iter().map(|&x| Point { x, y: 0.03, label: 1 }).collect()
            } else {
                seeds.into_iter().map(|(x, y)| Point { x, y, label: 1 }).collect()
            };
            points.extend(faces.iter().map(|f| Point { x: f[0] + f[2] / 2.0, y: f[1] + f[3] / 2.0, label: 0 }));
            points
        }
        "people" => {
            let points = people_prompts(faces);
            if points.is_empty() { return Err("No faces found in this photo; use Click to select".into()); }
            points
        }
        "subject" => {
            let mut points = people_prompts(faces);
            if points.is_empty() { points.push(Point { x: 0.5, y: 0.5, label: 1 }); }
            points
        }
        other => return Err(format!("Unknown region {other}")),
    };
    select_with(models_dir, file_id, points)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_mask_ramps_across_the_boundary_and_saturates_away_from_it() {
        // Left half strongly in, right half strongly out.
        let logits: Vec<f32> = (0..LOGIT_SIDE * LOGIT_SIDE).map(|i| if i % LOGIT_SIDE < 128 { 4.0 } else { -4.0 }).collect();
        let m = soft_mask(&logits, (1024, 768));
        assert_eq!(m.dimensions(), (1024, 768));
        assert!(m.get_pixel(100, 100)[0] >= 245);
        assert!(m.get_pixel(900, 100)[0] <= 10);
        let row: Vec<u8> = (505..520).map(|x| m.get_pixel(x, 300)[0]).collect();
        assert!(row.windows(2).all(|p| p[0] >= p[1]), "monotonic across the edge: {row:?}");
        assert!(row.iter().any(|&v| v > 20 && v < 235), "a real ramp, not a step: {row:?}");
    }

    #[test]
    fn sky_seeds_find_the_blue_top_and_skip_a_dark_one() {
        let sky = RgbImage::from_fn(400, 300, |_, y| if y < 120 { image::Rgb([120, 170, 235]) } else { image::Rgb([60, 90, 40]) });
        let seeds = sky_seeds(&sky);
        assert!(!seeds.is_empty());
        assert!(seeds.iter().all(|(_, y)| *y < 0.3));
        let night = RgbImage::from_pixel(400, 300, image::Rgb([20, 20, 30]));
        assert!(sky_seeds(&night).is_empty());
        // A white overcast sky counts too.
        let overcast = RgbImage::from_fn(400, 300, |_, y| if y < 120 { image::Rgb([230, 232, 236]) } else { image::Rgb([60, 90, 40]) });
        assert!(!sky_seeds(&overcast).is_empty());
    }

    #[test]
    fn people_prompts_put_a_point_on_each_face_and_its_torso() {
        let pts = people_prompts(&[[0.4, 0.2, 0.1, 0.15], [0.1, 0.1, 0.02, 0.03]]);
        assert_eq!(pts.len(), 4);
        assert!((pts[0].x - 0.45).abs() < 1e-5 && (pts[0].y - 0.275).abs() < 1e-5);
        assert!(pts[1].y > pts[0].y && pts[1].y <= 0.92);
        // A face at the bottom edge still gets a torso point, clamped inside the picture.
        let low = people_prompts(&[[0.4, 0.8, 0.1, 0.15]]);
        assert_eq!(low.len(), 2);
        assert!(low[1].y <= 0.95);
        assert!(people_prompts(&[]).is_empty());
    }

    #[test]
    fn status_names_the_missing_files() {
        let dir = std::env::temp_dir().join(format!("lap_select_status_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let s = status(&dir);
        assert_eq!(s["available"], false);
        assert!(s["reason"].as_str().unwrap().contains(ENCODER_FILE));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The real model on a synthetic picture: a bright disc on a dark field; a click on the disc
    /// selects it. Skipped when the model files are not in the repo's resources.
    #[test]
    fn click_selects_a_disc_with_the_real_model() {
        let models = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/models");
        if model_files(&models).is_err() { eprintln!("select model not present, skipping"); return; }
        let (w, h) = (640u32, 480u32);
        let img = RgbImage::from_fn(w, h, |x, y| {
            let d = ((x as f32 - 200.0).powi(2) + (y as f32 - 240.0).powi(2)).sqrt();
            if d < 90.0 { image::Rgb([240, 200, 60]) } else { image::Rgb([30, 40, 60]) }
        });
        let jpeg = crate::t_jpeg::encode_rgb8(&img, 92).unwrap();
        let file_id = 7_000_001;
        let info = prepare(&models, file_id, &jpeg).unwrap();
        assert_eq!(info["width"], 640);
        let sel = select_points(&models, file_id, vec![Point { x: 200.0 / 640.0, y: 0.5, label: 1 }]).unwrap();
        assert_eq!((sel.width, sel.height), (1024, 768));
        let png = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, sel.png.split_once(',').unwrap().1).unwrap();
        let m = image::load_from_memory(&png).unwrap().to_luma8();
        let s = 1024.0 / 640.0;
        assert!(m.get_pixel((200.0 * s) as u32, (240.0 * s) as u32)[0] > 200, "disc centre selected");
        assert!(m.get_pixel((500.0 * s) as u32, (240.0 * s) as u32)[0] < 40, "field not selected");
        assert!(m.get_pixel((200.0 * s) as u32, (60.0 * s) as u32)[0] < 40, "above the disc not selected");
        // Outside the disc as a negative point: the selection must not grow to the field.
        let sel2 = select_points(&models, file_id, vec![Point { x: 200.0 / 640.0, y: 0.5, label: 1 }, Point { x: 0.8, y: 0.5, label: 0 }]).unwrap();
        assert!(sel2.iou > 0.5);
        // Subject with no faces falls back to the centre; Sky on a dark field finds no seeds and still answers.
        assert!(select_region(&models, file_id, "subject", &[]).is_ok());
        assert!(select_region(&models, file_id, "people", &[]).is_err());
        forget(Some(file_id));
        assert!(select_points(&models, file_id, vec![Point { x: 0.5, y: 0.5, label: 1 }]).is_err());
    }
}
