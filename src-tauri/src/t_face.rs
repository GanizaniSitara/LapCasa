/**
 * Face Recognition module
 * Handles face detection (RetinaFace) and embedding (MobileFaceNet) using ONNX Runtime.
 */
use crate::{t_cluster, t_common, t_sqlite};
use crate::t_gpu::GpuSession as Session;
use image::DynamicImage;
use ndarray::Array;
use ort::{
    inputs,
    session::builder::GraphOptimizationLevel,
    value::Value,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

// cancellation token for face indexing
#[derive(Clone)]
pub struct FaceIndexCancellation(pub Arc<Mutex<bool>>);

// detailed status for face indexing
#[derive(Clone)]
pub struct FaceIndexingStatus(pub Arc<Mutex<bool>>);

// face indexing progress
#[derive(Clone, serde::Serialize)]
pub struct FaceIndexProgress {
    pub current: usize,
    pub total: usize,
    pub faces_found: usize,
    pub phase: String,
}

#[derive(Clone)]
pub struct FaceIndexProgressState(pub Arc<Mutex<FaceIndexProgress>>);

// face stats
#[derive(Clone, serde::Serialize)]
pub struct FaceStats {
    pub total: usize,
    pub processed: usize,
    pub unprocessed: usize,
    pub faces: usize,
}

/// Detected face bounding box and landmarks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaceBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub confidence: f32,
    pub landmarks: Option<Vec<(f32, f32)>>, // 5 facial landmarks
}

/// Long edge the detector sees (the size of the library's thumbnails, which the first indexing
/// ran on); the detection thresholds in t_common were set at this scale.
pub const THUMBNAIL_SCALE: u32 = 512;

/// Open a photo with its EXIF orientation applied and scale it to `long_edge` (never up).
/// Returns the small image and the oriented full size.
pub fn open_oriented_thumbnail(path: &str, long_edge: u32) -> Result<(DynamicImage, (u32, u32)), String> {
    let mut decoder = image::ImageReader::open(path)
        .map_err(|e| format!("Failed to open image: {}", e))?
        .with_guessed_format()
        .map_err(|e| format!("Failed to open image: {}", e))?
        .into_decoder()
        .map_err(|e| format!("Failed to decode image: {}", e))?;
    let orientation = image::ImageDecoder::orientation(&mut decoder).unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| format!("Failed to decode image: {}", e))?;
    img.apply_orientation(orientation);
    Ok(thumbnail_of(img, long_edge))
}

/// Scale to `long_edge` on the longer side, never up; returns the small image and the size it came from.
/// Scaled with Lanczos3 to preserve face detail when re-decoding from the original.
pub fn thumbnail_of(img: DynamicImage, long_edge: u32) -> (DynamicImage, (u32, u32)) {
    let (w, h) = (img.width(), img.height());
    if w.max(h) <= long_edge || w == 0 || h == 0 {
        return (img, (w, h));
    }
    let scale = long_edge as f32 / w.max(h) as f32;
    let nw = ((w as f32 * scale).round() as u32).max(1);
    let nh = ((h as f32 * scale).round() as u32).max(1);
    (img.resize_exact(nw, nh, image::imageops::FilterType::Lanczos3), (w, h))
}

/// Face with embedding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaceData {
    pub bbox: FaceBox,
    pub embedding: Vec<f32>,
}

struct Anchor {
    cx: f32,
    cy: f32,
}

pub struct FaceEngine {
    detection_model: Option<Session>, // RetinaFace
    embedding_model: Option<Session>, // MobileFaceNet
}

impl FaceEngine {
    pub fn new() -> Self {
        Self {
            detection_model: None,
            embedding_model: None,
        }
    }

    pub fn unload_models(&mut self) {
        self.detection_model = None;
        self.embedding_model = None;
    }

    pub fn load_models(&mut self, app: &AppHandle) -> Result<(), String> {
        if self.detection_model.is_some() {
            return Ok(());
        }

        // Resolve paths
        let resource_dir = crate::t_models::app_dir(app, "people");
        if !crate::t_models::available("people", &resource_dir) { return Err(crate::t_models::missing("people")); }
        self.load_models_from_dir(&resource_dir)
    }

    /// Load the face models from a directory (lap-cli has no AppHandle).
    pub fn load_models_from_dir(&mut self, resource_dir: &std::path::Path) -> Result<(), String> {
        if self.detection_model.is_some() {
            return Ok(());
        }
        let detection_model_path = resource_dir.join(t_common::DETECTION_MODEL);
        let embedding_model_path = resource_dir.join(t_common::EMBEDDING_MODEL);

        // Check if models exist
        if !detection_model_path.exists() {
            return Err(format!(
                "Detection model not found at {:?}",
                detection_model_path
            ));
        }
        if !embedding_model_path.exists() {
            return Err(format!(
                "Embedding model not found at {:?}",
                embedding_model_path
            ));
        }

        // Load Detection Model (RetinaFace)
        let detection_model = crate::t_gpu::load(&detection_model_path, false, || ort::session::Session::builder()
            .map_err(|e| e.to_string())?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| e.to_string())?
            .with_intra_threads(4)
            .map_err(|e| e.to_string()))
            .map_err(|e| format!("Failed to load detection model: {}", e))?;

        self.detection_model = Some(detection_model);

        // Load Embedding Model (MobileFaceNet)
        let embedding_model = crate::t_gpu::load(&embedding_model_path, false, || ort::session::Session::builder()
            .map_err(|e| e.to_string())?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| e.to_string())?
            .with_intra_threads(4)
            .map_err(|e| e.to_string()))
            .map_err(|e| format!("Failed to load embedding model: {}", e))?;

        self.embedding_model = Some(embedding_model);

        Ok(())
    }

    pub fn is_loaded(&self) -> bool {
        self.detection_model.is_some() && self.embedding_model.is_some()
    }

    /// Face boxes in `img` (pixels), for Auto Balance's whole-face rule (LAP-107).
    pub fn detect_boxes(&mut self, img: &DynamicImage) -> Result<Vec<FaceBox>, String> {
        self.detect_faces(img)
    }

    /// Detect faces implementation (from DynamicImage)
    fn detect_faces(&mut self, img: &DynamicImage) -> Result<Vec<FaceBox>, String> {
        let original_width = img.width() as f32;
        let original_height = img.height() as f32;

        // RetinaFace typically expects 640x640 input, but works with any size divisible by 32 (stride 32).
        // Optimization: For small images (like thumbnails ~512px), use their native size slightly rounded up.
        // For large images, downscale to 640px max dimension.
        let max_dim = original_width.max(original_height);
        let target_size = if max_dim < 640.0 {
            // Round up to nearest multiple of 32
            ((max_dim as u32 + 31) / 32) * 32
        } else {
            640
        };
        // Resize preserving aspect ratio (Letterbox)
        // Use max dimension to fit within target
        let scale = (target_size as f32) / original_width.max(original_height);
        // Use round() to minimize truncation error
        let new_w = (original_width * scale).round() as u32;
        let new_h = (original_height * scale).round() as u32;

        let rgb_buf; // Owned buffer if needed
        let rgb_img = if new_w == img.width() && new_h == img.height() {
            // Optimization: Skip resize if unnecessary
            if let Some(buf) = img.as_rgb8() {
                buf
            } else {
                rgb_buf = img.to_rgb8();
                &rgb_buf
            }
        } else {
            rgb_buf = img
                .resize_exact(new_w, new_h, image::imageops::FilterType::Triangle)
                .into_rgb8();
            &rgb_buf
        };

        // Standard InsightFace/RetinaFace preprocessing aligns to Top-Left (0,0)

        // Normalize: (pixel - 127.5) / 128.0
        // Initialize with zeros (padding)
        let mut array = Array::zeros((1, 3, target_size as usize, target_size as usize));

        if let Some(slice) = array.as_slice_mut() {
            let area = (target_size as usize) * (target_size as usize);
            let offset_b = 0;
            let offset_g = area;
            let offset_r = area * 2;
            let target_w = target_size as usize;

            for (x, y, pixel) in rgb_img.enumerate_pixels() {
                let r = (pixel[0] as f32 - 127.5) / 128.0;
                let g = (pixel[1] as f32 - 127.5) / 128.0;
                let b = (pixel[2] as f32 - 127.5) / 128.0;

                let idx = (y as usize) * target_w + (x as usize);

                slice[offset_b + idx] = b;
                slice[offset_g + idx] = g;
                slice[offset_r + idx] = r;
            }
        } else {
            // Fallback if array is not contiguous (should not happen with default init)
            for (x, y, pixel) in rgb_img.enumerate_pixels() {
                let r = (pixel[0] as f32 - 127.5) / 128.0;
                let g = (pixel[1] as f32 - 127.5) / 128.0;
                let b = (pixel[2] as f32 - 127.5) / 128.0;

                array[[0, 0, y as usize, x as usize]] = b; // Blue
                array[[0, 1, y as usize, x as usize]] = g; // Green
                array[[0, 2, y as usize, x as usize]] = r; // Red
            }
        }

        let input_value = Value::from_array(array).map_err(|e| e.to_string())?;

        // Use block scope to ensure outputs is dropped before calling nms
        let mut faces = {
            let outputs = self
                .detection_model
                .as_mut()
                .unwrap()
                .run(inputs!["input.1" => input_value])
                .map_err(|e| format!("Detection inference error: {}", e))?;

            let mut all_detections = Vec::new();
            let strides = [8, 16, 32];
            let min_sizes = [[16, 32], [64, 128], [256, 512]]; // Standard RetinaFace config

            // Map output indices based on observation
            // Scores, Boxes, Landmarks indices per stride
            let indices = [
                (0, 3, 6), // Stride 8
                (1, 4, 7), // Stride 16
                (2, 5, 8), // Stride 32
            ];

            let confidence_threshold = 0.6;

            for (i, &stride) in strides.iter().enumerate() {
                let (score_idx, box_idx, _) = indices[i];

                let scores_tensor = &outputs[score_idx];
                let boxes_tensor = &outputs[box_idx];

                let (_, scores_data) = scores_tensor
                    .try_extract_tensor::<f32>()
                    .map_err(|e| format!("Failed stride {} scores: {}", stride, e))?;
                let (_, boxes_data) = boxes_tensor
                    .try_extract_tensor::<f32>()
                    .map_err(|e| format!("Failed stride {} boxes: {}", stride, e))?;

                let feature_map_w = target_size / stride;
                let feature_map_h = target_size / stride;
                let anchors =
                    Self::generate_anchors(stride, &min_sizes[i], feature_map_w, feature_map_h);

                for (j, anchor) in anchors.iter().enumerate() {
                    let score = scores_data[j];
                    if score < confidence_threshold {
                        continue;
                    }

                    // Decode box: [l, t, r, b] (distances from center, normalized by stride)
                    // This assumes SCRFD model (det_10g.onnx) which outputs distances
                    let l = boxes_data[j * 4];
                    let t = boxes_data[j * 4 + 1];
                    let r = boxes_data[j * 4 + 2];
                    let b = boxes_data[j * 4 + 3];

                    // SCRFD uses stride-scaled distances
                    // x1 = cx - l * stride
                    // y1 = cy - t * stride
                    // x2 = cx + r * stride
                    // y2 = cy + b * stride

                    let x1 = anchor.cx - l * stride as f32;
                    let y1 = anchor.cy - t * stride as f32;
                    let x2 = anchor.cx + r * stride as f32;
                    let y2 = anchor.cy + b * stride as f32;

                    // Scale back to original image
                    // Use effective scale factors derived from actual resized dimensions
                    let inv_scale_x = original_width / new_w as f32;
                    let inv_scale_y = original_height / new_h as f32;

                    // Scale directly (no padding offset)
                    let original_x1 = x1 * inv_scale_x;
                    let original_y1 = y1 * inv_scale_y;
                    let original_x2 = x2 * inv_scale_x;
                    let original_y2 = y2 * inv_scale_y;

                    all_detections.push(FaceBox {
                        x: original_x1,
                        y: original_y1,
                        width: original_x2 - original_x1,
                        height: original_y2 - original_y1,
                        confidence: score,
                        landmarks: None,
                    });
                }
            }

            all_detections
        };

        // Non-maximum suppression
        faces = self.nms(faces, 0.4);

        if faces.is_empty() {
            // No faces found after NMS
        }

        Ok(faces)
    }

    /// Generate anchors for a specific stride
    fn generate_anchors(
        stride: u32,
        min_sizes: &[u32],
        feature_w: u32,
        feature_h: u32,
    ) -> Vec<Anchor> {
        let mut anchors =
            Vec::with_capacity((feature_w * feature_h * min_sizes.len() as u32) as usize);

        for y in 0..feature_h {
            for x in 0..feature_w {
                for &_min_size in min_sizes {
                    // Dense anchor centers
                    // Adjusted to 0.0 (top-left) from 0.5 (center) to fix systematic bottom-right shift
                    let cx = (x as f32) * stride as f32;
                    let cy = (y as f32) * stride as f32;

                    anchors.push(Anchor { cx, cy });
                }
            }
        }
        anchors
    }

    /// Get face embedding implementation (from DynamicImage)
    fn get_face_embedding(
        &mut self,
        img: &DynamicImage,
        bbox: &FaceBox,
    ) -> Result<Vec<f32>, String> {
        // Crop face region with some padding
        let padding = 0.2;
        let x = (bbox.x - bbox.width * padding).max(0.0) as u32;
        let y = (bbox.y - bbox.height * padding).max(0.0) as u32;
        let w = (bbox.width * (1.0 + 2.0 * padding)) as u32;
        let h = (bbox.height * (1.0 + 2.0 * padding)) as u32;

        let max_x = (x + w).min(img.width());
        let max_y = (y + h).min(img.height());

        // Optimize: check if we can reuse the crop or if we need to resize
        // MobileFaceNet expects 112x112
        let target_size = 112;
        let face_crop = img.crop_imm(x, y, max_x - x, max_y - y);
        let rgb_buf;
        let rgb_face = if face_crop.width() == target_size && face_crop.height() == target_size {
            if let Some(buf) = face_crop.as_rgb8() {
                buf
            } else {
                rgb_buf = face_crop.to_rgb8();
                &rgb_buf
            }
        } else {
            rgb_buf = face_crop
                .resize_exact(
                    target_size,
                    target_size,
                    image::imageops::FilterType::Triangle,
                )
                .into_rgb8();
            &rgb_buf
        };

        // Normalize: (pixel - 127.5) / 128.0
        let mut array = Array::zeros((1, 3, 112, 112));

        // Optimize: use slice access
        if let Some(slice) = array.as_slice_mut() {
            let area = 112 * 112;
            let offset_g = area;
            let offset_b = area * 2;
            let width = 112;

            for (x, y, pixel) in rgb_face.enumerate_pixels() {
                let r = (pixel[0] as f32 - 127.5) / 128.0;
                let g = (pixel[1] as f32 - 127.5) / 128.0;
                let b = (pixel[2] as f32 - 127.5) / 128.0;

                let idx = (y as usize) * width + (x as usize);

                slice[idx] = r;
                slice[offset_g + idx] = g;
                slice[offset_b + idx] = b;
            }
        } else {
            for (fx, fy, pixel) in rgb_face.enumerate_pixels() {
                let r = (pixel[0] as f32 - 127.5) / 128.0;
                let g = (pixel[1] as f32 - 127.5) / 128.0;
                let b = (pixel[2] as f32 - 127.5) / 128.0;

                array[[0, 0, fy as usize, fx as usize]] = r;
                array[[0, 1, fy as usize, fx as usize]] = g;
                array[[0, 2, fy as usize, fx as usize]] = b;
            }
        }

        let input_value = Value::from_array(array).map_err(|e| e.to_string())?;

        let outputs = self
            .embedding_model
            .as_mut()
            .unwrap()
            .run(inputs!["input.1" => input_value])
            .map_err(|e| format!("Embedding inference error: {}", e))?;

        let embedding = &outputs[0];
        let (_, embedding_data) = embedding
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Failed to extract embedding: {}", e))?;

        // Normalize embedding to unit vector
        let emb_vec = embedding_data.to_vec();
        let norm: f32 = emb_vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if !norm.is_finite() || norm <= f32::EPSILON {
            return Err("Invalid face embedding norm".to_string());
        }
        let normalized: Vec<f32> = emb_vec.iter().map(|x| x / norm).collect();

        Ok(normalized)
    }

    /// Embedding of a face box (detector-style box, image pixel coords) in an image.
    /// Used for faces imported without an embedding, such as Picasa face tags.
    pub fn embed_face_box(&mut self, img: &DynamicImage, bbox: &FaceBox) -> Result<Vec<f32>, String> {
        if self.embedding_model.is_none() {
            return Err("Face embedding model not loaded".to_string());
        }
        self.get_face_embedding(img, bbox)
    }

    /// Compute cosine similarity between two embeddings
    #[allow(dead_code)]
    pub fn compare_faces(emb1: &[f32], emb2: &[f32]) -> f32 {
        if emb1.len() != emb2.len() {
            return 0.0;
        }
        // Embeddings are already normalized, so dot product = cosine similarity
        emb1.iter().zip(emb2.iter()).map(|(a, b)| a * b).sum()
    }

    /// Process image: detect all faces and get embeddings
    /// Filters out low-quality faces (low confidence, small size, blurry)
    pub fn process_image(
        &mut self,
        image_path: &str,
    ) -> Result<(Vec<FaceData>, (u32, u32)), String> {
        let img = image::open(image_path).map_err(|e| format!("Failed to open image: {}", e))?;
        self.process_dynamic_image(&img)
    }

    pub fn process_image_from_bytes(
        &mut self,
        image_bytes: &[u8],
    ) -> Result<(Vec<FaceData>, (u32, u32)), String> {
        let img = image::load_from_memory(image_bytes)
            .map_err(|e| format!("Failed to load image from memory: {}", e))?;
        self.process_dynamic_image(&img)
    }

    /// Detect on the photo the way it is displayed: EXIF orientation applied, then scaled to
    /// thumbnail size (`THUMBNAIL_SCALE` on the long edge). The confidence and blur rules were
    /// set for that scale, where the library's thumbnails used to be detected on; at full size
    /// a soft face scores below both. Boxes come back in the scaled image's pixels with its
    /// size, as `process_image_from_bytes` does for a thumbnail.
    pub fn process_image_thumbnail_scale(
        &mut self,
        image_path: &str,
    ) -> Result<(Vec<FaceData>, (u32, u32)), String> {
        let (small, _) = open_oriented_thumbnail(image_path, THUMBNAIL_SCALE)?;
        self.process_dynamic_image(&small)
    }

    fn process_dynamic_image(
        &mut self,
        img: &DynamicImage,
    ) -> Result<(Vec<FaceData>, (u32, u32)), String> {
        let faces = self.detect_faces(img)?;

        let mut results = Vec::new();
        for face in faces {
            // Filter 1: Skip low confidence faces
            if face.confidence < t_common::MIN_CONFIDENCE {
                continue;
            }

            // Filter 2: Skip very small faces (likely background people)
            // let face_area = face.width * face.height;
            // let img_width = img.width() as f32;
            // let img_height = img.height() as f32;
            // let img_area = img_width * img_height;
            // if face_area / img_area < t_common::MIN_FACE_RATIO {
            //     continue;
            // }

            // Filter 3: Skip faces smaller than minimum pixel size
            // if face.width < t_common::MIN_FACE_SIZE || face.height < t_common::MIN_FACE_SIZE {
            //     continue;
            // }

            // Filter 4: Skip blurry faces
            let blur_score = self.calculate_blur_score(img, &face);
            if blur_score < t_common::MIN_BLUR_SCORE {
                continue;
            }

            // Get embedding for quality face
            let embedding = self.get_face_embedding(img, &face)?;
            results.push(FaceData {
                bbox: face,
                embedding,
            });
        }

        Ok((results, (img.width(), img.height())))
    }

    /// Calculate blur score using Variance of Laplacian
    /// Optimized: Uses Welford's online algorithm to avoid allocating a large vector
    fn calculate_blur_score(&self, img: &DynamicImage, bbox: &FaceBox) -> f32 {
        let x = bbox.x.max(0.0) as u32;
        let y = bbox.y.max(0.0) as u32;
        // Check bounds to ensure we don't crash on cropping
        let w = bbox.width.min(img.width() as f32 - bbox.x) as u32;
        let h = bbox.height.min(img.height() as f32 - bbox.y) as u32;

        if w < 3 || h < 3 {
            return 0.0;
        }

        let crop = img.crop_imm(x, y, w, h).to_luma8();
        let (width, height) = crop.dimensions();

        // Online variance calculation (Welford's algorithm)
        let mut count = 0usize;
        let mut m2 = 0.0;
        let mut mean = 0.0;

        for y in 1..height - 1 {
            for x in 1..width - 1 {
                let p = crop.get_pixel(x, y).0[0] as i16;
                let top = crop.get_pixel(x, y - 1).0[0] as i16;
                let bottom = crop.get_pixel(x, y + 1).0[0] as i16;
                let left = crop.get_pixel(x - 1, y).0[0] as i16;
                let right = crop.get_pixel(x + 1, y).0[0] as i16;

                let sum = top + bottom + left + right - 4 * p;
                let val = sum as f32;

                count += 1;
                let delta = val - mean;
                mean += delta / count as f32;
                let delta2 = val - mean;
                m2 += delta * delta2;
            }
        }

        if count < 2 {
            return 0.0;
        }

        // Variance
        m2 / (count as f32)
    }

    /// Non-maximum suppression
    fn nms(&self, mut boxes: Vec<FaceBox>, iou_threshold: f32) -> Vec<FaceBox> {
        boxes.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));

        let mut keep = Vec::new();
        let mut suppressed = vec![false; boxes.len()];

        for i in 0..boxes.len() {
            if suppressed[i] {
                continue;
            }
            keep.push(boxes[i].clone());

            for j in (i + 1)..boxes.len() {
                if suppressed[j] {
                    continue;
                }
                if self.iou(&boxes[i], &boxes[j]) > iou_threshold {
                    suppressed[j] = true;
                }
            }
        }

        keep
    }

    /// Intersection over Union
    /// Optimized: Simplified redundant max(0.0) for valid boxes
    fn iou(&self, a: &FaceBox, b: &FaceBox) -> f32 {
        let x1 = a.x.max(b.x);
        let y1 = a.y.max(b.y);
        let x2 = (a.x + a.width).min(b.x + b.width);
        let y2 = (a.y + a.height).min(b.y + b.height);

        if x2 <= x1 || y2 <= y1 {
            return 0.0;
        }

        let inter_area = (x2 - x1) * (y2 - y1);
        let a_area = a.width * a.height;
        let b_area = b.width * b.height;

        inter_area / (a_area + b_area - inter_area)
    }
}

#[derive(Clone)]
pub struct FaceState(pub std::sync::Arc<Mutex<FaceEngine>>);

pub fn run_face_indexing(
    app_handle: AppHandle,
    face_state: FaceState,
    cancel_token_struct: FaceIndexCancellation,
    status_token_struct: FaceIndexingStatus,
    progress_token_struct: FaceIndexProgressState,
    cluster_epsilon: Option<f32>,
) -> Result<(), String> {
    let cancel_token = cancel_token_struct.0.clone();
    let status_token = status_token_struct.0.clone();
    let progress_token = progress_token_struct.0.clone();
    // Use provided epsilon or default to 0.42
    let epsilon = cluster_epsilon.unwrap_or(0.42);

    // Check if already running
    {
        let mut running = status_token.lock().unwrap();
        if *running {
            return Err("Face indexing is already running".to_string());
        }
        *running = true;
    }

    // Reset cancellation flag
    *cancel_token.lock().unwrap() = false;

    // Reset progress
    {
        let mut progress = progress_token.lock().unwrap();
        progress.current = 0;
        progress.total = 0;
        progress.faces_found = 0;
        progress.phase = "indexing".to_string();
    }

    tauri::async_runtime::spawn(async move {
        // 1. Initialization
        let reset_status = || {
            if let Ok(mut running) = status_token.lock() {
                *running = false;
            }
        };

        if !crate::t_advanced_ai::background_enabled() || *cancel_token.lock().unwrap() {
            reset_status();
            return;
        }

        // Load models if not already loaded
        {
            let mut engine = face_state.0.lock().unwrap();
            if !crate::t_advanced_ai::background_enabled() || *cancel_token.lock().unwrap() {
                reset_status();
                return;
            }
            if !engine.is_loaded() {
                if let Err(e) = engine.load_models(&app_handle) {
                    eprintln!("Failed to load face models: {}", e);
                    let _ = app_handle.emit(
                        "face_index_finished",
                        serde_json::json!({
                            "total_faces": 0,
                            "total_persons": 0,
                            "cancelled": false,
                            "error": e.to_string()
                        }),
                    );
                    reset_status();
                    return;
                }
            }
        }

        // 2. Preparation (Get files and stats)
        let (processed_count, existing_faces_count) = match t_sqlite::Face::get_stats() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to get stats: {}", e);
                (0, 0)
            }
        };

        let files = match t_sqlite::Face::get_unprocessed_image_files() {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Failed to get unprocessed files: {}", e);
                let _ = app_handle.emit(
                    "face_index_finished",
                    serde_json::json!({
                        "total_faces": 0,
                        "total_persons": 0,
                        "cancelled": false,
                        "error": e
                    }),
                );
                reset_status();
                return;
            }
        };

        let total_files = processed_count + files.len();
        let mut total_faces = existing_faces_count;
        let mut current = processed_count;

        // Init progress
        {
            let mut progress = progress_token.lock().unwrap();
            progress.total = total_files;
            progress.current = current;
            progress.faces_found = total_faces;
            progress.phase = "indexing".to_string();
        }

        let _ = app_handle.emit(
            "face_index_progress",
            serde_json::json!({
                "current": current,
                "total": total_files,
                "faces_found": total_faces,
                "phase": "indexing"
            }),
        );

        // 3. Image Processing Loop
        let mut cancelled = false;
        let db_conn = match t_sqlite::open_conn() {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("Failed to open DB connection for face indexing: {}", e);
                let _ = app_handle.emit(
                    "face_index_finished",
                    serde_json::json!({
                        "total_faces": 0,
                        "total_persons": 0,
                        "cancelled": false,
                        "error": e
                    }),
                );
                reset_status();
                return;
            }
        };

        let mut excluded_folder_cache: std::collections::HashMap<i64, bool> =
            std::collections::HashMap::new();

        for (file_id, file_path, width, height, folder_id) in files {
            if *cancel_token.lock().unwrap() {
                cancelled = true;
                break;
            }

            // Yield CPU and I/O execution during active user interaction to eliminate grid scroll stutter
            crate::scheduler::cooperative_yield_blocking_cancellable(|| *cancel_token.lock().unwrap());
            if *cancel_token.lock().unwrap() {
                cancelled = true;
                break;
            }

            current += 1;

            // Check if folder is marked as faces_excluded; if so, explicitly skip face detection
            let is_excluded = match excluded_folder_cache.get(&folder_id) {
                Some(&excluded) => excluded,
                None => {
                    let excluded = t_sqlite::AFolder::is_faces_excluded_with_conn(&db_conn, folder_id)
                        .unwrap_or(false);
                    excluded_folder_cache.insert(folder_id, excluded);
                    excluded
                }
            };

            if is_excluded {
                if current % 10 == 0 || current == total_files {
                    let mut progress = progress_token.lock().unwrap();
                    progress.current = current;
                    progress.faces_found = total_faces;

                    let _ = app_handle.emit(
                        "face_index_progress",
                        serde_json::json!({
                            "current": current,
                            "total": total_files,
                            "faces_found": total_faces,
                            "phase": "indexing"
                        }),
                    );
                }
                continue;
            }

            let mut engine = face_state.0.lock().unwrap();

            // The thumbnail in the database when there is one; otherwise the photo itself,
            // oriented and scaled to the same size (full-size detection missed soft faces and
            // every sideways one, since image::open ignores EXIF orientation).
            let (process_result, used_thumb) = match t_sqlite::AThumb::fetch(file_id) {
                Ok(Some(thumb)) if thumb.thumb_data.is_some() => {
                    let thumb_bytes = thumb.thumb_data.as_ref().unwrap();
                    match engine.process_image_from_bytes(thumb_bytes) {
                        Ok(res) => (Ok(res), true),
                        Err(_) => (engine.process_image_thumbnail_scale(&file_path), true),
                    }
                }
                _ => (engine.process_image_thumbnail_scale(&file_path), true),
            };

            match process_result {
                Ok((mut faces, (proc_w, proc_h))) => {
                    // If we used a thumbnail, scale bbox to original size
                    if used_thumb {
                        let scale_x = width as f32 / proc_w as f32;
                        let scale_y = height as f32 / proc_h as f32;

                        for face in &mut faces {
                            face.bbox.x *= scale_x;
                            face.bbox.y *= scale_y;
                            face.bbox.width *= scale_x;
                            face.bbox.height *= scale_y;
                        }
                    }

                    let has_faces = !faces.is_empty();
                    let status = if has_faces { 1 } else { 2 };

                    if let Err(e) =
                        t_sqlite::Face::mark_scanned_with_conn(&db_conn, file_id, status)
                    {
                        eprintln!("Failed to mark file {} as scanned: {}", file_id, e);
                    }

                    if has_faces {
                        for face_data in &faces {
                            let bbox_json = serde_json::json!({
                                "coordinate_space": if used_thumb { "display" } else { "raw" },
                                "x": face_data.bbox.x,
                                "y": face_data.bbox.y,
                                "width": face_data.bbox.width,
                                "height": face_data.bbox.height,
                                "confidence": face_data.bbox.confidence,
                            })
                            .to_string();

                            match t_sqlite::Face::add_with_conn(
                                &db_conn,
                                file_id,
                                &bbox_json,
                                &face_data.embedding,
                            ) {
                                Ok(_) => total_faces += 1,
                                Err(e) => eprintln!("Failed to store face: {}", e),
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to process image {}: {}", file_path, e);
                    // Do not retry files the decoder cannot read on every resume.
                    // Status 2 means the image was processed without a face result.
                    if let Err(mark_error) = t_sqlite::Face::mark_scanned_with_conn(&db_conn, file_id, 2) {
                        eprintln!("Failed to mark unreadable file {} as skipped: {}", file_id, mark_error);
                    }
                }
            }

            // Periodic progress update (every 10 files or at end)
            if current % 10 == 0 || current == total_files {
                {
                    let mut progress = progress_token.lock().unwrap();
                    progress.current = current;
                    progress.faces_found = total_faces;
                }

                let _ = app_handle.emit(
                    "face_index_progress",
                    serde_json::json!({
                        "current": current,
                        "total": total_files,
                        "faces_found": total_faces,
                        "phase": "indexing"
                    }),
                );
            }
        }

        if cancelled {
            let _ = app_handle.emit(
                "face_index_finished",
                serde_json::json!({
                    "total_faces": total_faces,
                    "total_persons": 0,
                    "cancelled": true
                }),
            );
            reset_status();
            return;
        }

        // 4. Clustering
        {
            let mut progress = progress_token.lock().unwrap();
            progress.phase = "clustering".to_string();
        }

        let _ = app_handle.emit(
            "face_index_progress",
            serde_json::json!({
                "current": total_files,
                "total": total_files,
                "faces_found": total_faces,
                "phase": "clustering"
            }),
        );

        let cancel_token_cluster = cancel_token.clone();
        let total_persons = match t_cluster::cluster_faces(
            epsilon,
            |progress| {
                let _ = app_handle.emit(
                    "cluster_progress",
                    serde_json::json!({
                        "phase": progress.phase,
                        "current": progress.current,
                        "total": progress.total,
                    }),
                );
            },
            || {
                // Check if user has cancelled
                *cancel_token_cluster.lock().unwrap()
            },
        ) {
            Ok(count) => count,
            Err(e) => {
                eprintln!("Clustering failed: {}", e);
                0
            }
        };
        let cancelled_during_cluster = *cancel_token.lock().unwrap();

        // 5. Finished
        let _ = app_handle.emit(
            "face_index_finished",
            serde_json::json!({
                "total_faces": total_faces,
                "total_persons": total_persons,
                "cancelled": cancelled_during_cluster
            }),
        );
        reset_status();
    });

    Ok(())
}

/// Result of `redetect`.
#[derive(Debug, Default, Clone, Serialize)]
pub struct RedetectStats {
    pub files: usize,
    pub with_faces: usize,
    pub faces: usize,
    pub skipped_has_faces: usize,
    pub skipped_excluded: usize,
    pub skipped_no_dims: usize,
    pub unreadable: usize,
    pub suggested: usize,
}

/// Write one photo's detections: the face rows first, with their boxes scaled from the detected
/// image (`proc`) to the photo's display size and tagged display space, then the scanned mark
/// (1 with faces, 2 without), all in one transaction, so nothing can leave a photo marked as
/// having faces without the rows. Returns the new face ids.
pub fn store_detections(
    conn: &rusqlite::Connection,
    file_id: i64,
    faces: &[FaceData],
    proc: (u32, u32),
    display: (i64, i64),
) -> Result<Vec<i64>, String> {
    if display.0 <= 0 || display.1 <= 0 || proc.0 == 0 || proc.1 == 0 {
        return Err(format!(
            "Invalid dimensions for display scaling: display={:?}, proc={:?}",
            display, proc
        ));
    }
    let scale_x = display.0 as f32 / proc.0 as f32;
    let scale_y = display.1 as f32 / proc.1 as f32;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let mut ids = Vec::with_capacity(faces.len());
    for face in faces {
        let bbox_json = serde_json::json!({
            "coordinate_space": "display",
            "x": face.bbox.x * scale_x,
            "y": face.bbox.y * scale_y,
            "width": face.bbox.width * scale_x,
            "height": face.bbox.height * scale_y,
            "confidence": face.bbox.confidence,
        })
        .to_string();
        ids.push(t_sqlite::Face::add_with_conn(&tx, file_id, &bbox_json, &face.embedding)?);
    }
    t_sqlite::Face::mark_scanned_with_conn(&tx, file_id, if faces.is_empty() { 2 } else { 1 })?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(ids)
}

/// Detect faces again on given photos with the oriented thumbnail-scale path and put the new
/// faces into People suggestions only: no clustering, no person created, renamed or assigned.
/// A photo that already has face rows is left alone. `progress(done, total, file_id, faces)`.
pub fn redetect<F: FnMut(usize, usize, i64, usize)>(
    conn: &rusqlite::Connection,
    engine: &mut FaceEngine,
    file_ids: &[i64],
    mut progress: F,
) -> Result<RedetectStats, String> {
    let mut stats = RedetectStats::default();
    let mut new_faces: Vec<i64> = Vec::new();
    for (i, &file_id) in file_ids.iter().enumerate() {
        let existing: i64 = conn
            .query_row("SELECT COUNT(*) FROM faces WHERE file_id = ?1", [file_id], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if existing > 0 {
            stats.skipped_has_faces += 1;
            progress(i + 1, file_ids.len(), file_id, 0);
            continue;
        }
        let row: Option<(String, i64, i64, i64)> = conn
            .query_row(
                "SELECT f.path || '/' || a.name, COALESCE(a.width, 0), COALESCE(a.height, 0), COALESCE(f.faces_excluded, 0)
                 FROM afiles a JOIN afolders f ON a.folder_id = f.id WHERE a.id = ?1 AND a.file_type = 1",
                [file_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .ok();
        let Some((path, width, height, faces_excluded)) = row else {
            progress(i + 1, file_ids.len(), file_id, 0);
            continue;
        };
        if faces_excluded != 0 {
            stats.skipped_excluded += 1;
            progress(i + 1, file_ids.len(), file_id, 0);
            continue;
        }
        if width <= 0 || height <= 0 {
            stats.skipped_no_dims += 1;
            progress(i + 1, file_ids.len(), file_id, 0);
            continue;
        }
        stats.files += 1;
        // Reuse cached thumbnail when available in athumbs (same as run_face_indexing)
        let cached_thumb: Option<Vec<u8>> = conn
            .query_row(
                "SELECT thumb_data FROM athumbs WHERE file_id = ?1 AND thumb_data IS NOT NULL",
                [file_id],
                |r| r.get(0),
            )
            .ok();
        let detected = match cached_thumb.as_deref() {
            Some(bytes) => match engine.process_image_from_bytes(bytes) {
                Ok(res) => Ok(res),
                Err(_) => engine.process_image_thumbnail_scale(&path),
            },
            None => engine.process_image_thumbnail_scale(&path),
        };
        let found = match detected {
            Ok((faces, proc)) => {
                new_faces.extend(store_detections(conn, file_id, &faces, proc, (width, height))?);
                faces.len()
            }
            Err(e) => {
                eprintln!("Failed to process image {}: {}", path, e);
                t_sqlite::Face::mark_scanned_with_conn(conn, file_id, 2)?;
                stats.unreadable += 1;
                0
            }
        };
        if found > 0 {
            stats.with_faces += 1;
            stats.faces += found;
        }
        progress(i + 1, file_ids.len(), file_id, found);
    }
    if !new_faces.is_empty() {
        stats.suggested = crate::t_face_suggest::suggest_faces(conn, &new_faces, crate::t_face_suggest::DEFAULT_THRESHOLD)?;
    }
    Ok(stats)
}

#[cfg(test)]
mod thumbnail_scale_tests {
    use super::*;

    /// A 400x200 JPEG tagged EXIF orientation 6 (rotate 90 CW): displayed 200 wide, 400 tall. No face in it.
    pub(super) fn write_orientation6_jpeg(path: &std::path::Path) {
        let img = image::RgbImage::from_fn(400, 200, |x, _| image::Rgb([(x / 2) as u8, 80, 120]));
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
        // Minimal APP1 Exif segment (big endian) with one IFD0 entry: Orientation (0x0112) = 6.
        let mut exif = b"Exif\0\0".to_vec();
        exif.extend_from_slice(&[0x4D, 0x4D, 0x00, 0x2A, 0x00, 0x00, 0x00, 0x08]); // MM, 42, IFD at 8
        exif.extend_from_slice(&[0x00, 0x01]); // one entry
        exif.extend_from_slice(&[0x01, 0x12, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x00]);
        exif.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // no next IFD
        let mut out = Vec::new();
        out.extend_from_slice(&jpeg[..2]); // SOI
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&exif);
        out.extend_from_slice(&jpeg[2..]);
        std::fs::write(path, &out).unwrap();
    }

    #[test]
    fn thumbnail_of_scales_the_long_edge_down_and_never_up() {
        let big = DynamicImage::new_rgb8(2048, 1365);
        let (small, from) = thumbnail_of(big, THUMBNAIL_SCALE);
        assert_eq!((small.width(), small.height()), (512, 341));
        assert_eq!(from, (2048, 1365));
        let tall = DynamicImage::new_rgb8(1365, 2048);
        assert_eq!(thumbnail_of(tall, 512).0.height(), 512);
        let tiny = DynamicImage::new_rgb8(300, 200);
        let (same, from) = thumbnail_of(tiny, 512);
        assert_eq!((same.width(), same.height()), (300, 200));
        assert_eq!(from, (300, 200));
    }

    #[test]
    fn open_oriented_thumbnail_applies_the_exif_orientation() {
        // A 400x200 JPEG tagged orientation 6 (rotate 90 CW) must come back 200 wide, 400 tall.
        let dir = std::env::temp_dir().join(format!("lap_face_orient_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("o6.jpg");
        write_orientation6_jpeg(&path);
        let (small, from) = open_oriented_thumbnail(path.to_str().unwrap(), 512).unwrap();
        assert_eq!(from, (200, 400), "oriented size");
        assert_eq!((small.width(), small.height()), (200, 400));
        // image::open alone keeps the stored 400x200, which is the bug this path fixes.
        let raw = image::open(&path).unwrap();
        assert_eq!((raw.width(), raw.height()), (400, 200));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod redetect_tests {
    use super::*;

    fn db(dir: &std::path::Path) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE persons (id INTEGER PRIMARY KEY, name TEXT, is_anchored INTEGER);
             CREATE TABLE faces (id INTEGER PRIMARY KEY, file_id INTEGER, bbox TEXT, embedding BLOB, person_id INTEGER, created_at INTEGER);
             CREATE TABLE afolders (id INTEGER PRIMARY KEY, path TEXT, faces_excluded INTEGER);
             CREATE TABLE afiles (id INTEGER PRIMARY KEY, folder_id INTEGER, name TEXT, file_type INTEGER, width INTEGER, height INTEGER, has_faces INTEGER, ai_objects TEXT);
             CREATE TABLE athumbs (id INTEGER PRIMARY KEY, file_id INTEGER, thumb_data BLOB);
             INSERT INTO afolders VALUES (1, '{0}', 0);
             INSERT INTO afolders VALUES (2, '{0}', 1);
             INSERT INTO afiles VALUES (10, 1, 'o6.jpg', 1, 200, 400, 2, 'person');
             INSERT INTO afiles VALUES (11, 1, 'o6.jpg', 1, 200, 400, 1, 'person');
             INSERT INTO afiles VALUES (12, 2, 'o6.jpg', 1, 200, 400, 2, 'person');
             INSERT INTO afiles VALUES (13, 1, 'o6.jpg', 1, 0, 0, 2, 'person');
             INSERT INTO afiles VALUES (14, 1, 'thumb_only.jpg', 1, 200, 400, 2, 'person');
             INSERT INTO faces (id, file_id, bbox, embedding, person_id) VALUES (500, 11, '{{}}', NULL, NULL);",
            dir.to_string_lossy().replace('\\', "/").replace('\'', "''")
        )).unwrap();
        crate::t_face_suggest::migrate(&conn).unwrap();
        conn
    }

    fn fake_face(x: f32, y: f32, w: f32, h: f32) -> FaceData {
        FaceData { bbox: FaceBox { x, y, width: w, height: h, confidence: 0.9, landmarks: None }, embedding: vec![0.0; 512] }
    }

    #[test]
    fn store_detections_scales_to_display_space_and_marks_after_the_rows() {
        let dir = std::env::temp_dir().join(format!("lap_store_det_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = db(&dir);
        let ids = store_detections(&conn, 10, &[fake_face(10.0, 20.0, 30.0, 40.0)], (100, 200), (1000, 2000)).unwrap();
        assert_eq!(ids.len(), 1);
        let (bbox, person, has): (String, Option<i64>, i64) = conn
            .query_row("SELECT f.bbox, f.person_id, a.has_faces FROM faces f JOIN afiles a ON a.id = f.file_id WHERE f.id = ?1", [ids[0]], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap();
        let b: serde_json::Value = serde_json::from_str(&bbox).unwrap();
        assert_eq!(b["coordinate_space"], "display");
        assert_eq!((b["x"].as_f64().unwrap(), b["y"].as_f64().unwrap(), b["width"].as_f64().unwrap(), b["height"].as_f64().unwrap()), (100.0, 200.0, 300.0, 400.0));
        assert_eq!(person, None, "never assigned");
        assert_eq!(has, 1);
        // No faces: the mark says so and no row appears.
        conn.execute("UPDATE afiles SET has_faces = NULL WHERE id = 10", []).unwrap();
        assert!(store_detections(&conn, 10, &[], (100, 200), (1000, 2000)).unwrap().is_empty());
        assert_eq!(conn.query_row("SELECT has_faces FROM afiles WHERE id = 10", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM faces WHERE file_id = 10", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_detections_hard_errors_on_invalid_display_dimensions() {
        let dir = std::env::temp_dir().join(format!("lap_store_det_err_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = db(&dir);
        assert!(store_detections(&conn, 10, &[fake_face(1.0, 1.0, 10.0, 10.0)], (100, 200), (0, 0)).is_err());
        assert!(store_detections(&conn, 10, &[fake_face(1.0, 1.0, 10.0, 10.0)], (100, 200), (-5, 100)).is_err());
        assert!(store_detections(&conn, 10, &[fake_face(1.0, 1.0, 10.0, 10.0)], (0, 0), (1000, 2000)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Real models on a face-less oriented JPEG: the photo is read the display way up, gets no row
    /// and is marked 2; a photo that already has a face row, an excluded folder, or missing dims is skipped.
    #[test]
    fn redetect_skips_photos_with_faces_and_marks_faceless_ones() {
        let models = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/models");
        let mut engine = FaceEngine::new();
        if engine.load_models_from_dir(&models).is_err() { eprintln!("face models not present, skipping"); return; }
        let dir = std::env::temp_dir().join(format!("lap_redetect_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        super::thumbnail_scale_tests::write_orientation6_jpeg(&dir.join("o6.jpg"));
        let conn = db(&dir);
        let mut seen = Vec::new();
        let stats = redetect(&conn, &mut engine, &[10, 11, 12, 13], |done, total, file_id, faces| seen.push((done, total, file_id, faces))).unwrap();
        assert_eq!(stats.files, 1, "only the photo without face rows/exclusions/missing dims is processed");
        assert_eq!(stats.skipped_has_faces, 1);
        assert_eq!(stats.skipped_excluded, 1);
        assert_eq!(stats.skipped_no_dims, 1);
        assert_eq!(stats.unreadable, 0);
        assert_eq!(stats.faces, 0);
        assert_eq!(conn.query_row("SELECT has_faces FROM afiles WHERE id = 10", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        assert_eq!(conn.query_row("SELECT has_faces FROM afiles WHERE id = 11", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM faces", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(seen.len(), 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// When athumbs contains cached thumbnail bytes, redetect processes them without needing the original file on disk.
    #[test]
    fn redetect_uses_cached_thumbnail_from_db_when_available() {
        let models = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/models");
        let mut engine = FaceEngine::new();
        if engine.load_models_from_dir(&models).is_err() { eprintln!("face models not present, skipping"); return; }
        let dir = std::env::temp_dir().join(format!("lap_redetect_thumb_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // File 14 points to 'thumb_only.jpg' which does NOT exist on disk!
        let conn = db(&dir);
        let img = image::RgbImage::from_fn(100, 100, |_, _| image::Rgb([100, 100, 100]));
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
        conn.execute("INSERT INTO athumbs (file_id, thumb_data) VALUES (14, ?1)", rusqlite::params![jpeg]).unwrap();
        let stats = redetect(&conn, &mut engine, &[14], |_, _, _, _| ()).unwrap();
        assert_eq!(stats.files, 1);
        assert_eq!(stats.unreadable, 0, "processed from cached athumbs thumb_data without file on disk");
        assert_eq!(conn.query_row("SELECT has_faces FROM afiles WHERE id = 14", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
