//! Picasa tool recipes applied to an in-memory image, never to source files.
use base64::Engine;
use image::{DynamicImage, GenericImageView, Rgb};
use serde_json::Value;

fn number(v: &Value, key: &str, default: f32) -> f32 {
    v[key].as_f64().unwrap_or(default as f64) as f32
}
fn channel(v: f32) -> u8 {
    v.clamp(0.0, 255.0).round() as u8
}
#[cfg(test)]
pub(crate) fn apply(img: DynamicImage, tools: &Value) -> Result<DynamicImage, String> {
    apply_for(img, tools, None)
}
/// The tools applied to `photo`'s pixels; `photo` locates the masks of local effects.
pub(crate) fn apply_for(mut img: DynamicImage, tools: &Value, photo: Option<&std::path::Path>) -> Result<DynamicImage, String> {
    let (w, h) = img.dimensions();
    if let Some(patches) = tools["retouch"].as_array() {
        let mut pixels = img.to_rgb8();
        for patch in patches {
            let old = pixels.clone();
            let sx = number(patch, "sx", 0.5) * w as f32;
            let sy = number(patch, "sy", 0.5) * h as f32;
            let tx = number(patch, "tx", 0.5) * w as f32;
            let ty = number(patch, "ty", 0.5) * h as f32;
            let r = (number(patch, "radius", 0.02) * w.min(h) as f32)
                .clamp(1.0, (w.min(h) as f32 / 3.0).max(1.0));
            for y in (ty - r).max(0.0) as u32..((ty + r).ceil() as u32).min(h) {
                for x in (tx - r).max(0.0) as u32..((tx + r).ceil() as u32).min(w) {
                    let d = ((x as f32 - tx).powi(2) + (y as f32 - ty).powi(2)).sqrt() / r;
                    if d >= 1.0 {
                        continue;
                    }
                    let px = (sx + x as f32 - tx).clamp(0.0, (w - 1) as f32) as u32;
                    let py = (sy + y as f32 - ty).clamp(0.0, (h - 1) as f32) as u32;
                    let source = old.get_pixel(px, py);
                    let target = old.get_pixel(x, y);
                    let alpha = ((1.0 - d) * 4.0).min(1.0);
                    pixels.put_pixel(
                        x,
                        y,
                        Rgb(std::array::from_fn(|c| {
                            channel(target[c] as f32 * (1.0 - alpha) + source[c] as f32 * alpha)
                        })),
                    );
                }
            }
        }
        img = DynamicImage::ImageRgb8(pixels);
    }
    if let Some(layers) = tools["text"].as_array() {
        for layer in layers {
            let data = base64::engine::general_purpose::STANDARD
                .decode(layer["png"].as_str().ok_or("Missing text layer")?)
                .map_err(|e| e.to_string())?;
            let overlay = image::load_from_memory(&data).map_err(|e| e.to_string())?;
            let width = (number(layer, "width", 0.1) * w as f32)
                .round()
                .max(1.0)
                .min(w as f32) as u32;
            let height = (number(layer, "height", 0.1) * h as f32)
                .round()
                .max(1.0)
                .min(h as f32) as u32;
            let overlay =
                overlay.resize_exact(width, height, image::imageops::FilterType::Lanczos3);
            image::imageops::overlay(
                &mut img,
                &overlay,
                (number(layer, "x", 0.1) * w as f32) as i64,
                (number(layer, "y", 0.1) * h as f32) as i64,
            );
        }
    }
    // Automatic operations store bounded percentile transforms, shared by every render path.
    for key in ["autoContrast", "autoColor"] {
        if let Some(levels) = tools[key].as_array() {
            let transform: [(f64, f64); 3] = std::array::from_fn(|c| {
                let low = levels.get(c).and_then(|v| v[0].as_f64()).unwrap_or(0.0).clamp(0.0, 12.0);
                let gain = levels.get(c).and_then(|v| v[1].as_f64()).unwrap_or(1.0)
                    .clamp(1.0, if key == "autoColor" { 1.08 } else { 1.25 });
                (low, gain)
            });
            let mut pixels = img.to_rgb8();
            for p in pixels.pixels_mut() {
                for c in 0..3 {
                    let (low, gain) = transform[c];
                    p[c] = channel(((p[c] as f64 - low) * gain) as f32);
                }
            }
            img = DynamicImage::ImageRgb8(pixels);
        }
    }
    let fill = number(tools, "fillLight", 0.0) / 100.0;
    let highlights = number(tools, "highlights", 0.0) / 100.0;
    let shadows = number(tools, "shadows", 0.0) / 100.0;
    let temp = number(tools, "temperature", 0.0) / 100.0;
    let neutral = tools["neutral"].as_array();
    if fill != 0.0 || highlights != 0.0 || shadows != 0.0 || temp != 0.0 || neutral.is_some() {
        let mut pixels = img.to_rgb8();
        let gains: [f32; 3] = std::array::from_fn(|c| {
            neutral
                .and_then(|n| n.get(c))
                .and_then(Value::as_f64)
                .unwrap_or(1.0) as f32
        });
        for p in pixels.pixels_mut() {
            // Fill Light first, with its own colour rule (LAP-108), then Highlights and Shadows.
            let mut rgb: [f32; 3] = std::array::from_fn(|c| p[c] as f32);
            if fill != 0.0 {
                rgb = fill_light(rgb, fill);
            }
            let l = (rgb[0] * 0.299 + rgb[1] * 0.587 + rgb[2] * 0.114) / 255.0;
            // LAP-048: the three tone sliders move luminance along a curve that stays inside 0..1
            // and keeps tones in order, and the colour follows as a gain (hue and saturation kept),
            // instead of adding the same amount to every channel (washed-out shadows, highlights
            // clipped to white, blacks crushed).
            let toned = match (highlights, shadows) {
                (0.0, 0.0) => rgb,
                _ => tone_gain(rgb, l, tone_curve(l, 0.0, highlights, shadows)),
            };
            for c in 0..3 {
                p[c] = channel(
                    toned[c] * gains[c]
                        + if c == 0 {
                            temp * 30.0
                        } else if c == 2 {
                            -temp * 30.0
                        } else {
                            0.0
                        },
                );
            }
        }
        img = DynamicImage::ImageRgb8(pixels);
    }
    if let Some(effects) = tools["effects"].as_array() {
        for effect in effects {
            img = match effect["mask"].as_str() {
                Some(id) => crate::t_edit_mask::apply(img.clone(), apply_effect(img, effect)?, photo, id),
                None => apply_effect(img, effect)?,
            };
        }
    }
    // Local curves (LAP-107) before the global curves: the editor draws its live global-curve draft
    // over a base render that already has them.
    img = crate::t_edit_local::apply(img, &tools["local"], photo)?;
    if let Some(curves) = tools.get("curves") {
        let mut tables = [[0u8; 256]; 4];
        for (i, name) in ["rgb", "red", "green", "blue"].iter().enumerate() {
            let points: Vec<[f64;2]> = match curves.get(*name) {
                Some(points) => serde_json::from_value(points.clone()).map_err(|e|e.to_string())?,
                None => vec![[0.0,0.0],[1.0,1.0]],
            };
            tables[i] = crate::t_curves::lut(&points)?;
        }
        let mut pixels=img.to_rgb8();
        for pixel in pixels.pixels_mut() { *pixel=Rgb(crate::t_curves::map(pixel.0,&tables)); }
        img=DynamicImage::ImageRgb8(pixels);
    }
    Ok(img)
}

/// The tone sliders' luminance curve (0..1 in, 0..1 out, monotonic, identity at all-zero).
/// Fill Light is a gamma lift of the whole tonal range, strongest in the shadows, black and white
/// fixed; Highlights brightens bright tones with a shoulder that only approaches white (negative:
/// compresses the top 20% as before); Shadows darkens dark tones and fades into black instead of
/// crushing it.
fn tone_curve(l: f32, fill: f32, highlights: f32, shadows: f32) -> f32 {
    let mut t = l.clamp(0.0, 1.0);
    if fill != 0.0 {
        // LAP-108: t^(1 - 0.76 fill), fitted to the reference editor's Fill Light measured on six
        // photos at every 10 %: within 2/255 of its luminance up to 50 %, 6/255 on average at 100 %.
        t = t.powf(1.0 - FILL_EXP * fill.clamp(0.0, 1.0));
    }
    if highlights > 0.0 {
        t += highlights.min(1.0) * 0.35 * t * t * (1.0 - t) * 2.0;
    } else if highlights < 0.0 {
        // Compress only the top 20%, joining with slope 1 at 80%.
        let top = (t - 0.8).max(0.0);
        let strength = -highlights * 4.0;
        t += top / (1.0 + strength * top / 0.2) - top;
    }
    if shadows != 0.0 {
        // darkest tones lose a share of themselves (never below 0)
        t -= shadows.clamp(0.0, 1.0) * 0.35 * t * (1.0 - t).powi(2) * 1.5;
    }
    t.clamp(0.0, 1.0)
}

const FILL_EXP: f32 = 0.76;

/// Fill Light on one pixel (LAP-108). Luminance follows `tone_curve`'s gamma lift; the colour
/// difference from grey grows by the luminance gain times sqrt(1 - 0.76 fill), the share of a fully
/// colour-keeping lift the reference editor keeps (measured 0.90 / 0.78 / 0.46 at 25 / 50 / 100 %),
/// at most 3x so near-black noise does not turn to colour, and is eased towards grey only as far as
/// a channel would pass 255.
fn fill_light(rgb: [f32; 3], fill: f32) -> [f32; 3] {
    let l = (rgb[0] * 0.299 + rgb[1] * 0.587 + rgb[2] * 0.114) / 255.0;
    if l <= 0.0 {
        return rgb;
    }
    let fill = fill.clamp(0.0, 1.0);
    let tone = tone_curve(l, fill, 0.0, 0.0);
    let keep = (tone / l * (1.0 - FILL_EXP * fill).sqrt()).min(3.0);
    let (l255, t255) = (l * 255.0, tone * 255.0);
    let mut out = rgb.map(|v| t255 + (v - l255) * keep);
    let top = out.iter().cloned().fold(0.0f32, f32::max);
    if top > 255.0 {
        let room = (255.0 - t255).max(0.0) / (top - t255).max(1e-3);
        out = out.map(|v| t255 + (v - t255) * room);
    }
    out
}

/// Colour at a new luminance: luminance moves to `tone`, the colour difference from grey (chroma)
/// grows only by the square root of the gain, at most 1.5x - full gain on chroma turns lifted
/// shadows neon, none washes them out. Never past 255 (scaled back as a whole).
pub(crate) fn tone_gain(rgb: [f32; 3], l: f32, tone: f32) -> [f32; 3] {
    let (l255, t255) = (l * 255.0, tone * 255.0);
    let chroma = if l255 > 4.0 { (tone / l).sqrt().clamp(0.0, 1.5) } else { 1.0 };
    let mut out = rgb.map(|v| t255 + (v - l255) * chroma);
    let top = out.iter().cloned().fold(0.0f32, f32::max);
    if top > 255.0 {
        // pull the colour towards its luminance until the brightest channel fits
        let room = (255.0 - t255).max(0.0) / (top - t255).max(1e-3);
        out = out.map(|v| t255 + (v - t255) * room);
    }
    out
}

/// Picasa's Sharpen (glimmer::SharpenImageOperation, 0x00b8cba0, constants read from
/// Picasa3.exe): amount 0..100 clamped, k = amount / -100, a 3x3 kernel with every neighbour k and
/// the centre 1 - 8k. LapCasa's amount is 0..1, so k = -amount.
fn picasa_sharpen(img: &DynamicImage, amount: f32) -> DynamicImage {
    let src = img.to_rgb8();
    let (w, h) = src.dimensions();
    let k = -amount.clamp(0.0, 1.0);
    let centre = 1.0 - 8.0 * k;
    let mut out = src.clone();
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0f32; 3];
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let sx = (x as i32 + dx).clamp(0, w as i32 - 1) as u32;
                    let sy = (y as i32 + dy).clamp(0, h as i32 - 1) as u32;
                    let weight = if dx == 0 && dy == 0 { centre } else { k };
                    let p = src.get_pixel(sx, sy);
                    for c in 0..3 { acc[c] += p[c] as f32 * weight; }
                }
            }
            out.put_pixel(x, y, Rgb(acc.map(channel)));
        }
    }
    DynamicImage::ImageRgb8(out)
}

// ---- Picasa 3.9's extra effects (LAP-049) ----
// Each follows that effect's Glimmer graph in Picasa's runtime/filterdesc.xml: the same operations
// in the same order with the same constants and slider ranges. The Glimmer primitives themselves
// (Flash-style inner glow and blur, colour matrix, AutoFix, IR) are approximations, so the look is
// Picasa's recipe, not a pixel match. Amount is the effect's main Picasa slider over its range (or
// 1 - Fade for the effects whose only slider is Fade).

/// Pixel sizes Picasa gives as plain numbers, scaled to this image: they are pixels of its working
/// preview, about 1600 px across, so a preview and a full-size export look the same.
fn picasa_px(m: f32, v: f32) -> f32 {
    v * m / 1600.0
}
fn erf(x: f32) -> f32 {
    // Abramowitz and Stegun 7.1.26 (|error| < 1.5e-7)
    let t = 1.0 / (1.0 + 0.3275911 * x.abs());
    let y = 1.0 - ((((1.061405429 * t - 1.453152027) * t + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * (-x * x).exp();
    if x < 0.0 { -y } else { y }
}
/// Gaussian-blurred coverage of the frame [0, len) at x: 1 well inside, 1/2 on the edge.
fn edge_cover(x: f32, len: f32, sigma: f32) -> f32 {
    if sigma <= 0.0 { return 1.0; }
    let s = sigma * std::f32::consts::SQRT_2;
    (0.5 * (erf(x / s) + erf((len - x) / s))).clamp(0.0, 1.0)
}
/// Glimmer GlowImageOperation with innerglow=true, knockout=false (Flash's inner glow): `color`
/// drawn in from the frame's edges at alpha = strength x (1 - the frame's blurred coverage).
fn inner_glow(px: &mut image::RgbImage, color: [f32; 3], xblur: f32, yblur: f32, strength: f32) {
    let (w, h) = px.dimensions();
    let cols: Vec<f32> = (0..w).map(|x| edge_cover(x as f32 + 0.5, w as f32, xblur / 2.0)).collect();
    let rows: Vec<f32> = (0..h).map(|y| edge_cover(y as f32 + 0.5, h as f32, yblur / 2.0)).collect();
    for (x, y, p) in px.enumerate_pixels_mut() {
        let a = (strength * (1.0 - cols[x as usize] * rows[y as usize])).clamp(0.0, 1.0);
        *p = Rgb(std::array::from_fn(|c| channel(p[c] as f32 * (1.0 - a) + color[c] * a)));
    }
}
/// Glimmer CircularGradientImageMask: `inner` inside r0, `outer` beyond r1, linear between.
fn circle_mask(x: f32, y: f32, centre: (f32, f32), r0: f32, r1: f32, inner: f32, outer: f32) -> f32 {
    let d = ((x - centre.0).powi(2) + (y - centre.1).powi(2)).sqrt();
    let t = if r1 <= r0 { (d >= r0) as u8 as f32 } else { ((d - r0) / (r1 - r0)).clamp(0.0, 1.0) };
    inner + (outer - inner) * t
}
/// Glimmer BlurImageOperation (Flash blur sizes, quality 3: close to a Gaussian of sigma size/2;
/// at least 2 px, so a scaled-down size still blurs a small image).
fn glimmer_blur(px: &image::RgbImage, size: f32) -> image::RgbImage {
    if size <= 0.0 { return px.clone(); }
    image::imageops::fast_blur(px, size.max(2.0) / 2.0)
}
/// `top` over `base` where `alpha(x, y)` says how much of `top` shows.
fn mix_by(base: &mut image::RgbImage, top: &image::RgbImage, alpha: impl Fn(u32, u32) -> f32) {
    for (x, y, p) in base.enumerate_pixels_mut() {
        let a = alpha(x, y).clamp(0.0, 1.0);
        if a <= 0.0 { continue; }
        let t = top.get_pixel(x, y);
        *p = Rgb(std::array::from_fn(|c| channel(p[c] as f32 * (1.0 - a) + t[c] as f32 * a)));
    }
}
/// Glimmer SimpleColorMatrixImageOperation: Saturation, Contrast, Brightness, each -100..100.
fn simple_color_matrix(px: &mut image::RgbImage, saturation: f32, contrast: f32, brightness: f32) {
    let (s, k) = (1.0 + saturation / 100.0, 1.0 + contrast / 100.0);
    for p in px.pixels_mut() {
        let v: [f32; 3] = std::array::from_fn(|c| p[c] as f32);
        let g = 0.299 * v[0] + 0.587 * v[1] + 0.114 * v[2];
        *p = Rgb(std::array::from_fn(|c| channel(((g + (v[c] - g) * s) - 128.0) * k + 128.0 + brightness * 2.55)));
    }
}
/// Glimmer AutoFixImageOperation, as an auto-levels stretch of luminance (0.5% clipped each end).
fn auto_fix(px: &mut image::RgbImage) {
    let mut hist = [0u32; 256];
    for p in px.pixels() {
        hist[((p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000) as usize] += 1;
    }
    let cut = (px.width() * px.height()) / 200;
    let (mut acc, mut lo, mut hi) = (0, 0usize, 255usize);
    for (i, c) in hist.iter().enumerate() { acc += c; if acc > cut { lo = i; break; } }
    acc = 0;
    for (i, c) in hist.iter().enumerate().rev() { acc += c; if acc > cut { hi = i; break; } }
    if hi <= lo + 8 { return; }
    let scale = 255.0 / (hi - lo) as f32;
    for p in px.pixels_mut() {
        *p = Rgb(std::array::from_fn(|c| channel((p[c] as f32 - lo as f32) * scale)));
    }
}
/// Repeatable grey noise in 0..1 (the same photo always gets the same grain).
fn grain(x: u32, y: u32, seed: u32) -> f32 {
    let h = (x.wrapping_mul(374761393) ^ y.wrapping_mul(668265263) ^ seed.wrapping_mul(2246822519)).wrapping_mul(1274126177);
    (h >> 24) as f32 / 255.0
}
/// Glimmer NoiseImageOperation (grey, low..high of 255) multiplied in at `alpha`.
fn noise_multiply(px: &mut image::RgbImage, low: f32, high: f32, alpha: f32, seed: u32) {
    for (x, y, p) in px.enumerate_pixels_mut() {
        let n = (low + (high - low) * grain(x, y, seed)) / 255.0;
        *p = Rgb(std::array::from_fn(|c| channel(p[c] as f32 * (1.0 - alpha + alpha * n))));
    }
}
/// Glimmer AdjustCurvesImageOperation curves, given as Picasa's 0..255 points.
fn curve_table(points: &[(f64, f64)]) -> Result<[u8; 256], String> {
    let points: Vec<[f64; 2]> = points.iter().map(|&(x, y)| [x / 255.0, y / 255.0]).collect();
    crate::t_curves::lut(&points)
}
/// HSV (h in degrees, s and v 0..1) to RGB 0..255.
fn hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 { 0 => (c, x, 0.0), 1 => (x, c, 0.0), 2 => (0.0, c, x), 3 => (0.0, x, c), 4 => (x, 0.0, c), _ => (c, 0.0, x) };
    [(r + v - c) * 255.0, (g + v - c) * 255.0, (b + v - c) * 255.0]
}

const PICASA39_EFFECTS: [&str; 11] =
    ["Vignette", "Pixelate", "Focal Zoom", "Lomo-ish", "Holga-ish", "HDR-ish", "Cinemascope", "Orton-ish", "Infrared", "Heat Map", "Cross Process"];

fn picasa39_effect(img: &DynamicImage, name: &str, amount: f32, effect: &Value) -> Result<DynamicImage, String> {
    let orig = img.to_rgb8();
    let (w, h) = orig.dimensions();
    let (wf, hf) = (w as f32, h as f32);
    let m = wf.max(hf);
    let mut px = orig.clone();
    match name {
        // Darken the edges: a black inner glow, blur 0..50 (Amount; Picasa's default 35), strength 1.4.
        "Vignette" => {
            let blur = 50.0 * amount * 0.02 * m / 4.0;
            inner_glow(&mut px, [0.0; 3], blur, blur, 1.4);
        }
        // Blocky and low-res: shrink by the pixel size 2..150 (Amount, squared so the useful small
        // sizes get most of the slider; Picasa's default 20 is Amount 37%), then grow back unsmoothed.
        "Pixelate" => {
            let size = picasa_px(m, 2.0 + 148.0 * amount * amount).round().max(2.0) as u32;
            for by in (0..h).step_by(size as usize) {
                for bx in (0..w).step_by(size as usize) {
                    let (x1, y1) = ((bx + size).min(w), (by + size).min(h));
                    let mut sum = [0u64; 3];
                    for y in by..y1 { for x in bx..x1 { let p = orig.get_pixel(x, y); for c in 0..3 { sum[c] += p[c] as u64; } } }
                    let n = ((x1 - bx) * (y1 - by)) as u64;
                    let avg = Rgb(std::array::from_fn(|c| (sum[c] / n) as u8));
                    for y in by..y1 { for x in bx..x1 { px.put_pixel(x, y, avg); } }
                }
            }
        }
        // Zoom everything outside a central area: a radial (zoom) blur towards the chosen centre,
        // strength 1..100 (Amount), masked clear inside Picasa's default focal circle (radius a
        // quarter of the short side, hardness 50).
        "Focal Zoom" => {
            let centre = (number(effect, "cx", 0.5) * wf, number(effect, "cy", 0.5) * hf);
            let impact = 1.0 + 99.0 * amount;
            let r = wf.min(hf) / 4.0;
            let (r0, r1) = (r * 50.0 / 101.0, r * (2.0 - 50.0 / 101.0));
            let spread = impact / 100.0 * 0.25;
            const SAMPLES: u32 = 24;
            for (x, y, p) in px.enumerate_pixels_mut() {
                let a = circle_mask(x as f32, y as f32, centre, r0, r1, 0.0, 1.0);
                if a <= 0.0 { continue; }
                let mut sum = [0.0f32; 3];
                for k in 0..SAMPLES {
                    let s = 1.0 - spread * k as f32 / (SAMPLES - 1) as f32;
                    let sx = (centre.0 + (x as f32 - centre.0) * s).clamp(0.0, wf - 1.0) as u32;
                    let sy = (centre.1 + (y as f32 - centre.1) * s).clamp(0.0, hf - 1.0) as u32;
                    let q = orig.get_pixel(sx, sy);
                    for c in 0..3 { sum[c] += q[c] as f32; }
                }
                let o = orig.get_pixel(x, y);
                *p = Rgb(std::array::from_fn(|c| channel(o[c] as f32 * (1.0 - a) + sum[c] / SAMPLES as f32 * a)));
            }
        }
        // Lomo: black inner glow (blur 0.35 x the long side, strength 1.1), blur 20 outside a circle
        // whose softness is Amount (Picasa's Blur 0..100, default 50), then Saturation 20, Contrast 35,
        // Brightness 5.
        "Lomo-ish" => {
            let glow = 35.0 * 0.02 * m / 2.0;
            inner_glow(&mut px, [0.0; 3], glow, glow, 1.1);
            let blurred = glimmer_blur(&px, picasa_px(m, 20.0));
            let (r0, r1) = (m / 2.0 * 0.5, m / 2.0 * (2.0 - amount));
            mix_by(&mut px, &blurred, |x, y| circle_mask(x as f32, y as f32, (wf / 2.0, hf / 2.0), r0, r1, 0.0, 1.0));
            simple_color_matrix(&mut px, 20.0, 35.0, 5.0);
        }
        // Holga: AutoFix, black inner glow (0.5 / 0.4 x half the long side, strength 1.4), blur 18 x 20
        // outside a circle whose softness is Amount (Picasa's Blur, default 70), B&W through a
        // 0xff6666 filter, Contrast 25, then grain 30 multiplied in at 60%.
        "Holga-ish" => {
            auto_fix(&mut px);
            inner_glow(&mut px, [0.0; 3], 0.5 * m / 2.0, 0.4 * m / 2.0, 1.4);
            let blurred = glimmer_blur(&px, picasa_px(m, 19.0));
            let (r0, r1) = (m / 2.0 * 0.9, m / 2.0 * (2.0 - amount));
            mix_by(&mut px, &blurred, |x, y| circle_mask(x as f32, y as f32, (wf / 2.0, hf / 2.0), r0, r1, 0.0, 1.0));
            for p in px.pixels_mut() {
                let v = (p[0] as f32 * 1.0 + p[1] as f32 * 0.4 + p[2] as f32 * 0.4) / 1.8;
                *p = Rgb([channel(v); 3]);
            }
            simple_color_matrix(&mut px, 0.0, 25.0, 0.0);
            noise_multiply(&mut px, 255.0 - (30.0f32 / 200.0 * 255.0).round(), 255.0, 0.6, 5);
        }
        // HDR-ish: Glimmer LocalContrast, radius 20, strength 1..7 (Amount; Picasa's default 3 is
        // Amount 33%): detail above a blur of the photo is multiplied by the strength.
        "HDR-ish" => {
            let strength = 1.0 + 6.0 * amount;
            let blurred = glimmer_blur(&orig, picasa_px(m, 20.0));
            for (x, y, p) in px.enumerate_pixels_mut() {
                let (o, b) = (orig.get_pixel(x, y), blurred.get_pixel(x, y));
                *p = Rgb(std::array::from_fn(|c| channel(b[c] as f32 + (o[c] as f32 - b[c] as f32) * strength)));
            }
        }
        // Cinemascope: the middle band at 1.7:1, graded (AutoFix, Saturation -25, Picasa's master /
        // red / blue curves, grain 225..245 multiplied in; Amount is how much of the grade shows),
        // squeezed to 95% height between black bars 15% of the band tall. Picasa resizes the photo to
        // that frame; here the frame is fitted into the photo's own size, the rest black, so the
        // photo's size never changes.
        "Cinemascope" => {
            let band_h = (wf / 1.7).round().min(hf).max(1.0) as u32;
            let y0 = (h - band_h) / 2;
            let mut band = image::imageops::crop_imm(&orig, 0, y0, w, band_h).to_image();
            let mut graded = band.clone();
            auto_fix(&mut graded);
            simple_color_matrix(&mut graded, -25.0, 0.0, 0.0);
            let tables = [
                curve_table(&[(0.0, 0.0), (29.0, 19.0), (110.0, 150.0), (233.0, 245.0), (255.0, 255.0)])?,
                curve_table(&[(0.0, 0.0), (111.0, 141.0), (255.0, 255.0)])?,
                curve_table(&[(0.0, 0.0), (255.0, 255.0)])?,
                curve_table(&[(0.0, 0.0), (136.0, 121.0), (255.0, 255.0)])?,
            ];
            for p in graded.pixels_mut() { *p = Rgb(crate::t_curves::map(p.0, &tables)); }
            noise_multiply(&mut graded, 225.0, 245.0, 1.0, 0);
            mix_by(&mut band, &graded, |_, _| amount);
            let squeezed_h = ((band_h as f32 * 0.95).round() as u32).max(1);
            let bar = (band_h as f32 * 0.15).round() as u32;
            let frame_h = squeezed_h + 2 * bar;
            let fit = (hf / frame_h as f32).min(1.0);
            let (fw, fh) = (((wf * fit).round() as u32).max(1), ((squeezed_h as f32 * fit).round() as u32).max(1));
            let content = image::imageops::resize(&band, fw, fh, image::imageops::FilterType::Triangle);
            px = image::RgbImage::from_pixel(w, h, Rgb([0, 0, 0]));
            image::imageops::overlay(&mut px, &content, ((w - fw) / 2) as i64, ((h - fh) / 2) as i64);
        }
        // Orton-ish: the photo blurred by Bloom 0..50 (Amount; Picasa's default 25) overlay-blended on
        // itself (Brightness at its default 50 leaves the curve flat).
        "Orton-ish" => {
            let blurred = glimmer_blur(&orig, picasa_px(m, 50.0 * amount));
            for (x, y, p) in px.enumerate_pixels_mut() {
                let (o, b) = (orig.get_pixel(x, y), blurred.get_pixel(x, y));
                *p = Rgb(std::array::from_fn(|c| {
                    let (a, t) = (o[c] as f32 / 255.0, b[c] as f32 / 255.0);
                    channel(255.0 * if a < 0.5 { 2.0 * a * t } else { 1.0 - 2.0 * (1.0 - a) * (1.0 - t) })
                }));
            }
        }
        // Infrared Film: Glimmer IRImageOperation with Picasa's greenglow 5, greenglowalpha 0.25,
        // redweight -0.5: black and white weighted to green (foliage turns near white, a neutral grey
        // keeps its tone), its blur screened back in at 25% (the glow lifts everything a little).
        // Amount is 1 - Fade.
        "Infrared" => {
            let mono = image::GrayImage::from_fn(w, h, |x, y| {
                let p = orig.get_pixel(x, y);
                image::Luma([channel(-0.5 * p[0] as f32 + 1.5 * p[1] as f32)])
            });
            let glow = image::imageops::fast_blur(&mono, picasa_px(m, 5.0).max(0.5));
            for (x, y, p) in px.enumerate_pixels_mut() {
                let (v, g) = (mono.get_pixel(x, y)[0] as f32, glow.get_pixel(x, y)[0] as f32);
                let screen = 255.0 - (255.0 - v) * (255.0 - g) / 255.0;
                let ir = v * 0.75 + screen * 0.25;
                let o = orig.get_pixel(x, y);
                *p = Rgb(std::array::from_fn(|c| channel(o[c] as f32 * (1.0 - amount) + ir * amount)));
            }
        }
        // Heat Map: grey, then Picasa's HSV gradient map: dark blue (0), blue (31.875), green (127.5),
        // red (223.125), dark red (255). Amount is 1 - Fade.
        "Heat Map" => {
            const STOPS: [(f32, f32, f32); 5] = [(0.0, 240.0, 0.5), (31.875, 240.0, 1.0), (127.5, 120.0, 1.0), (223.125, 0.0, 1.0), (255.0, 0.0, 0.5)];
            let table: Vec<[f32; 3]> = (0..256).map(|i| {
                let v = i as f32;
                let k = STOPS.windows(2).position(|s| v <= s[1].0).unwrap_or(3);
                let (a, b) = (STOPS[k], STOPS[k + 1]);
                let t = ((v - a.0) / (b.0 - a.0)).clamp(0.0, 1.0);
                hsv(a.1 + (b.1 - a.1) * t, 1.0, a.2 + (b.2 - a.2) * t)
            }).collect();
            for (x, y, p) in px.enumerate_pixels_mut() {
                let o = orig.get_pixel(x, y);
                let g = (0.3086 * o[0] as f32 + 0.6094 * o[1] as f32 + 0.082 * o[2] as f32).round().clamp(0.0, 255.0) as usize;
                *p = Rgb(std::array::from_fn(|c| channel(o[c] as f32 * (1.0 - amount) + table[g][c] * amount)));
            }
        }
        // Cross Process (LAP-071): Picasa's CrossProcess graph - curves red (0,0)(60,30)(210,255)
        // (255,255), green (0,0)(47,38)(101,111)(187,206)(255,255), blue (0,32)(255,216) (shadows go
        // blue-green, highlights warm yellow), Contrast 10, Brightness 10, then 0xfcff00 multiplied in
        // at 20%. Amount is 1 - Fade (Picasa's default: no fade).
        "Cross Process" => {
            let tables = [
                curve_table(&[(0.0, 0.0), (255.0, 255.0)])?,
                curve_table(&[(0.0, 0.0), (60.0, 30.0), (210.0, 255.0), (255.0, 255.0)])?,
                curve_table(&[(0.0, 0.0), (47.0, 38.0), (101.0, 111.0), (187.0, 206.0), (255.0, 255.0)])?,
                // a straight line (0,32)-(255,216): the editor curves must start at 0 and end at 255
                std::array::from_fn(|i| channel(32.0 + i as f32 * (216.0 - 32.0) / 255.0)),
            ];
            let mut graded = orig.clone();
            for p in graded.pixels_mut() { *p = Rgb(crate::t_curves::map(p.0, &tables)); }
            simple_color_matrix(&mut graded, 0.0, 10.0, 10.0);
            const TINT: [f32; 3] = [252.0, 255.0, 0.0];
            for p in graded.pixels_mut() {
                *p = Rgb(std::array::from_fn(|c| channel(p[c] as f32 * (0.8 + 0.2 * TINT[c] / 255.0))));
            }
            mix_by(&mut px, &graded, |_, _| amount);
        }
        _ => return Err(format!("Unknown effect: {name}")),
    }
    Ok(DynamicImage::ImageRgb8(px))
}

fn apply_effect(img: DynamicImage, effect: &Value) -> Result<DynamicImage, String> {
    let name = effect["name"].as_str().ok_or("Missing effect name")?;
    let amount = number(effect, "amount", 0.5).clamp(0.0, 1.0);
    if name == "Sharpen" {
        return Ok(picasa_sharpen(&img, amount));
    }
    if PICASA39_EFFECTS.contains(&name) {
        return picasa39_effect(&img, name, amount, effect);
    }
    let blurred = if name == "Soft Focus" || name == "Glow" {
        Some(img.blur(1.0 + amount * 8.0).to_rgb8())
    } else {
        None
    };
    let (w, h) = img.dimensions();
    let mut pixels = img.to_rgb8();
    let color = effect["color"].as_str().unwrap_or("#4090cc");
    let tint: [f32; 3] = std::array::from_fn(|c| {
        color
            .get(1 + 2 * c..3 + 2 * c)
            .and_then(|s| u8::from_str_radix(s, 16).ok())
            .unwrap_or(128) as f32
    });
    let cx = number(effect, "cx", 0.5);
    let cy = number(effect, "cy", 0.5);
    for (x, y, p) in pixels.enumerate_pixels_mut() {
        let r = p[0] as f32;
        let g = p[1] as f32;
        let b = p[2] as f32;
        let gray = 0.299 * r + 0.587 * g + 0.114 * b;
        let distance = (((x as f32 / w as f32 - cx).powi(2) + (y as f32 / h as f32 - cy).powi(2))
            .sqrt()
            * 2.0)
            .clamp(0.0, 1.0);
        let out = match name {
            "Sepia" => [
                0.393 * r + 0.769 * g + 0.189 * b,
                0.349 * r + 0.686 * g + 0.168 * b,
                0.272 * r + 0.534 * g + 0.131 * b,
            ],
            "B&W" => [gray; 3],
            "Warmify" => [r + 20.0 * amount, g + 8.0 * amount, b - 20.0 * amount],
            "Film Grain" => {
                // Stable noise makes reopening/export match the viewer exactly.
                let seed = (x.wrapping_mul(374761393) ^ y.wrapping_mul(668265263))
                    .wrapping_mul(1274126177);
                let noise = ((seed >> 24) as f32 / 255.0 - 0.5) * 60.0 * amount;
                [r + noise, g + noise, b + noise]
            }
            "Tint" => std::array::from_fn(|c| {
                [r, g, b][c] * (1.0 - amount) + gray * tint[c] / 255.0 * amount
            }),
            "Saturation" => std::array::from_fn(|c| gray + ([r, g, b][c] - gray) * (amount * 2.0)),
            "Soft Focus" => std::array::from_fn(|c| {
                let a = distance * amount;
                [r, g, b][c] * (1.0 - a) + blurred.as_ref().unwrap().get_pixel(x, y)[c] as f32 * a
            }),
            "Glow" => std::array::from_fn(|c| {
                let v = blurred.as_ref().unwrap().get_pixel(x, y)[c] as f32;
                [r, g, b][c] + (255.0 - [r, g, b][c]) * v / 255.0 * amount
            }),
            "Filtered B&W" => {
                let v =
                    (r * tint[0] + g * tint[1] + b * tint[2]) / (tint.iter().sum::<f32>().max(1.0));
                [v; 3]
            }
            "Focal B&W" => std::array::from_fn(|c| {
                let a = distance.powf(1.0 + amount * 3.0);
                [r, g, b][c] * (1.0 - a) + gray * a
            }),
            "Graduated Tint" => std::array::from_fn(|c| {
                let a = (1.0 - y as f32 / h as f32) * amount;
                [r, g, b][c] * (1.0 - a) + tint[c] * a
            }),
            _ => return Err(format!("Unknown effect: {name}")),
        };
        *p = Rgb(out.map(channel));
    }
    Ok(DynamicImage::ImageRgb8(pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharpen_is_picasas_3x3_kernel() {
        // 3x3: centre 60, the rest 50. Amount 0.5 = Picasa 50: k = -0.5, centre weight 5.
        let mut img = image::RgbImage::from_pixel(3, 3, Rgb([50, 50, 50]));
        img.put_pixel(1, 1, Rgb([60, 60, 60]));
        let out = picasa_sharpen(&DynamicImage::ImageRgb8(img.clone()), 0.5).to_rgb8();
        assert_eq!(out.get_pixel(1, 1)[0], 100);            // 5*60 - 0.5*8*50
        // a corner (edges repeat): neighbours 50 x5, 60 x1 (the centre), itself 50 x3 in the window
        // weights: itself 5, the other 8 at -0.5 -> 5*50 - 0.5*(50*7 + 60) = 45
        assert_eq!(out.get_pixel(0, 0)[0], 45);
        // amount 0 changes nothing; a flat image is never changed
        assert_eq!(picasa_sharpen(&DynamicImage::ImageRgb8(img.clone()), 0.0).to_rgb8(), img);
        let flat = image::RgbImage::from_pixel(4, 4, Rgb([120, 80, 40]));
        assert_eq!(picasa_sharpen(&DynamicImage::ImageRgb8(flat.clone()), 1.0).to_rgb8(), flat);
    }

    #[test]
    fn cross_process_follows_picasas_curves_and_fades_to_the_photo() {
        let run = |rgb: [u8; 3], amount: f64| {
            let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 4, Rgb(rgb)));
            apply(img, &serde_json::json!({"effects": [{"name": "Cross Process", "amount": amount}]})).unwrap().to_rgb8().get_pixel(1, 1).0
        };
        // shadows: red pulled down, blue lifted from 32 - a cold blue-green dark
        let dark = run([40, 40, 40], 1.0);
        assert!(dark[2] > dark[0] && dark[1] > dark[0], "{dark:?}");
        // highlights: red to white, blue held down (curve top 216, then the yellow tint) - warm yellow
        let light = run([215, 215, 215], 1.0);
        assert!(light[0] >= 250 && light[1] > light[2] + 40 && light[0] > light[2] + 40, "{light:?}");
        // black gets the blue curve's lift (0 -> 32 before contrast and tint)
        let black = run([0, 0, 0], 1.0);
        assert!(black[2] > black[0], "{black:?}");
        // Amount 0 (full fade) leaves the photo as it was; halfway is between the two
        assert_eq!(run([120, 90, 60], 0.0), [120, 90, 60]);
        let (full, half) = (run([215, 215, 215], 1.0), run([215, 215, 215], 0.5));
        for c in 0..3 { assert!((half[c] as i32 - (215 + full[c] as i32) / 2).abs() <= 1, "{half:?} vs {full:?}"); }
    }

    #[test]
    fn eyedropper_temperature_turns_the_picked_colour_neutral() {
        // The Tuning eyedropper sets Color Temperature to (B - R) / 60 (x100 on the slider): the
        // temperature step adds 30 x t to red and takes it from blue, so the picked colour ends with
        // red equal to blue.
        for rgb in [[180u8, 160, 120], [110, 120, 160], [200, 200, 200]] {
            let slider = ((rgb[2] as f32 - rgb[0] as f32) * 100.0 / 60.0).clamp(-100.0, 100.0);
            let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, Rgb(rgb)));
            let out = apply(img, &serde_json::json!({"temperature": slider})).unwrap().to_rgb8().get_pixel(0, 0).0;
            assert!((out[0] as i32 - out[2] as i32).abs() <= 1, "{rgb:?} -> {out:?}");
            assert_eq!(out[1], rgb[1]);
        }
    }

    #[test]
    fn tone_sliders_are_monotonic_bounded_and_identity_at_zero() {
        for (f, hi, sh) in [(1.0, 0.0, 0.0), (0.5, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, -1.0, 0.0), (0.0, 0.0, 1.0), (1.0, 1.0, 1.0)] {
            let mut prior = -1.0;
            for i in 0..=255 {
                let t = tone_curve(i as f32 / 255.0, f, hi, sh);
                assert!((0.0..=1.0).contains(&t), "out of range {t} at {i} for {f},{hi},{sh}");
                assert!(t >= prior, "not monotonic at {i} for {f},{hi},{sh}");
                prior = t;
            }
            assert_eq!(tone_curve(0.0, 0.0, 0.0, 0.0), 0.0);
        }
        for i in 0..=255 { let l = i as f32 / 255.0; assert_eq!(tone_curve(l, 0.0, 0.0, 0.0), l); }
        // fill lifts shadows most, leaves white white
        assert!(tone_curve(0.1, 1.0, 0.0, 0.0) - 0.1 > tone_curve(0.7, 1.0, 0.0, 0.0) - 0.7);
        assert_eq!(tone_curve(1.0, 1.0, 0.0, 0.0), 1.0);
        // shadows never push black below black
        assert_eq!(tone_curve(0.0, 0.0, 0.0, 1.0), 0.0);
    }

    #[test]
    fn fill_light_follows_the_measured_reference_luminance() {
        // LAP-108: output luminance (0..255) of the reference editor's Fill Light per input luminance,
        // pooled over six photos; the fitted curve stays within 4 at half and 11 at full strength.
        for (fill, tol, points) in [
            (0.5, 4.0, [(8, 27), (16, 43), (32, 69), (64, 111), (128, 169), (192, 215), (240, 244)]),
            (1.0, 11.0, [(8, 101), (16, 128), (32, 157), (64, 193), (128, 218), (192, 234), (240, 247)]),
        ] {
            for (input, reference) in points {
                let out = 255.0 * tone_curve(input as f32 / 255.0, fill, 0.0, 0.0);
                assert!((out - reference as f32).abs() <= tol, "fill {fill}: {input} -> {out}, reference {reference}");
            }
        }
    }

    #[test]
    fn tone_sliders_keep_colour_and_never_clip_bright_tones() {
        let pixel = |rgb: [u8; 3], tools: Value| {
            let img = image::RgbImage::from_pixel(1, 1, Rgb(rgb));
            apply(DynamicImage::ImageRgb8(img), &tools).unwrap().to_rgb8().get_pixel(0, 0).0
        };
        // Highlights at full on a bright grey: brighter, still short of white
        let out = pixel([235, 235, 235], serde_json::json!({"highlights": 100}));
        assert!(out[0] > 235 && out[0] < 255, "{out:?}");
        // Fill Light keeps most of the colour (the old uniform lift washed it towards grey: 2.0 -> ~1.5;
        // the reference editor gives about 1.8 and 1.7 here)
        let out = pixel([120, 60, 30], serde_json::json!({"fillLight": 50}));
        assert!(out[0] > 120, "{out:?}");
        let ratio = |p: [u8; 3]| (p[0] as f32 / p[1] as f32, p[1] as f32 / p[2] as f32);
        let b = ratio(out);
        assert!((1.65..=2.02).contains(&b.0) && (1.5..=2.02).contains(&b.1), "{b:?}");
        // ...and never turns a dark saturated colour neon: its chroma grows about as the reference's
        // does (1.9x here), never past 3x
        let chroma = |p: [u8; 3]| *p.iter().max().unwrap() as f32 - *p.iter().min().unwrap() as f32;
        let out = pixel([80, 30, 20], serde_json::json!({"fillLight": 100}));
        assert!(chroma(out) <= 2.0 * chroma([80, 30, 20]) + 1.0 && out[0] > 80, "{out:?}");
        // near-black noise stays near grey: a 3x cap on the colour gain
        let out = pixel([3, 0, 0], serde_json::json!({"fillLight": 100}));
        assert!(chroma(out) <= 3.0 * 3.0 + 1.0, "{out:?}");
        // a saturated bright colour that would overflow is lifted as a whole: same hue order, every
        // channel brighter, its colour eased towards grey just enough to fit (never one channel cut)
        let out = pixel([250, 120, 40], serde_json::json!({"highlights": 100, "fillLight": 100}));
        assert!(out[0] >= out[1] && out[1] >= out[2] && out[1] > 120 && out[2] > 40, "{out:?}");
        // Shadows darkens a dark colour, black stays black
        let out = pixel([40, 30, 20], serde_json::json!({"shadows": 100}));
        assert!(out[0] < 40 && out[2] <= 20, "{out:?}");
        assert_eq!(pixel([0, 0, 0], serde_json::json!({"shadows": 100})), [0, 0, 0]);
    }
    #[test]
    fn text_layers_blend_alpha_without_replacing_the_photo() {
        let source = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(10, 10, Rgb([0, 0, 100])));
        let layer = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([200, 0, 0, 128]),
        ));
        let mut bytes = std::io::Cursor::new(Vec::new());
        layer.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let tools = serde_json::json!({"text":[{"png":base64::engine::general_purpose::STANDARD.encode(bytes.into_inner()),"x":0.2,"y":0.2,"width":0.2,"height":0.2}]});
        let result = apply(source.clone(), &tools).unwrap().to_rgb8();
        assert_eq!(result.get_pixel(0, 0), source.to_rgb8().get_pixel(0, 0));
        let pixel = result.get_pixel(2, 2);
        assert!(pixel[0] > 90 && pixel[0] < 110 && pixel[2] > 40 && pixel[2] < 60);
        assert_eq!(source.to_rgb8().get_pixel(2, 2), &Rgb([0, 0, 100]));
    }
    #[test]
    fn every_picasa_effect_is_repeatable_and_preserves_geometry() {
        let source = DynamicImage::ImageRgb8(image::RgbImage::from_fn(24, 20, |x, y| {
            Rgb([
                ((x * 9 + y % 2 * 60) % 256) as u8,
                ((y * 11 + x % 2 * 40) % 256) as u8,
                60,
            ])
        }));
        for name in [
            "Sharpen",
            "Sepia",
            "B&W",
            "Warmify",
            "Film Grain",
            "Tint",
            "Saturation",
            "Soft Focus",
            "Glow",
            "Filtered B&W",
            "Focal B&W",
            "Graduated Tint",
            "Vignette",
            "Pixelate",
            "Focal Zoom",
            "Lomo-ish",
            "Holga-ish",
            "HDR-ish",
            "Cinemascope",
            "Orton-ish",
            "Infrared",
            "Heat Map",
        ] {
            let tools = serde_json::json!({"effects":[{"name":name,"amount":0.7}]});
            let result = apply(source.clone(), &tools).unwrap();
            assert_eq!(result.dimensions(), source.dimensions());
            assert_eq!(
                result.to_rgb8(),
                apply(source.clone(), &tools).unwrap().to_rgb8(),
                "{name}"
            );
            assert_ne!(result.to_rgb8(), source.to_rgb8(), "{name}");
        }
    }
    fn effect(img: &image::RgbImage, name: &str, amount: f32) -> image::RgbImage {
        let tools = serde_json::json!({"effects":[{"name":name,"amount":amount,"cx":0.5,"cy":0.5}]});
        apply(DynamicImage::ImageRgb8(img.clone()), &tools).unwrap().to_rgb8()
    }

    #[test]
    fn picasa39_vignette_darkens_edges_not_the_middle_and_amount_0_is_identity() {
        let grey = image::RgbImage::from_pixel(160, 120, Rgb([180, 180, 180]));
        let out = effect(&grey, "Vignette", 0.7);
        assert!(out.get_pixel(0, 0)[0] < 60, "corner {:?}", out.get_pixel(0, 0));
        assert!(out.get_pixel(80, 60)[0] >= 175, "middle {:?}", out.get_pixel(80, 60));
        assert!(out.get_pixel(0, 60)[0] < out.get_pixel(40, 60)[0]);
        assert_eq!(effect(&grey, "Vignette", 0.0), grey);
        // Lomo and Holga darken their corners the same way
        assert!(effect(&grey, "Lomo-ish", 0.5).get_pixel(0, 0)[0] < 90);
        assert!(effect(&grey, "Holga-ish", 0.7).get_pixel(0, 0)[0] < 90);
    }

    #[test]
    fn picasa39_pixelate_makes_flat_blocks_of_the_slider_size() {
        let src = image::RgbImage::from_fn(1600, 40, |x, y| Rgb([(x % 256) as u8, (y * 6) as u8, 90]));
        let out = effect(&src, "Pixelate", 0.37); // Picasa's default size 20 at a 1600 px photo
        let size = (2.0f32 + 148.0 * 0.37 * 0.37).round() as u32;
        for x in 0..size { assert_eq!(out.get_pixel(x, 3), out.get_pixel(0, 0)); }
        assert_ne!(out.get_pixel(size, 0), out.get_pixel(0, 0));
    }

    #[test]
    fn picasa39_focal_zoom_keeps_the_centre_and_blurs_the_edges() {
        let src = image::RgbImage::from_fn(200, 200, |x, _| if (x / 4) % 2 == 0 { Rgb([250; 3]) } else { Rgb([5; 3]) });
        let out = effect(&src, "Focal Zoom", 1.0);
        assert_eq!(out.get_pixel(100, 100), src.get_pixel(100, 100));
        // the left edge's middle: the zoom runs along the row, across the stripes
        let contrast = |img: &image::RgbImage, x0: u32| (x0..x0 + 8).map(|x| img.get_pixel(x, 100)[0] as i32).max().unwrap()
            - (x0..x0 + 8).map(|x| img.get_pixel(x, 100)[0] as i32).min().unwrap();
        assert!(contrast(&out, 2) < contrast(&src, 2) / 2, "edge still sharp");
    }

    #[test]
    fn picasa39_hdr_at_strength_1_is_identity_and_more_adds_local_contrast() {
        let src = image::RgbImage::from_fn(1600, 16, |x, _| Rgb([if x < 800 { 100 } else { 140 }; 3]));
        assert_eq!(effect(&src, "HDR-ish", 0.0), src);
        let out = effect(&src, "HDR-ish", 0.33);
        assert!(out.get_pixel(797, 8)[0] < 100 && out.get_pixel(802, 8)[0] > 140, "edge not boosted");
        assert_eq!(out.get_pixel(400, 8)[0], 100); // flat areas stay
    }

    #[test]
    fn masked_hdr_changes_only_the_painted_half_and_global_hdr_is_unchanged() {
        let dir = std::env::temp_dir().join(format!("lap_masked_hdr_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let photo = dir.join("P1.jpg");
        let (w, h) = (96u32, 96u32);
        let src = image::RgbImage::from_fn(w, h, |x, y| Rgb([((x / 6 + y / 6) % 2 * 60 + 90) as u8; 3]));
        let paint = image::GrayImage::from_fn(w, h, |_, y| image::Luma([if y < h / 2 { 255 } else { 0 }]));
        let mut png = Vec::new();
        DynamicImage::ImageLuma8(paint).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let id = crate::t_edit_mask::save(&photo, &png, (w, h), 0.02, false, Value::Null).unwrap();
        let global = serde_json::json!({"effects": [{"name": "HDR-ish", "amount": 0.5}]});
        let masked = serde_json::json!({"effects": [{"name": "HDR-ish", "amount": 0.5, "mask": id}]});
        let g = apply_for(DynamicImage::ImageRgb8(src.clone()), &global, Some(&photo)).unwrap().to_rgb8();
        assert_eq!(g, effect(&src, "HDR-ish", 0.5), "global HDR-ish as before");
        let m = apply_for(DynamicImage::ImageRgb8(src.clone()), &masked, Some(&photo)).unwrap().to_rgb8();
        let band = (0.02 * h as f32 * 3.0).ceil() as u32 + 1;
        let (mut top_diff, mut bottom_diff) = (0, 0);
        for (x, y, p) in m.enumerate_pixels() {
            if y + band < h / 2 { assert_eq!(p, g.get_pixel(x, y)); if p != src.get_pixel(x, y) { top_diff += 1; } }
            if y >= h / 2 + band && p != src.get_pixel(x, y) { bottom_diff += 1; }
        }
        assert!(top_diff > 100, "top half unchanged");
        assert_eq!(bottom_diff, 0, "bottom half touched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn picasa39_cinemascope_letterboxes_without_changing_the_size() {
        let src = image::RgbImage::from_pixel(170, 128, Rgb([200, 150, 100]));
        let out = effect(&src, "Cinemascope", 1.0);
        assert_eq!(out.dimensions(), src.dimensions());
        assert_eq!(out.get_pixel(85, 0), &Rgb([0, 0, 0]));
        assert_eq!(out.get_pixel(85, 127), &Rgb([0, 0, 0]));
        // band 170/1.7 = 100 tall, squeezed to 95 between 15 px bars: 125 of the 128 rows
        assert_eq!(out.get_pixel(85, 15), &Rgb([0, 0, 0]));
        assert_ne!(out.get_pixel(85, 64), &Rgb([0, 0, 0]));
        assert_ne!(out.get_pixel(85, 20), &Rgb([0, 0, 0]));
    }

    #[test]
    fn picasa39_heat_map_and_infrared_follow_picasas_maps() {
        let ramp = image::RgbImage::from_fn(256, 1, |x, _| Rgb([x as u8; 3]));
        let heat = effect(&ramp, "Heat Map", 1.0);
        assert_eq!(heat.get_pixel(0, 0), &Rgb([0, 0, 128]));       // dark blue
        assert_eq!(heat.get_pixel(128, 0)[1], 255);                // green at the middle
        assert!(heat.get_pixel(128, 0)[0] < 10 && heat.get_pixel(128, 0)[2] < 10);
        assert_eq!(heat.get_pixel(255, 0), &Rgb([128, 0, 0]));     // dark red
        let ir = effect(&image::RgbImage::from_fn(64, 64, |x, _| if x < 32 { Rgb([60, 160, 50]) } else { Rgb([120, 120, 120]) }), "Infrared", 1.0);
        let foliage = ir.get_pixel(8, 8);
        assert!(foliage[0] == foliage[1] && foliage[1] == foliage[2] && foliage[0] > 190, "foliage {foliage:?}");
        let grey = ir.get_pixel(60, 8);
        assert!(grey[0] == grey[1] && grey[1] == grey[2] && foliage[0] > grey[0] + 50, "grey {grey:?}");
    }

    #[test]
    fn negative_highlights_compress_only_top_tones_monotonically() {
        let source = DynamicImage::ImageRgb8(image::RgbImage::from_fn(256, 1, |x, _| Rgb([x as u8; 3])));
        let negative = apply(source.clone(), &serde_json::json!({"highlights":-100})).unwrap().to_rgb8();
        let positive = apply(source.clone(), &serde_json::json!({"highlights":50})).unwrap().to_rgb8();
        for x in 0..=204 {
            assert_eq!(negative.get_pixel(x,0), &Rgb([x as u8;3]));
        }
        for x in 205..256 {
            assert!(negative.get_pixel(x,0)[0] <= x as u8);
            assert!(negative.get_pixel(x,0)[0] >= negative.get_pixel(x-1,0)[0]);
        }
        assert!(negative.get_pixel(255,0)[0] < 230);
        assert!(positive.get_pixel(220,0)[0] > 220);
        assert_eq!(source.to_rgb8().get_pixel(255,0), &Rgb([255;3]));
    }

    #[test]
    fn tuning_and_retouch_change_only_virtual_pixels() {
        let source = DynamicImage::ImageRgb8(image::RgbImage::from_fn(20, 20, |x, _| {
            if x < 10 {
                Rgb([255, 0, 0])
            } else {
                Rgb([0, 255, 0])
            }
        }));
        let patch =
            serde_json::json!({"retouch":[{"sx":0.2,"sy":0.5,"tx":0.8,"ty":0.5,"radius":0.1}]});
        assert_eq!(
            apply(source.clone(), &patch)
                .unwrap()
                .to_rgb8()
                .get_pixel(16, 10),
            &Rgb([255, 0, 0])
        );
        assert_eq!(source.to_rgb8().get_pixel(16, 10), &Rgb([0, 255, 0]));
        assert_ne!(
            apply(
                source.clone(),
                &serde_json::json!({"fillLight":20,"highlights":20,"shadows":10,"temperature":20})
            )
            .unwrap()
            .to_rgb8(),
            source.to_rgb8()
        );
    }
}
