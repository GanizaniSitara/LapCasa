/**
 * Non-destructive image enhancement ("I'm Feeling Lucky" / Picasa & iAuto pipelines).
 * Provides 5 distinct reverse-engineered one-shot auto-enhancement pipelines,
 * each evaluated non-additively from the unedited base image:
 *   1: iAuto 2019 (Default — balanced contrast pop, 60/40 cast removal, +10% midtone vibrancy that
 *      skin tones half take, highlight rolloff instead of clipping). LapCasa's own pipeline tuned
 *      to iAuto 2019's stored settings (uid 2019102, no red-eye); Perfectly Clear's adaptive
 *      formulas are not recovered, so this is not a pixel match (LAP-043).
 *   2: iAuto Classic (Gentle natural stretch, 50/50 cast balance, highlight deflash rolloff)
 *   3: The Outdoors (Crisp contrast, +18% vibrancy, selective Sky blue & Foliage green boost)
 *   4: Portrait (Soft shadow fill-light lift, gentle facial contrast, warm skin-tone protection)
 *   5: Vivid (High-impact depth S-curve, black-point anchor, +22% midtone vibrancy)
 */
use image::{DynamicImage, GenericImageView, ImageFormat};
use std::io::Cursor;

pub const PRESET_COUNT: u8 = 5;

pub fn normalize_preset(preset: u8) -> u8 {
    if (1..=PRESET_COUNT).contains(&preset) {
        preset
    } else {
        1
    }
}

pub fn preset_name(preset: u8) -> &'static str {
    match normalize_preset(preset) {
        1 => "Balanced",
        2 => "Natural",
        3 => "Landscape",
        4 => "Portrait",
        5 => "Vivid",
        _ => "Balanced",
    }
}

/// Applies the default auto-enhance ("I'm Feeling Lucky" preset 1: iAuto 2019) in-place.
pub fn apply_auto_enhance(img: &mut DynamicImage) {
    apply_auto_enhance_preset(img, 1);
}

/// Applies a specific non-additive `I'm Feeling Lucky` / `iAuto` preset (`1..=5`) in-place
/// starting from the base image buffer.
pub fn apply_auto_enhance_preset(img: &mut DynamicImage, preset: u8) {
    let preset = normalize_preset(preset);
    match img {
        DynamicImage::ImageRgb8(rgb) => {
            enhance_interleaved_buffer_preset(rgb.as_mut(), 3, preset);
        }
        DynamicImage::ImageRgba8(rgba) => {
            enhance_interleaved_buffer_preset(rgba.as_mut(), 4, preset);
        }
        _ => {
            let mut rgba = img.to_rgba8();
            enhance_interleaved_buffer_preset(rgba.as_mut(), 4, preset);
            *img = DynamicImage::ImageRgba8(rgba);
        }
    }
}

/// Generic enhancer for interleaved 8-bit RGB/RGBA pixel slices with preset selection.
fn enhance_interleaved_buffer_preset(buffer: &mut [u8], step: usize, preset: u8) {
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

    // Preset-specific clipping percentiles and channel-vs-luminance blend weights
    let (low_pct, high_pct, channel_weight, gamma_min, gamma_max) = match preset {
        1 => (0.005, 0.995, 0.60, 0.75, 1.35), // iAuto 2019
        2 => (0.005, 0.995, 0.48, 0.84, 1.20), // iAuto Classic (gentler stretch & cast balance)
        3 => (0.003, 0.997, 0.55, 0.78, 1.30), // The Outdoors (crisp full-range stretch)
        4 => (0.006, 0.994, 0.42, 0.74, 1.18), // Portrait / Beautify (flattering shadow lift, soft highlights)
        5 => (0.002, 0.998, 0.68, 0.72, 1.38), // Vivid (deep blacks, strong color contrast)
        _ => (0.005, 0.995, 0.60, 0.75, 1.35),
    };

    let low_threshold = (sampled_count as f32 * low_pct) as u32;
    let high_threshold = (sampled_count as f32 * high_pct) as u32;

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
    let target_mid: f32 = match preset {
        4 => 0.53, // Portrait slightly brightens midtones for flattering skin
        _ => 0.50,
    };
    let raw_gamma = (target_mid.ln()) / (normalized_mid.ln());
    let gamma = raw_gamma.clamp(gamma_min, gamma_max);

    let (min_r, max_r) = blend_range_weighted(r_low, r_high, y_low, y_high, channel_weight);
    let (min_g, max_g) = blend_range_weighted(g_low, g_high, y_low, y_high, channel_weight);
    let (min_b, max_b) = blend_range_weighted(b_low, b_high, y_low, y_high, channel_weight);

    // the brightest value each channel really has (the stretch's 99.5% point clips the rest)
    let top_of = |h: &[u32; 256]| h.iter().rposition(|&c| c > 0).unwrap_or(255) as f32;
    let lut_r = build_preset_lut(min_r, max_r, top_of(&hist_r), gamma, preset);
    let lut_g = build_preset_lut(min_g, max_g, top_of(&hist_g), gamma, preset);
    let lut_b = build_preset_lut(min_b, max_b, top_of(&hist_b), gamma, preset);

    for chunk in buffer.chunks_exact_mut(step) {
        let mut r = lut_r[chunk[0] as usize] as f32;
        let mut g = lut_g[chunk[1] as usize] as f32;
        let mut b = lut_b[chunk[2] as usize] as f32;

        match preset {
            1 => {
                // iAuto 2019: +10% mid-tone vibrancy (iBuiltinOutLutVibrancy 10) that skin tones
                // only half take (skinToneCorrectionOn, skinToneScale 1) - skin found by its
                // chroma (YCbCr), not by any warm colour (LAP-043)
                let y = 0.299 * r + 0.587 * g + 0.114 * b;
                let y_norm = (y / 255.0).clamp(0.0, 1.0);
                let max_c = r.max(g).max(b);
                let min_c = r.min(g).min(b);
                let sat = if max_c > 1.0 { (max_c - min_c) / max_c } else { 0.0 };
                let mid_weight = 4.0 * y_norm * (1.0 - y_norm);
                let skin_guard = if is_skin_tone(r, g, b) { 0.5 } else { 1.0 };
                let vib = 1.0 + 0.10 * (1.0 - sat).powi(2) * mid_weight * skin_guard;
                r = y + (r - y) * vib;
                g = y + (g - y) * vib;
                b = y + (b - y) * vib;
            }
            2 => {
                // iAuto Classic: subtle +5% natural vibrancy, faithful Picasa classic feel
                let y = 0.299 * r + 0.587 * g + 0.114 * b;
                let y_norm = (y / 255.0).clamp(0.0, 1.0);
                let max_c = r.max(g).max(b);
                let min_c = r.min(g).min(b);
                let sat = if max_c > 1.0 { (max_c - min_c) / max_c } else { 0.0 };
                let mid_weight = 4.0 * y_norm * (1.0 - y_norm);
                let vib = 1.0 + 0.05 * (1.0 - sat) * mid_weight;
                r = y + (r - y) * vib;
                g = y + (g - y) * vib;
                b = y + (b - y) * vib;
            }
            3 => {
                // The Outdoors: +18% midtone vibrancy + selective Sky (blue/cyan) & Foliage (green/gold) LUT boost
                let y = 0.299 * r + 0.587 * g + 0.114 * b;
                let y_norm = (y / 255.0).clamp(0.0, 1.0);
                let max_c = r.max(g).max(b);
                let min_c = r.min(g).min(b);
                let sat = if max_c > 1.0 { (max_c - min_c) / max_c } else { 0.0 };
                let mid_weight = 4.0 * y_norm * (1.0 - y_norm);
                let vib = 1.0 + 0.18 * (1.0 - sat).powi(2) * mid_weight;
                r = y + (r - y) * vib;
                g = y + (g - y) * vib;
                b = y + (b - y) * vib;

                // Sky enhancement (bBuiltinInLutSky): deepen blue skies & slightly recover sky luminance
                if b > r * 1.08 && b > g * 1.02 {
                    let sky_strength = ((b - r.max(g)) / 255.0).clamp(0.0, 0.35);
                    b = (b + 18.0 * sky_strength).min(255.0);
                    r = (r - 8.0 * sky_strength).max(0.0);
                }
                // Foliage enhancement (bBuiltinInLutFoliage): enrich greens and warm golden greens
                if g > b * 1.06 && g >= r * 0.92 {
                    let foliage_strength = ((g - b) / 255.0).clamp(0.0, 0.35);
                    g = (g + 16.0 * foliage_strength).min(255.0);
                    r = (r + 5.0 * foliage_strength).min(255.0);
                }
            }
            4 => {
                // Portrait / Beautify: flattering warm skin balance, soft contrast, protected skin saturation
                let y = 0.299 * r + 0.587 * g + 0.114 * b;
                let y_norm = (y / 255.0).clamp(0.0, 1.0);
                let max_c = r.max(g).max(b);
                let min_c = r.min(g).min(b);
                let sat = if max_c > 1.0 { (max_c - min_c) / max_c } else { 0.0 };
                let mid_weight = 4.0 * y_norm * (1.0 - y_norm);
                let is_skin = r > g && g > b && (r - b) > 12.0;
                let vib = if is_skin {
                    // Gently calm overly ruddy skin while adding subtle warmth
                    0.96 + 0.04 * (1.0 - sat)
                } else {
                    1.0 + 0.08 * (1.0 - sat) * mid_weight
                };
                r = y + (r - y) * vib + 2.5 * mid_weight;
                g = y + (g - y) * vib + 1.0 * mid_weight;
                b = y + (b - y) * vib - 1.5 * mid_weight;
            }
            5 => {
                // Vivid: high-depth pop (+24% midtone vibrancy + rich color contrast)
                let y = 0.299 * r + 0.587 * g + 0.114 * b;
                let y_norm = (y / 255.0).clamp(0.0, 1.0);
                let max_c = r.max(g).max(b);
                let min_c = r.min(g).min(b);
                let sat = if max_c > 1.0 { (max_c - min_c) / max_c } else { 0.0 };
                let mid_weight = 4.0 * y_norm * (1.0 - y_norm);
                let skin_guard = if r > g && g > b { 0.72 } else { 1.0 };
                let vib = 1.0 + 0.24 * (1.0 - sat * 0.7) * mid_weight * skin_guard;
                r = y + (r - y) * vib;
                g = y + (g - y) * vib;
                b = y + (b - y) * vib;
            }
            _ => {}
        }

        chunk[0] = (r + 0.5).clamp(0.0, 255.0) as u8;
        chunk[1] = (g + 0.5).clamp(0.0, 255.0) as u8;
        chunk[2] = (b + 0.5).clamp(0.0, 255.0) as u8;
    }
}

/// Skin chroma (the common YCbCr skin box: Cb 77..127, Cr 133..173), lit enough to judge.
fn is_skin_tone(r: f32, g: f32, b: f32) -> bool {
    let y = 0.299 * r + 0.587 * g + 0.114 * b;
    let cb = 128.0 - 0.168736 * r - 0.331264 * g + 0.5 * b;
    let cr = 128.0 + 0.5 * r - 0.418688 * g - 0.081312 * b;
    y > 40.0 && (77.0..=127.0).contains(&cb) && (133.0..=173.0).contains(&cr)
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

fn blend_range_weighted(c_low: u8, c_high: u8, y_low: u8, y_high: u8, c_weight: f32) -> (f32, f32) {
    let y_weight = 1.0 - c_weight;
    let low = c_weight * (c_low as f32) + y_weight * (y_low as f32);
    let high = c_weight * (c_high as f32) + y_weight * (y_high as f32);
    if high <= low {
        (low, low + 1.0)
    } else {
        (low, high)
    }
}

fn build_preset_lut(min_v: f32, max_v: f32, data_max: f32, gamma: f32, preset: u8) -> [u8; 256] {
    let mut lut = [0u8; 256];
    let range = (max_v - min_v).max(1.0);

    // S-curve steepness and blend factor per preset
    let (s_exp, s_blend) = match preset {
        1 => (0.15_f32, 0.65_f32), // iAuto 2019: Picasa finishing contrast (65)
        2 => (0.06_f32, 0.25_f32), // iAuto Classic: minimal S-curve, natural rolloff
        3 => (0.19_f32, 0.75_f32), // The Outdoors: crisp outdoor contrast (93)
        4 => (0.08_f32, 0.30_f32), // Portrait: low contrast (42) with shadow fill-light lift
        5 => (0.26_f32, 0.85_f32), // Vivid: high-depth positive contrast (110)
        _ => (0.15_f32, 0.65_f32),
    };

    let k = 4.0_f32.powf(s_exp);
    let s0 = 1.0 / (1.0 + (k * 0.5).exp());
    let s1 = 1.0 / (1.0 + (-k * 0.5).exp());
    let s_span = (s1 - s0).max(1e-5);

    // iAuto 2019 (LAP-043): highlight rolloff instead of a hard clip. The stretch sends the photo's
    // top 0.5% past white; they are kept in the last 4% of the range instead (white only at the
    // photo's own brightest value), and the tones from 94% up give that room by easing a little.
    // Everything below 94% is exactly as before, so a few specular pixels never dull the photo.
    let top = (data_max - min_v) / range;
    let rolloff = preset == 1 && top > 1.0 + 1e-3;
    const KNEE: f32 = 0.94;
    const TAIL: f32 = 0.96;

    for i in 0..256 {
        let raw = (i as f32 - min_v) / range;
        let norm = if !rolloff || raw <= KNEE {
            raw.clamp(0.0, 1.0)
        } else if raw <= 1.0 {
            KNEE + (raw - KNEE) * (TAIL - KNEE) / (1.0 - KNEE)
        } else {
            TAIL + (1.0 - TAIL) * ((raw - 1.0) / (top - 1.0)).min(1.0)
        };
        let mut g = norm.powf(gamma);

        if preset == 2 {
            // iAuto Classic: highlight deflash protection above 0.82
            if g > 0.82 {
                let t = (g - 0.82) / 0.18;
                g = 0.82 + 0.18 * (t - 0.12 * t * (1.0 - t));
            }
        } else if preset == 4 {
            // Portrait / Beautify: gentle Fill Light shadow lift y + 0.18 * y * (1 - y)^2
            g = (g + 0.18 * g * (1.0 - g).powi(2)).clamp(0.0, 1.0);
        } else if preset == 5 {
            // Vivid: subtle black-point anchor (iBlackEnhance = 12)
            g = (g - 0.03 * (1.0 - g).powi(2) * g).clamp(0.0, 1.0);
        }

        let s = (1.0 / (1.0 + (-k * (g - 0.5)).exp()) - s0) / s_span;
        let mapped = (s_blend * s + (1.0 - s_blend) * g).clamp(0.0, 1.0);
        lut[i] = (mapped * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
    }
    lut
}

/// Applies auto enhancement directly to encoded image bytes (JPEG, PNG, WebP, etc.).
/// Encodes the enhanced image back into its original format (or JPEG fallback).
pub fn apply_auto_enhance_to_bytes(data: &[u8]) -> Result<Vec<u8>, String> {
    apply_auto_enhance_to_bytes_for_file(data, None)
}

/// Reads the stored `auto_enhance_preset` (`1..=5`) from an `edits` JSON string.
pub fn extract_auto_enhance_preset(edits_json: &str) -> u8 {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(edits_json) {
        let p = val
            .get("auto_enhance_preset")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u8;
        normalize_preset(p)
    } else {
        1
    }
}

/// Queries SQLite for the active `auto_enhance_preset` (`1..=5`) of `file_id`.
pub fn get_file_auto_enhance_preset(file_id: i64) -> u8 {
    if file_id <= 0 {
        return 1;
    }
    let conn = match crate::t_sqlite::open_conn() {
        Ok(c) => c,
        Err(_) => return 1,
    };
    let edits_str: Option<String> = conn
        .query_row(
            "SELECT edits FROM afiles WHERE id = ?1",
            rusqlite::params![file_id],
            |row| row.get(0),
        )
        .ok()
        .flatten();

    edits_str
        .as_deref()
        .map(extract_auto_enhance_preset)
        .unwrap_or(1)
}

/// Apply the Magic Wand to a preview, using the file's stored `auto_enhance_preset`.
/// Always starts from the unedited base preview. No red-eye step: iAuto 2019 has none (its
/// "+ RE" variant does), and it turned reddish skin near the eyes green (LAP-043).
pub fn apply_auto_enhance_to_bytes_for_file(data: &[u8], file_id: Option<i64>) -> Result<Vec<u8>, String> {
    let mut img = image::load_from_memory(data).map_err(|e| e.to_string())?;
    let preset = file_id.map(get_file_auto_enhance_preset).unwrap_or(1);
    apply_auto_enhance_preset(&mut img, preset);
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
            // A red pupil is a small blob far redder than the skin around it. Reddish or sunburnt
            // skin also passes a plain "red dominates" test, so a pixel must beat the region's
            // median redness by a margin, must not be skin-toned, and the candidates must stay a
            // small part of the region; otherwise this is skin (or a red scarf), left alone.
            let inside: Vec<(u32, u32)> = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let dx = (x as f32 + 0.5 - cx) / rx;
                    let dy = (y as f32 + 0.5 - cy) / ry;
                    dx * dx + dy * dy <= 1.0
                }).collect();
            if inside.is_empty() { continue; }
            let mut redness: Vec<f32> = inside.iter().map(|&(x, y)| pupil_redness(rgb.get_pixel(x, y).0)).collect();
            let candidate = |p: [u8; 4], floor: f32| {
                let [r, g, b, _] = p;
                r > 60 && pupil_redness(p) >= floor && !is_skin_tone(r as f32, g as f32, b as f32)
            };
            redness.sort_by(|a, b| a.total_cmp(b));
            let floor = (redness[redness.len() / 2] + 0.12).max(0.55);
            let pupil: Vec<(u32, u32)> = inside.iter().copied().filter(|&(x, y)| candidate(rgb.get_pixel(x, y).0, floor)).collect();
            if pupil.len() * 100 > inside.len() * 35 { continue; }
            for (x, y) in pupil {
                // Neutral dark pupil: red drops to the brighter of green and blue, and green and
                // blue meet it, so no hue is left behind (red-to-(G+B)/2 turned warm skin green).
                let p = rgb.get_pixel_mut(x, y);
                let v = p.0[1].max(p.0[2]);
                p.0[0] = v; p.0[1] = v; p.0[2] = v;
            }
        }
    }
    *img = DynamicImage::ImageRgba8(rgb);
}

/// Red's share of the pixel's channel sum: about 0.45 for skin, 0.75+ for a flash-lit pupil.
fn pupil_redness(p: [u8; 4]) -> f32 {
    let sum = p[0] as f32 + p[1] as f32 + p[2] as f32;
    if sum <= 0.0 { 0.0 } else { p[0] as f32 / sum }
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
    fn red_eye_fixes_the_pupil_but_never_turns_reddish_skin_green() {
        // Warm, flushed skin (red clearly dominates: the old rule turned it to (80,85,75), green)
        // with a flash-lit red pupil in each eye region.
        let skin = Rgba([220, 88, 76, 255]);
        let mut img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 200, skin));
        let face = FaceRegion { x: 0.0, y: 0.0, width: 200.0, height: 200.0 };
        let pupils = [(64u32, 76u32), (136, 76)];
        for &(px, py) in &pupils {
            for y in py - 4..=py + 4 { for x in px - 4..=px + 4 {
                if (x as i32 - px as i32).pow(2) + (y as i32 - py as i32).pow(2) <= 16 {
                    img.as_mut_rgba8().unwrap().put_pixel(x, y, Rgba([210, 28, 34, 255]));
                }
            }}
        }
        remove_red_eye(&mut img, &[face], (200.0, 200.0));
        let out = img.to_rgba8();
        for &(px, py) in &pupils {
            let p = out.get_pixel(px, py).0;
            assert!(p[0] == p[1] && p[1] == p[2] && p[0] < 60, "pupil must be dark neutral, got {p:?}");
        }
        let green = out.pixels().filter(|p| p[1] > p[0]).count();
        assert_eq!(green, 0, "no pixel may end up greener than red");
        let changed_skin = out.enumerate_pixels().filter(|(x, y, p)| {
            **p != skin && !pupils.iter().any(|&(px, py)| (*x as i32 - px as i32).pow(2) + (*y as i32 - py as i32).pow(2) <= 16)
        }).count();
        assert_eq!(changed_skin, 0, "skin around the eyes must not change");

        // Flushed skin with no red pupil at all: nothing changes.
        let mut plain = DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 200, Rgba([205, 92, 84, 255])));
        let before = plain.to_rgba8();
        remove_red_eye(&mut plain, &[FaceRegion { x: 0.0, y: 0.0, width: 200.0, height: 200.0 }], (200.0, 200.0));
        assert_eq!(plain.to_rgba8(), before);
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

        assert!(min_val < 30, "Expected min_val < 30 after auto-enhance, got {}", min_val);
        assert!(max_val > 220, "Expected max_val > 220 after auto-enhance, got {}", max_val);
    }

    #[test]
    fn test_all_five_iauto_presets_produce_distinct_non_additive_outputs() {
        let mut base = RgbImage::new(60, 60);
        for (x, y, pixel) in base.enumerate_pixels_mut() {
            let r = (70 + (x * 2) % 120) as u8;
            let g = (80 + (y * 2) % 110) as u8;
            let b = (90 + ((x + y) * 2) % 130) as u8;
            *pixel = Rgb([r, g, b]);
        }
        let mut outputs = Vec::new();
        for preset in 1..=PRESET_COUNT {
            let mut img = DynamicImage::ImageRgb8(base.clone());
            apply_auto_enhance_preset(&mut img, preset);
            outputs.push(img.into_rgb8().into_raw());
        }
        for i in 0..outputs.len() {
            for j in (i + 1)..outputs.len() {
                assert_ne!(
                    outputs[i], outputs[j],
                    "Preset {} ({}) and preset {} ({}) should produce distinct outputs",
                    i + 1,
                    preset_name((i + 1) as u8),
                    j + 1,
                    preset_name((j + 1) as u8)
                );
            }
        }
    }

    fn luma(p: &Rgb<u8>) -> f32 { 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32 }

    #[test]
    fn iauto2019_lifts_a_dark_scene_and_keeps_its_blacks() {
        // a dim, low-key frame: tones 4..90, most of it in the shadows
        let src = RgbImage::from_fn(200, 100, |x, y| { let v = (4 + (x * x) / 460 + y / 10) as u8; Rgb([v, v.saturating_sub(2), v.saturating_sub(4)]) });
        let mut img = DynamicImage::ImageRgb8(src.clone());
        apply_auto_enhance(&mut img);
        let out = img.into_rgb8();
        let mean = |i: &RgbImage| i.pixels().map(luma).sum::<f32>() / (i.width() * i.height()) as f32;
        assert!(mean(&out) > mean(&src) * 1.6, "dark scene mean {} -> {}", mean(&src), mean(&out));
        assert!(luma(out.get_pixel(0, 0)) < 8.0, "black lifted to {:?}", out.get_pixel(0, 0));
        // tones keep their order across the frame
        let row: Vec<f32> = (0..200).map(|x| luma(out.get_pixel(x, 50))).collect();
        assert!(row.windows(2).all(|w| w[1] >= w[0] - 0.6), "tone order broken");
    }

    #[test]
    fn iauto2019_rolls_highlights_off_instead_of_clipping_them() {
        // mostly mid tones, with a few bright details above the 99.5% point
        let src = RgbImage::from_fn(400, 100, |x, y| {
            if y == 0 && x < 160 { let v = 226 + (x / 6) as u8; Rgb([v, v, v]) } else { let v = (60 + (x + y) % 150) as u8; Rgb([v, v, v]) }
        });
        let mut img = DynamicImage::ImageRgb8(src.clone());
        apply_auto_enhance(&mut img);
        let out = img.into_rgb8();
        let bright: std::collections::BTreeSet<u8> = (0..160).map(|x| out.get_pixel(x, 0)[0]).collect();
        assert!(bright.len() >= 6, "bright details merged into {} values: {bright:?}", bright.len());
        let clipped = (0..160).filter(|&x| out.get_pixel(x, 0)[0] == 255).count();
        // a hard clip at the 99.5% point whitens all 160; the rolloff only the very brightest few
        assert!(clipped < 40, "{clipped} of 160 bright details clipped to white");
        // and the preset still stretches the mid tones
        assert!(out.get_pixel(0, 50)[0] < src.get_pixel(0, 50)[0]);
    }

    #[test]
    fn iauto2019_vibrancy_spares_skin_tones() {
        let chroma = |p: &Rgb<u8>| p[0].max(p[1]).max(p[2]) as f32 - p[0].min(p[1]).min(p[2]) as f32;
        // half skin (a typical face tone), half a muted teal of the same lightness
        let skin = Rgb([200u8, 150, 125]);
        let teal = Rgb([110u8, 165, 160]);
        let mut src = RgbImage::from_fn(100, 100, |x, _| if x < 50 { skin } else { teal });
        // a black and a white corner so the stretch leaves both colours where they are
        src.put_pixel(0, 0, Rgb([0, 0, 0]));
        src.put_pixel(99, 99, Rgb([255, 255, 255]));
        assert!(is_skin_tone(200.0, 150.0, 125.0) && !is_skin_tone(110.0, 165.0, 160.0));
        assert!(!is_skin_tone(230.0, 120.0, 20.0), "a saturated orange is not skin");
        let mut img = DynamicImage::ImageRgb8(src.clone());
        apply_auto_enhance(&mut img);
        let out = img.into_rgb8();
        let gain = |x: u32| chroma(out.get_pixel(x, 50)) / chroma(src.get_pixel(x, 50));
        assert!(gain(10) < gain(80), "skin chroma x{} vs teal x{}", gain(10), gain(80));
    }

    #[test]
    fn test_apply_auto_enhance_to_bytes() {
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
