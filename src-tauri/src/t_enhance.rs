/**
 * Non-destructive image enhancement ("I'm Feeling Lucky").
 * Provides fast FOSS auto-contrast stretch, color balance, and adaptive
 * midtone luminance correction using the `image` crate.
 */
use image::{DynamicImage, GenericImageView, ImageFormat};
use std::io::Cursor;

/// Applies a fast auto-enhance ("I'm Feeling Lucky") algorithm in-place.
///
/// One-click photo enhancement:
/// 1. Builds subsampled histograms of R, G, B channels and luminance Y.
/// 2. Performs percentile clipping (0.5% shadow, 0.5% highlight) to establish dynamic range.
/// 3. Computes adaptive gamma from median luminance to lift underexposed shadows
///    and tame overexposed highlights without blowing out detail.
/// 4. Blends per-channel bounds with overall luminance bounds to remove unsightly
///    color casts (e.g. tungsten/fluorescent) while preserving natural scene hues.
/// 5. Applies 256-element precomputed LUTs for high-throughput pixel transformation.
pub fn apply_auto_enhance(img: &mut DynamicImage) {
    match img {
        DynamicImage::ImageRgb8(rgb) => {
            enhance_rgb_buffer(rgb.as_mut());
        }
        DynamicImage::ImageRgba8(rgba) => {
            enhance_rgba_buffer(rgba.as_mut());
        }
        _ => {
            let mut rgba = img.to_rgba8();
            enhance_rgba_buffer(rgba.as_mut());
            *img = DynamicImage::ImageRgba8(rgba);
        }
    }
}

/// Enhances an RGB (3 bytes per pixel) image buffer in-place.
fn enhance_rgb_buffer(buffer: &mut [u8]) {
    enhance_interleaved_buffer(buffer, 3);
}

/// Enhances an RGBA (4 bytes per pixel) image buffer in-place (alpha channel left untouched).
fn enhance_rgba_buffer(buffer: &mut [u8]) {
    enhance_interleaved_buffer(buffer, 4);
}

/// Generic enhancer for interleaved 8-bit RGB/RGBA pixel slices.
fn enhance_interleaved_buffer(buffer: &mut [u8], step: usize) {
    let num_pixels = buffer.len() / step;
    if num_pixels == 0 {
        return;
    }

    // Subsample if image is large (> 200,000 pixels) so histogram calculation takes < 1ms
    let stride = (num_pixels / 200_000).max(1);

    let mut hist_r = [0u32; 256];
    let mut hist_g = [0u32; 256];
    let mut hist_b = [0u32; 256];
    let mut hist_y = [0u32; 256];
    let mut sampled_count = 0u32;

    for i in (0..num_pixels).step_by(stride) {
        let offset = i * step;
        let r = buffer[offset];
        let g = buffer[offset + 1];
        let b = buffer[offset + 2];

        // BT.601 integer luminance: (299*R + 587*G + 114*B) / 1000
        let y = ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000).min(255) as usize;

        hist_r[r as usize] += 1;
        hist_g[g as usize] += 1;
        hist_b[b as usize] += 1;
        hist_y[y] += 1;
        sampled_count += 1;
    }

    if sampled_count == 0 {
        return;
    }

    // 0.5% percentile clipping for shadows and highlights
    let low_threshold = (sampled_count as f32 * 0.005) as u32;
    let high_threshold = (sampled_count as f32 * 0.995) as u32;

    let (y_low, y_high) = find_percentiles(&hist_y, low_threshold, high_threshold);
    let (r_low, r_high) = find_percentiles(&hist_r, low_threshold, high_threshold);
    let (g_low, g_high) = find_percentiles(&hist_g, low_threshold, high_threshold);
    let (b_low, b_high) = find_percentiles(&hist_b, low_threshold, high_threshold);

    // Median luminance calculation
    let median_threshold = sampled_count / 2;
    let mut cum_y = 0u32;
    let mut median_y = 128u8;
    for (i, &count) in hist_y.iter().enumerate() {
        cum_y += count;
        if cum_y >= median_threshold {
            median_y = i as u8;
            break;
        }
    }

    // Adaptive gamma based on median luminance
    let y_range = (y_high as f32 - y_low as f32).max(1.0);
    let normalized_mid = ((median_y as f32 - y_low as f32) / y_range).clamp(0.05, 0.95);
    // Target normalized midtone of 0.5: midtone^gamma = 0.5 => gamma = ln(0.5) / ln(mid)
    let raw_gamma = (0.5f32.ln()) / (normalized_mid.ln());
    // Clamp gamma to safe range [0.75, 1.35]
    let gamma = raw_gamma.clamp(0.75, 1.35);

    // Blend per-channel bounds with luminance bounds (60% channel, 40% luminance)
    // to correct color cast without altering strong intentional color moods
    let (min_r, max_r) = blend_range(r_low, r_high, y_low, y_high);
    let (min_g, max_g) = blend_range(g_low, g_high, y_low, y_high);
    let (min_b, max_b) = blend_range(b_low, b_high, y_low, y_high);

    let lut_r = build_lut(min_r, max_r, gamma);
    let lut_g = build_lut(min_g, max_g, gamma);
    let lut_b = build_lut(min_b, max_b, gamma);

    // Apply LUTs
    for chunk in buffer.chunks_exact_mut(step) {
        chunk[0] = lut_r[chunk[0] as usize];
        chunk[1] = lut_g[chunk[1] as usize];
        chunk[2] = lut_b[chunk[2] as usize];
    }
}

fn find_percentiles(hist: &[u32; 256], low_th: u32, high_th: u32) -> (u8, u8) {
    let mut cum = 0u32;
    let mut low = 0u8;
    for (i, &count) in hist.iter().enumerate() {
        cum += count;
        if cum >= low_th {
            low = i as u8;
            break;
        }
    }

    let mut cum2 = 0u32;
    let mut high = 255u8;
    for (i, &count) in hist.iter().enumerate() {
        cum2 += count;
        if cum2 >= high_th {
            high = i as u8;
            break;
        }
    }

    if high <= low {
        (low, (low as u16 + 1).min(255) as u8)
    } else {
        (low, high)
    }
}

fn blend_range(c_low: u8, c_high: u8, y_low: u8, y_high: u8) -> (f32, f32) {
    let low = 0.6 * (c_low as f32) + 0.4 * (y_low as f32);
    let high = 0.6 * (c_high as f32) + 0.4 * (y_high as f32);
    if high <= low {
        (low, low + 1.0)
    } else {
        (low, high)
    }
}

fn build_lut(min_v: f32, max_v: f32, gamma: f32) -> [u8; 256] {
    let mut lut = [0u8; 256];
    let range = (max_v - min_v).max(1.0);
    for i in 0..256 {
        let norm = ((i as f32 - min_v) / range).clamp(0.0, 1.0);
        let mapped = norm.powf(gamma);
        lut[i] = (mapped * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
    }
    lut
}

/// Applies auto enhancement directly to encoded image bytes (JPEG, PNG, WebP, etc.).
/// Encodes the enhanced image back into its original format (or JPEG fallback).
pub fn apply_auto_enhance_to_bytes(data: &[u8]) -> Result<Vec<u8>, String> {
    apply_auto_enhance_to_bytes_for_file(data, None)
}

/// Apply the Magic Wand to a preview, using indexed face boxes when available.
/// The source image and stored edits are never changed.
pub fn apply_auto_enhance_to_bytes_for_file(data: &[u8], file_id: Option<i64>) -> Result<Vec<u8>, String> {
    let mut img = image::load_from_memory(data).map_err(|e| e.to_string())?;
    apply_auto_enhance(&mut img);
    if let Some(file_id) = file_id {
        if let Ok(conn) = crate::t_sqlite::open_conn() {
            let dimensions: Option<(f32, f32)> = conn.query_row(
                "SELECT width, height FROM afiles WHERE id = ?1",
                rusqlite::params![file_id],
                |row| Ok((row.get::<_, f32>(0)?, row.get::<_, f32>(1)?)),
            ).ok();
            if let Some((source_w, source_h)) = dimensions.filter(|(w, h)| *w > 0.0 && *h > 0.0) {
                if let Ok(mut stmt) = conn.prepare("SELECT bbox FROM faces WHERE file_id = ?1") {
                    if let Ok(rows) = stmt.query_map(rusqlite::params![file_id], |row| row.get::<_, String>(0)) {
                        let boxes: Vec<FaceRegion> = rows.filter_map(Result::ok)
                            .filter_map(|json| serde_json::from_str(&json).ok()).collect();
                        remove_red_eye(&mut img, &boxes, (source_w, source_h));
                    }
                }
            }
        }
    }
    let mut out = Cursor::new(Vec::new());
    let format = image::guess_format(data).unwrap_or(ImageFormat::Jpeg);
    match img.write_to(&mut out, format) {
        Ok(_) => Ok(out.into_inner()),
        Err(_) => {
            let mut fallback_out = Cursor::new(Vec::new());
            img.write_to(&mut fallback_out, ImageFormat::Jpeg)
                .map_err(|e| e.to_string())?;
            Ok(fallback_out.into_inner())
        }
    }
}

#[derive(serde::Deserialize)]
struct FaceRegion { x: f32, y: f32, width: f32, height: f32 }

pub(crate) fn correct_red_eye_for_file(img: &mut DynamicImage, file_id: i64) -> Result<(),String> {
    let file=crate::t_sqlite::AFile::get_file_info(file_id)?.ok_or("Photo not found")?;
    let path=file.file_path.as_deref().ok_or("Photo has no path")?;
    let (width,height)=crate::t_image::get_image_dimensions(path)?;
    let orientation=crate::t_image::get_image_orientation(path);
    let conn=crate::t_sqlite::open_conn()?;
    let mut regions=Vec::new();
    let mut dimensions=(width,height);
    for face in crate::t_sqlite::Face::get_for_file(file_id)? {
        let bbox:serde_json::Value=serde_json::from_str(&face.bbox).map_err(|e|e.to_string())?;
        let b=["x","y","width","height"].map(|k|bbox[k].as_f64().unwrap_or(0.0));
        let display=crate::t_face_thumb::is_display_box(&conn,file_id,face.id)?;
        if let Some((rect,dims))=crate::t_edit_recipe::map_bbox(b,width,height,orientation,display,None) {
            dimensions=(dims[0],dims[1]);
            regions.push(FaceRegion{x:rect[0] as f32,y:rect[1] as f32,width:rect[2] as f32,height:rect[3] as f32});
        }
    }
    remove_red_eye(img,&regions,(dimensions.0 as f32,dimensions.1 as f32));
    Ok(())
}

/// Limit corrections to two small eye regions in each detected face. Indexed
/// boxes use displayed-image coordinates; previews may be scaled thumbnails.
fn remove_red_eye(img: &mut DynamicImage, faces: &[FaceRegion], source: (f32, f32)) {
    if faces.is_empty() { return; }
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 { return; }
    if ((w as f32 / h as f32) / (source.0 / source.1) - 1.0).abs() > 0.1 { return; }
    let sx = w as f32 / source.0;
    let sy = h as f32 / source.1;
    let mut rgb = img.to_rgba8();
    for face in faces {
        if !face.x.is_finite() || !face.y.is_finite() || !face.width.is_finite() || !face.height.is_finite()
            || face.width <= 0.0 || face.height <= 0.0 { continue; }
        let fx = face.x * sx;
        let fy = face.y * sy;
        let fw = face.width * sx;
        let fh = face.height * sy;
        for eye_x in [0.32f32, 0.68] {
            let cx = fx + eye_x * fw;
            let cy = fy + 0.38 * fh;
            let rx = (fw * 0.13).max(1.0);
            let ry = (fh * 0.10).max(1.0);
            let x0 = (cx - rx).max(0.0) as u32;
            let x1 = (cx + rx).min(w as f32) as u32;
            let y0 = (cy - ry).max(0.0) as u32;
            let y1 = (cy + ry).min(h as f32) as u32;
            for y in y0..y1 {
                for x in x0..x1 {
                    let dx = (x as f32 + 0.5 - cx) / rx;
                    let dy = (y as f32 + 0.5 - cy) / ry;
                    if dx * dx + dy * dy > 1.0 { continue; }
                    let p = rgb.get_pixel_mut(x, y);
                    let [r, g, b, _] = p.0;
                    if r > 100 && r as f32 > g as f32 * 1.6 && r as f32 > b as f32 * 1.5 {
                        p.0[0] = ((g as u16 + b as u16) / 2) as u8;
                    }
                }
            }
        }
    }
    *img = DynamicImage::ImageRgba8(rgb);
}

/// Checks whether an `edits` JSON string has `auto_enhance: true`.
pub fn is_auto_enhance_active(edits_json: &str) -> bool {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(edits_json) {
        val.get("auto_enhance")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    } else {
        false
    }
}

/// Queries SQLite to check if a file has `auto_enhance: true` in its `edits` column.
pub fn is_file_auto_enhanced(file_id: i64) -> bool {
    if file_id <= 0 {
        return false;
    }
    let conn = match crate::t_sqlite::open_conn() {
        Ok(c) => c,
        Err(_) => return false,
    };
    let edits_str: Option<String> = conn
        .query_row(
            "SELECT edits FROM afiles WHERE id = ?1",
            rusqlite::params![file_id],
            |row| row.get(0),
        )
        .ok()
        .flatten();

    edits_str.as_deref().map(is_auto_enhance_active).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage, Rgba, RgbaImage};

    #[test]
    fn red_eye_changes_only_red_pixels_in_detected_eye_regions() {
        let mut img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 100, Rgba([80, 70, 60, 255])));
        let red = Rgba([240, 35, 30, 255]);
        img.as_mut_rgba8().unwrap().put_pixel(32, 38, red);
        img.as_mut_rgba8().unwrap().put_pixel(68, 38, red);
        img.as_mut_rgba8().unwrap().put_pixel(50, 80, red);
        remove_red_eye(&mut img, &[FaceRegion { x: 0.0, y: 0.0, width: 100.0, height: 100.0 }], (100.0, 100.0));
        let pixels = img.to_rgba8();
        assert!(pixels.get_pixel(32, 38)[0] < 100);
        assert!(pixels.get_pixel(68, 38)[0] < 100);
        assert_eq!(*pixels.get_pixel(50, 80), red);
        assert_eq!(*pixels.get_pixel(50, 50), Rgba([80, 70, 60, 255]));
    }

    #[test]
    fn test_is_auto_enhance_active() {
        assert!(is_auto_enhance_active(r#"{"auto_enhance": true}"#));
        assert!(is_auto_enhance_active(r#"{"auto_enhance": true, "crop": [0,0,10,10]}"#));
        assert!(!is_auto_enhance_active(r#"{"auto_enhance": false}"#));
        assert!(!is_auto_enhance_active(r#"{}"#));
        assert!(!is_auto_enhance_active("invalid json"));
    }

    #[test]
    fn test_contrast_stretch_expands_range() {
        // Create a 100x100 dull low-contrast image where all values are between 100 and 150
        let mut img = RgbImage::new(100, 100);
        for (x, y, pixel) in img.enumerate_pixels_mut() {
            let val = 100 + ((x + y) % 50) as u8;
            *pixel = Rgb([val, val, val]);
        }
        let mut dyn_img = DynamicImage::ImageRgb8(img);
        apply_auto_enhance(&mut dyn_img);

        let rgb = dyn_img.as_rgb8().expect("Should remain RGB8");
        let min_val = rgb.pixels().map(|p| p[0]).min().unwrap();
        let max_val = rgb.pixels().map(|p| p[0]).max().unwrap();

        // The contrast stretch should significantly expand the range beyond [100, 149]
        assert!(min_val < 30, "Expected min_val < 30 after auto-enhance, got {}", min_val);
        assert!(max_val > 220, "Expected max_val > 220 after auto-enhance, got {}", max_val);
    }

    #[test]
    fn test_apply_auto_enhance_to_bytes() {
        // Create a small PNG image
        let mut img = RgbaImage::new(20, 20);
        for pixel in img.pixels_mut() {
            *pixel = Rgba([80, 90, 100, 255]);
        }
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(img)
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();

        let enhanced_bytes = apply_auto_enhance_to_bytes(&bytes)
            .expect("Enhance bytes should succeed");
        assert!(!enhanced_bytes.is_empty());

        let decoded = image::load_from_memory(&enhanced_bytes)
            .expect("Enhanced bytes should be a valid image");
        assert_eq!(decoded.width(), 20);
        assert_eq!(decoded.height(), 20);
    }
}
