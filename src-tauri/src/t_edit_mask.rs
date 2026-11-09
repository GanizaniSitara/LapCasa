//! Masks for local (painted or gradient) effects, shared with the AI editing masks.
//!
//! A mask is an 8-bit greyscale PNG the size of the photo (as oriented, before crop and rotate,
//! the frame effects are applied in) plus a small JSON (feather, inverted, how it was made). Both
//! live next to the photo under `.lapcasa-ai/<file name>/masks/<id>.png|json`, where the id is a
//! hash of their content, so a file is never rewritten and every history step keeps its own mask.
//! An effect step names its mask by id: `{"name": "HDR-ish", "amount": 0.33, "mask": "<id>"}`.
use image::{DynamicImage, GrayImage, RgbImage};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub struct Mask {
    pub luma: GrayImage,
    /// Gaussian sigma as a fraction of the short side of the image the mask is applied to.
    pub feather: f32,
    pub inverted: bool,
}

pub fn mask_dir(photo: &Path) -> Result<PathBuf, String> {
    let name = photo.file_name().ok_or("Photo has no file name")?;
    let parent = photo.parent().ok_or("Photo has no folder")?;
    Ok(parent.join(".lapcasa-ai").join(name).join("masks"))
}

fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Store a painted mask (any image; its grey level is the effect strength, white = full) for
/// `photo`, scaled to `size` (width, height of the oriented photo; swapped if the mask was
/// painted the other way round). Returns the mask id.
pub fn save(photo: &Path, image_bytes: &[u8], size: (u32, u32), feather: f32, inverted: bool, info: Value) -> Result<String, String> {
    let painted = image::load_from_memory(image_bytes).map_err(|e| format!("Mask is not an image: {e}"))?.to_luma8();
    let (pw, ph) = painted.dimensions();
    let (mut w, mut h) = size;
    if w == 0 || h == 0 { (w, h) = (pw, ph); }
    if (pw > ph) != (w > h) && pw != ph { std::mem::swap(&mut w, &mut h); }
    let luma = if (pw, ph) == (w, h) { painted } else {
        image::imageops::resize(&painted, w, h, image::imageops::FilterType::Triangle)
    };
    let mut png = Vec::new();
    DynamicImage::ImageLuma8(luma).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let mut meta = json!({"version": 1, "width": w, "height": h, "feather": feather.clamp(0.0, 0.25), "inverted": inverted});
    if let (Some(meta), Some(info)) = (meta.as_object_mut(), info.as_object()) {
        for (k, v) in info { meta.entry(k.clone()).or_insert(v.clone()); }
    }
    let text = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(&png);
    hasher.update(text.as_bytes());
    let id = hasher.finalize().to_hex()[..32].to_string();
    let dir = mask_dir(photo)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let png_path = dir.join(format!("{id}.png"));
    if !png_path.exists() {
        std::fs::write(dir.join(format!("{id}.json")), &text).map_err(|e| e.to_string())?;
        std::fs::write(&png_path, &png).map_err(|e| e.to_string())?;
    }
    Ok(id)
}

pub fn load(photo: &Path, id: &str) -> Result<Mask, String> {
    if !valid_id(id) { return Err(format!("Invalid mask id {id}")); }
    let dir = mask_dir(photo)?;
    let luma = image::open(dir.join(format!("{id}.png"))).map_err(|e| format!("Mask {id}: {e}"))?.to_luma8();
    let meta: Value = std::fs::read_to_string(dir.join(format!("{id}.json"))).ok()
        .and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    Ok(Mask {
        luma,
        feather: meta["feather"].as_f64().unwrap_or(0.0) as f32,
        inverted: meta["inverted"].as_bool().unwrap_or(false),
    })
}

/// Separable Gaussian, cut off at 3 sigma so pixels further than that from any painted pixel stay
/// exactly 0 (and the photo there exactly unchanged).
fn feather(luma: &GrayImage, sigma: f32) -> GrayImage {
    if sigma < 0.3 { return luma.clone(); }
    let radius = (sigma * 3.0).ceil() as i32;
    let kernel: Vec<f32> = (-radius..=radius).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let total: f32 = kernel.iter().sum();
    let kernel: Vec<f32> = kernel.iter().map(|k| k / total).collect();
    let (w, h) = luma.dimensions();
    let pass = |src: &[f32], horizontal: bool| -> Vec<f32> {
        let mut out = vec![0.0f32; src.len()];
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let mut sum = 0.0;
                for (k, weight) in kernel.iter().enumerate() {
                    let d = k as i32 - radius;
                    let (sx, sy) = if horizontal { ((x + d).clamp(0, w as i32 - 1), y) } else { (x, (y + d).clamp(0, h as i32 - 1)) };
                    sum += src[(sy as u32 * w + sx as u32) as usize] * weight;
                }
                out[(y as u32 * w + x as u32) as usize] = sum;
            }
        }
        out
    };
    let src: Vec<f32> = luma.as_raw().iter().map(|&v| v as f32).collect();
    let out = pass(&pass(&src, true), false);
    GrayImage::from_raw(w, h, out.iter().map(|&v| v.clamp(0.0, 255.0).round() as u8).collect()).unwrap()
}

/// The mask's weight per pixel for a `w` x `h` image: scaled, feathered, inverted if asked.
pub fn weights(mask: &Mask, w: u32, h: u32) -> GrayImage {
    let scaled = if mask.luma.dimensions() == (w, h) { mask.luma.clone() } else {
        image::imageops::resize(&mask.luma, w, h, image::imageops::FilterType::Triangle)
    };
    let mut out = feather(&scaled, mask.feather * w.min(h) as f32);
    if mask.inverted { for v in out.iter_mut() { *v = 255 - *v; } }
    out
}

/// original * (1 - m) + effect * m per pixel; m = 0 keeps the original byte for byte.
pub fn blend(original: &RgbImage, effect: &RgbImage, m: &GrayImage) -> RgbImage {
    let mut out = original.clone();
    for (x, y, p) in out.enumerate_pixels_mut() {
        let a = m.get_pixel(x, y)[0];
        if a == 0 { continue; }
        let e = effect.get_pixel(x, y);
        if a == 255 { *p = *e; continue; }
        let a = a as f32 / 255.0;
        for c in 0..3 { p[c] = (p[c] as f32 + (e[c] as f32 - p[c] as f32) * a).round() as u8; }
    }
    out
}

/// Apply an effect's output through its mask. A mask that cannot be read leaves the photo as it
/// was before the effect (a moved photo without its `.lapcasa-ai` folder), never a broken render.
pub fn apply(original: DynamicImage, effect: DynamicImage, photo: Option<&Path>, id: &str) -> DynamicImage {
    let mask = match photo.ok_or_else(|| "no photo path".to_string()).and_then(|p| load(p, id)) {
        Ok(mask) => mask,
        Err(e) => { eprintln!("effect mask skipped: {e}"); return original; }
    };
    let original = original.to_rgb8();
    let effect = effect.to_rgb8();
    if original.dimensions() != effect.dimensions() { return DynamicImage::ImageRgb8(effect); }
    let m = weights(&mask, original.width(), original.height());
    DynamicImage::ImageRgb8(blend(&original, &effect, &m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_photo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lap_mask_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("IMG_0001.jpg")
    }

    fn top_half(w: u32, h: u32) -> Vec<u8> {
        let img = GrayImage::from_fn(w, h, |_, y| image::Luma([if y < h / 2 { 255 } else { 0 }]));
        let mut png = Vec::new();
        DynamicImage::ImageLuma8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        png
    }

    #[test]
    fn saved_mask_is_image_sized_grey_png_with_json_and_content_id() {
        let photo = temp_photo("save");
        let id = save(&photo, &top_half(40, 30), (400, 300), 0.02, false, json!({"kind": "paint"})).unwrap();
        let dir = mask_dir(&photo).unwrap();
        let png = image::open(dir.join(format!("{id}.png"))).unwrap();
        assert_eq!((png.width(), png.height()), (400, 300));
        assert!(matches!(png, DynamicImage::ImageLuma8(_)));
        let meta: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(format!("{id}.json"))).unwrap()).unwrap();
        assert_eq!(meta["kind"], "paint");
        assert_eq!(meta["inverted"], false);
        // Same content, same id; a different setting, a new file (history keeps both).
        assert_eq!(save(&photo, &top_half(40, 30), (400, 300), 0.02, false, json!({"kind": "paint"})).unwrap(), id);
        let inverted = save(&photo, &top_half(40, 30), (400, 300), 0.02, true, json!({"kind": "paint"})).unwrap();
        assert_ne!(inverted, id);
        assert!(load(&photo, &inverted).unwrap().inverted);
        // A portrait painting for a photo whose raw size is landscape (EXIF-rotated) keeps its shape.
        let rotated = save(&photo, &top_half(30, 40), (400, 300), 0.0, false, Value::Null).unwrap();
        assert_eq!(load(&photo, &rotated).unwrap().luma.dimensions(), (300, 400));
        assert!(load(&photo, "../../etc/passwd").is_err());
        let _ = std::fs::remove_dir_all(photo.parent().unwrap());
    }

    #[test]
    fn masked_effect_changes_only_inside_and_keeps_outside_byte_identical() {
        let photo = temp_photo("blend");
        let (w, h) = (64u32, 64u32);
        let id = save(&photo, &top_half(w, h), (w, h), 0.03, false, Value::Null).unwrap();
        let original = RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 4) as u8, (y * 4) as u8, 90]));
        let effect = RgbImage::from_pixel(w, h, image::Rgb([255, 0, 0]));
        let out = apply(DynamicImage::ImageRgb8(original.clone()), DynamicImage::ImageRgb8(effect.clone()), Some(&photo), &id).to_rgb8();
        let band = (0.03 * h as f32 * 3.0).ceil() as u32 + 1;
        for (x, y, p) in out.enumerate_pixels() {
            if y + band < h / 2 { assert_eq!(p, effect.get_pixel(x, y), "inside at {x},{y}"); }
            if y >= h / 2 + band { assert_eq!(p, original.get_pixel(x, y), "outside at {x},{y}"); }
        }
        // The edge is a smooth ramp, not a step.
        let col: Vec<u8> = (0..h).map(|y| out.get_pixel(0, y)[1]).collect();
        assert!(col.windows(2).filter(|p| p[0] != p[1]).count() > 2);
        // Inverted: the other half.
        let inv = save(&photo, &top_half(w, h), (w, h), 0.03, true, Value::Null).unwrap();
        let out = apply(DynamicImage::ImageRgb8(original.clone()), DynamicImage::ImageRgb8(effect.clone()), Some(&photo), &inv).to_rgb8();
        assert_eq!(out.get_pixel(5, 1), original.get_pixel(5, 1));
        assert_eq!(out.get_pixel(5, h - 2), effect.get_pixel(5, h - 2));
        // A missing mask leaves the photo as it was before the effect.
        let out = apply(DynamicImage::ImageRgb8(original.clone()), DynamicImage::ImageRgb8(effect), Some(&photo), &"0".repeat(32)).to_rgb8();
        assert_eq!(out, original);
        let _ = std::fs::remove_dir_all(photo.parent().unwrap());
    }

    #[test]
    fn gradient_mask_gives_a_monotonic_transition() {
        let photo = temp_photo("gradient");
        let (w, h) = (32u32, 100u32);
        let grad = GrayImage::from_fn(w, h, |_, y| image::Luma([(255.0 * (1.0 - (y as f32 - 25.0).clamp(0.0, 50.0) / 50.0)).round() as u8]));
        let mut png = Vec::new();
        DynamicImage::ImageLuma8(grad).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let id = save(&photo, &png, (w, h), 0.0, false, json!({"kind": "gradient"})).unwrap();
        let m = weights(&load(&photo, &id).unwrap(), w, h);
        let col: Vec<u8> = (0..h).map(|y| m.get_pixel(3, y)[0]).collect();
        assert!(col.windows(2).all(|p| p[0] >= p[1]));
        assert_eq!((col[0], col[(h - 1) as usize]), (255, 0));
        assert!(col.iter().filter(|&&v| v > 0 && v < 255).count() > 40);
        let _ = std::fs::remove_dir_all(photo.parent().unwrap());
    }
}
