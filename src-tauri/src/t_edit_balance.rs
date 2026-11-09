//! Auto Balance (LAP-107, round 3): find what in the photo needs balancing and how, the way HDR
//! tone-mapping would, as ordinary Automask entries (detail mode, see t_edit_local) the user can then
//! change, switch off or remove. Pure and deterministic: the same photo gives the same plan.
//!
//! Regions, in the order they are applied:
//!   Hazy areas  a smooth large-scale veil (the smallest colour value over windows of 20% of the short
//!               side, blurred) where flare or mist lifts everything; a black point on each channel.
//!   Sky         the selection model's sky, kept only if it looks like sky, grown 2% and gated by its own
//!               brightness range so its edge leaves no rim on a neck or a wire.
//!   Bright / Dark areas  from the brightness map, gated by the edge-aware base tone (smooth inside an
//!               object, sharp at its edges) and kept off every person and face.
//!   People      every face (the library's and the face finder's) prompting one selection: faces are
//!               masked whole or not at all; lifted only when clearly in the dark.
//!   Whole photo the range compression of a tone-map, when nothing above has lifted the shadows.
//! Every mask is feathered edge-aware (a guided filter with the photo as guide), so its transition
//! follows the subject's outline instead of spilling a glow. When dark areas fill most of the photo but
//! still hold their detail, the darkness is the mood: no shadow lift, no range compression.
// Reference implementation used when validating the adjustment.
use crate::t_edit_local::{base_layer, guided};
use image::{imageops::FilterType, ImageBuffer, Luma, RgbImage};
use serde_json::{json, Value};

type Map = Vec<f32>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Values { pub shadows: i32, pub highlights: i32, pub whites: i32, pub blacks: i32 }

impl Values {
    fn any(&self) -> bool { [self.shadows, self.highlights, self.whites, self.blacks].iter().any(|v| v.abs() >= 10) }
    pub fn entry(&self, region: &str, mask: Option<&str>) -> Value {
        json!({"region": region, "mask": mask, "enabled": true, "detail": true, "shadows": self.shadows,
               "highlights": self.highlights, "whites": self.whites, "blacks": self.blacks, "curve": [[0, 0], [1, 1]]})
    }
}

pub struct Planned { pub region: &'static str, pub mask: Map, pub values: Values }

fn smooth(x: f32, a: f32, b: f32) -> f32 { let t = ((x - a) / (b - a)).clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }
fn luma(img: &RgbImage) -> Map { img.pixels().map(|p| (p[0] as f32 * 0.299 + p[1] as f32 * 0.587 + p[2] as f32 * 0.114) / 255.0).collect() }
fn mean(m: &[f32]) -> f32 { m.iter().sum::<f32>() / m.len().max(1) as f32 }

fn resize(m: &[f32], w: u32, h: u32, nw: u32, nh: u32) -> Map {
    let img: ImageBuffer<Luma<f32>, Vec<f32>> = ImageBuffer::from_raw(w, h, m.to_vec()).expect("map size");
    image::imageops::resize(&img, nw, nh, FilterType::Triangle).into_raw()
}
fn small_size(w: u32, h: u32) -> (u32, u32) {
    let s = 512.0 / w.max(h) as f32;
    (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1))
}
fn gaussian(m: &[f32], w: u32, h: u32, sigma: f32) -> Map {
    let img: ImageBuffer<Luma<f32>, Vec<f32>> = ImageBuffer::from_raw(w, h, m.to_vec()).expect("map size");
    image::imageops::blur(&img, sigma.max(0.5)).into_raw()
}
/// Square min (erode) or max (dilate) filter of radius r.
fn extreme(m: &[f32], w: usize, h: usize, r: usize, max: bool) -> Map {
    let pick = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
    let mut a = vec![0.0; m.len()];
    for y in 0..h { for x in 0..w {
        let mut v = m[y * w + x];
        for k in x.saturating_sub(r)..(x + r + 1).min(w) { v = pick(v, m[y * w + k]); }
        a[y * w + x] = v;
    } }
    let mut out = vec![0.0; m.len()];
    for x in 0..w { for y in 0..h {
        let mut v = a[y * w + x];
        for k in y.saturating_sub(r)..(y + r + 1).min(h) { v = pick(v, a[k * w + x]); }
        out[y * w + x] = v;
    } }
    out
}
/// Weighted quantile of `v` (values 0..1) by weights `w`, on a 1024-bin histogram.
fn wq(v: &[f32], w: &[f32], q: f32) -> f32 {
    let mut hist = vec![0.0f64; 1024];
    let mut total = 0.0f64;
    for (x, &wt) in v.iter().zip(w) { if wt > 0.0 { hist[((x.clamp(0.0, 1.0) * 1023.0) as usize).min(1023)] += wt as f64; total += wt as f64; } }
    if total <= 0.0 { return 0.5; }
    let target = q as f64 * total;
    let mut c = 0.0;
    for (i, h) in hist.iter().enumerate() { c += h; if c >= target { return i as f32 / 1023.0; } }
    1.0
}

/// Edge-aware feather: the mask's soft edge snaps to the subject's own outline (guided filter, the
/// photo's brightness as guide, radius 0.4% of the long side), so no glow spills onto a neighbour.
fn feather(m: &[f32], l: &[f32], w: usize, h: usize) -> Map {
    let r = ((w.max(h) as f32 * 0.004).round() as usize).max(2);
    guided(l, m, w, h, r, 1e-3).into_iter().map(|v| v.clamp(0.0, 1.0)).collect()
}

/// Every face box grown by 60% of its size each way (the head), soft-edged; faces as [x, y, w, h] fractions.
pub fn face_boxes(faces: &[[f32; 4]], w: u32, h: u32) -> Map {
    let mut m = vec![0.0f32; (w * h) as usize];
    for f in faces {
        let x0 = ((f[0] - f[2] * 0.6) * w as f32).max(0.0) as u32;
        let y0 = ((f[1] - f[3] * 0.6) * h as f32).max(0.0) as u32;
        let x1 = (((f[0] + f[2] * 1.6) * w as f32).max(0.0) as u32).min(w);
        let y1 = (((f[1] + f[3] * 1.6) * h as f32).max(0.0) as u32).min(h);
        for y in y0..y1 { for x in x0..x1 { m[(y * w + x) as usize] = 1.0; } }
    }
    if faces.is_empty() { m } else { gaussian(&m, w, h, (0.01 * w.min(h) as f32).max(1.0)) }
}

/// The library's faces plus the face finder's, without counting one face twice.
pub fn merge_faces(library: &[[f32; 4]], found: &[[f32; 4]]) -> Vec<[f32; 4]> {
    let mut out = library.to_vec();
    for f in found {
        let (cx, cy) = (f[0] + f[2] / 2.0, f[1] + f[3] / 2.0);
        if !out.iter().any(|g| g[0] <= cx && cx <= g[0] + g[2] && g[1] <= cy && cy <= g[1] + g[3]) { out.push(*f); }
    }
    out
}

/// Prompts for one selection of all people: a point on each face and one on its torso (largest six).
pub fn people_points(faces: &[[f32; 4]]) -> Vec<(f32, f32)> {
    let mut faces = faces.to_vec();
    faces.sort_by(|a, b| (b[2] * b[3]).total_cmp(&(a[2] * a[3])));
    let mut out = Vec::new();
    for f in faces.iter().take(6) {
        let (cx, cy) = (f[0] + f[2] / 2.0, f[1] + f[3] / 2.0);
        out.push((cx, cy));
        let torso = (cy + 1.6 * f[3]).min(0.95);
        if torso > cy + 0.15 * f[3] { out.push((cx, torso)); }
    }
    out
}

fn looks_like_sky(m: &[f32], l: &[f32], w: usize, h: usize) -> bool {
    let rows = (h / 20).max(1);
    let area = m.iter().sum::<f32>();
    let frac = area / (w * h) as f32;
    let top = m[..rows * w].iter().sum::<f32>() / (rows * w) as f32;
    let bright = m.iter().zip(l).map(|(a, b)| a * b).sum::<f32>() / area.max(1.0);
    (0.03..=0.75).contains(&frac) && top > 0.3 && bright >= 0.45
}

/// Sky made halo-free: grown 2% of the short side, gated by its own base-brightness range (2..98%).
fn refine_sky(m: &[f32], base: &[f32], w: u32, h: u32) -> Map {
    let (lo, hi) = (wq(base, m, 0.02), wq(base, m, 0.98));
    let (sw, sh) = small_size(w, h);
    let small = resize(m, w, h, sw, sh);
    let r = (((0.02 * sw.min(sh) as f32).round() as usize).max(3) | 1) / 2;
    let grown = resize(&extreme(&small, sw as usize, sh as usize, r, true), sw, sh, w, h);
    (0..m.len()).map(|k| {
        let gate = smooth(base[k], lo - 0.08, lo) * (1.0 - smooth(base[k], hi, hi + 0.08));
        m[k].max(grown[k]) * gate
    }).collect()
}

/// Dark and Bright areas: the base tone decides, inside areas that are dark or bright overall.
fn tone_masks(l: &[f32], base: &[f32], w: u32, h: u32) -> (Map, Map) {
    let (sw, sh) = small_size(w, h);
    let small = resize(l, w, h, sw, sh);
    let blur = gaussian(&small, sw, sh, 0.02 * sw.min(sh) as f32);
    let r = ((0.04 * sw.min(sh) as f32).round() as usize).max(1);
    let lo = resize(&extreme(&blur, sw as usize, sh as usize, r, false), sw, sh, w, h);
    let hi = resize(&extreme(&blur, sw as usize, sh as usize, r, true), sw, sh, w, h);
    let dark = (0..l.len()).map(|k| smooth(base[k], 0.45, 0.25) * smooth(lo[k], 0.55, 0.40)).collect();
    let bright = (0..l.len()).map(|k| smooth(base[k], 0.65, 0.85) * smooth(hi[k], 0.45, 0.60)).collect();
    (dark, bright)
}

/// The haze / flare veil (0..1): smallest colour over 20% windows, blurred (sigma 10%).
fn veil(img: &RgbImage) -> Map {
    let (w, h) = img.dimensions();
    let mn: Map = img.pixels().map(|p| *p.0.iter().min().unwrap() as f32 / 255.0).collect();
    let (sw, sh) = small_size(w, h);
    let small = resize(&mn, w, h, sw, sh);
    let r = ((0.2 * sw.min(sh) as f32).round() as usize).max(3) / 2;
    let v = gaussian(&extreme(&small, sw as usize, sh as usize, r, false), sw, sh, 0.1 * sw.min(sh) as f32);
    resize(&v, sw, sh, w, h)
}

/// Slider values for a region from its base histogram (and its pixels' darkest 0.5% for haze).
pub fn auto_values(base: &[f32], l: &[f32], m: &[f32], region: &str) -> Values {
    let w: Map = m.iter().map(|&v| if v > 0.05 { v } else { 0.0 }).collect();
    let (med, top, low) = (wq(base, &w, 0.5), wq(base, &w, 0.995), wq(l, &w, 0.005));
    let total: f32 = w.iter().sum();
    let blown = w.iter().zip(l).filter(|(_, p)| **p >= 0.97).map(|(a, _)| a).sum::<f32>() / total.max(1e-6);
    let t = match region { "people" => 0.45, "dark" => 0.38, "sky" => 0.70, "bright" => 0.60, _ => 0.46 };
    let (mut s, mut hl, mut wh, mut bl) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let x = med.clamp(0.03, 0.97);
    if region == "people" {
        if med < 0.33 { s = ((t - x) / (2.025 * x * (1.0 - x).powi(2))).min(0.6); }     // only people clearly in the dark
    } else if med < t - 0.03 && region != "sky" {                                       // a deep blue sky is not lifted
        s = ((t - x) / (2.025 * x * (1.0 - x).powi(2))).min(0.8);
    } else if med > t + 0.03 && blown < 0.3 {                                            // a mostly blown area has nothing to recover
        let cap = if region == "bright" { -1.0 } else { -0.7 };                         // a held-back sky stays a sky
        hl = ((t - x) / (2.025 * x * x * (1.0 - x))).max(cap);
    }
    if region == "people" && blown > 0.02 && top > 0.97 { hl = -0.3; }                 // held back only where they clip
    if top > 0.94 && top < 0.985 && blown < 0.05 { wh = ((0.92 - top) / (0.12 * top.powi(4))).max(-1.0); }
    if low > 0.06 && (region == "dark" || region == "people") {                          // dark with no real black: hazy
        bl = (-(low - 0.03) / crate::t_edit_local::BLACK_POINT_MAX).max(-0.6);
    }
    let r = |v: f32| (v.clamp(-1.0, 1.0) * 100.0).round() as i32;
    Values { shadows: r(s), highlights: r(hl), whites: r(wh), blacks: r(bl) }
}

/// One detail-mode entry blended through `m` (0..1): what the renderer will do, for measuring the next.
fn apply(img: &RgbImage, v: Values, m: &[f32]) -> RgbImage {
    let done = crate::t_edit_local::detail_image(img, &v.entry("whole", None)).expect("detail curve");
    let mut out = img.clone();
    for (k, p) in out.pixels_mut().enumerate() {
        let a = m[k].clamp(0.0, 1.0);
        if a <= 0.0 { continue; }
        let d = done.get_pixel((k as u32) % img.width(), (k as u32) / img.width());
        *p = image::Rgb(std::array::from_fn(|c| (p[c] as f32 * (1.0 - a) + d[c] as f32 * a).round().clamp(0.0, 255.0) as u8));
    }
    out
}

/// The plan for `img` (the photo as edited but without any Automask or curves), given the selection
/// model's sky and people masks (0..1, img size; None when not available) and every face (fractions).
/// Returns the entries in apply order with their masks, and whether the mood check kept the darkness.
pub fn plan(img: &RgbImage, sky: Option<&[f32]>, people: Option<&[f32]>, faces: &[[f32; 4]]) -> (Vec<Planned>, bool) {
    let (w, h) = img.dimensions();
    let (wu, hu) = (w as usize, h as usize);
    let l0 = luma(img);
    let base0 = base_layer(&l0, wu, hu);
    let mut regs: Vec<(&'static str, Map)> = Vec::new();
    let mut taken = vec![0.0f32; l0.len()];
    if let Some(s) = sky {
        if looks_like_sky(s, &l0, wu, hu) {
            let m = feather(&refine_sky(s, &base0, w, h), &l0, wu, hu);
            for (t, v) in taken.iter_mut().zip(&m) { *t = t.max(*v); }
            regs.push(("sky", m));
        }
    }
    let mut people_mask = vec![0.0f32; l0.len()];
    if let Some(p) = people {
        if mean(p) >= 0.01 {
            let m = feather(p, &l0, wu, hu);
            for (t, v) in taken.iter_mut().zip(&m) { *t = t.max(*v); }
            people_mask = m.clone();
            regs.push(("people", m));
        }
    }
    // Whole or not at all: no tone area cuts through a person or a face.
    let heads = face_boxes(faces, w, h);
    let keep_out: Map = taken.iter().zip(&heads).map(|(a, b)| a.max(*b)).collect();
    let (dark, bright) = tone_masks(&l0, &base0, w, h);
    let bright: Map = feather(&bright, &l0, wu, hu).iter().zip(&keep_out).map(|(m, k)| m * (1.0 - k)).collect();
    let dark: Map = feather(&dark, &l0, wu, hu).iter().zip(&keep_out).map(|(m, k)| m * (1.0 - k)).collect();
    regs.push(("bright", bright));
    regs.push(("dark", dark));
    let order = |r: &str| match r { "sky" => 0, "bright" => 1, "dark" => 2, _ => 3 };
    regs.sort_by_key(|(r, _)| order(r));

    let mut cur = img.clone();
    let mut chosen: Vec<Planned> = Vec::new();
    // Haze first.
    let vl = veil(img);
    let top = wq(&vl, &vec![1.0; vl.len()], 0.95);
    if top > 0.18 {
        let sky_m = regs.iter().find(|(r, _)| *r == "sky").map(|(_, m)| m.clone()).unwrap_or_else(|| vec![0.0; vl.len()]);
        let hm: Map = (0..vl.len()).map(|k| (vl[k] / top).clamp(0.0, 1.0) * (1.0 - sky_m[k]) * (1.0 - people_mask[k].max(heads[k]))).collect();
        let v = Values { shadows: 0, highlights: 0, whites: 0, blacks: -((((top - 0.10) / crate::t_edit_local::BLACK_POINT_MAX).min(1.0) * 100.0).round() as i32) };
        if mean(&hm) >= 0.05 && v.any() { cur = apply(&cur, v, &hm); chosen.push(Planned { region: "haze", mask: hm, values: v }); }
    }
    // Mood: dark areas fill most of the photo but still hold their detail.
    let dk = regs.iter().find(|(r, _)| *r == "dark").map(|(_, m)| m).unwrap();
    let crushed = dk.iter().zip(&l0).filter(|(_, l)| **l < 0.06).map(|(m, _)| m).sum::<f32>() / dk.iter().sum::<f32>().max(1.0);
    let mood = mean(dk) > 0.5 && crushed < 0.04;
    for (region, m) in regs {
        if mood && (region == "dark" || region == "people") { continue; }
        if mean(&m) < 0.02 { continue; }
        let l = luma(&cur);
        let v = auto_values(&base_layer(&l, wu, hu), &l, &m, region);
        if !v.any() { continue; }
        cur = apply(&cur, v, &m);
        chosen.push(Planned { region, mask: m, values: v });
    }
    if mood { return (chosen, true); }
    // Last, the whole photo's range, as a tone-map does.
    let l = luma(&cur);
    let base = base_layer(&l, wu, hu);
    let ones = vec![1.0f32; l.len()];
    let (q25, q90) = (wq(&base, &ones, 0.25), wq(&base, &ones, 0.90));
    let blown = l.iter().filter(|v| **v >= 0.97).count() as f32 / l.len() as f32;
    let lifted = chosen.iter().any(|c| c.region == "dark" || c.region == "people");
    let (mut s, mut hl) = (0.0f32, 0.0f32);
    if q25 < 0.25 && !lifted { let x = q25.max(0.03); s = ((0.28 - x) / (2.025 * x * (1.0 - x).powi(2))).min(0.6); }
    if q90 > 0.90 && blown < 0.15 { let y = q90.min(0.99); hl = ((0.88 - y) / (2.025 * y * y * (1.0 - y))).max(-0.6); }
    let v = Values { shadows: (s * 100.0).round() as i32, highlights: (hl * 100.0).round() as i32, whites: 0, blacks: 0 };
    if v.any() { chosen.push(Planned { region: "whole", mask: ones, values: v }); }
    (chosen, false)
}

/// A 0..1 map as an 8-bit grey PNG (the mask files t_edit_mask stores).
pub fn mask_png(m: &[f32], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let img = image::GrayImage::from_raw(w, h, m.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect()).ok_or("mask size")?;
    let mut png = Vec::new();
    image::DynamicImage::ImageLuma8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(png)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backlit, hazy scene: bright grey-veiled top, a dark foreground band with texture.
    fn scene() -> RgbImage {
        RgbImage::from_fn(160, 120, |x, y| {
            let tex = ((x / 3 + y / 3) % 2) as u8 * 8;
            if y < 50 { image::Rgb([235, 236, 238]) } else if y < 80 { image::Rgb([150 + tex, 160 + tex, 140 + tex]) } else { image::Rgb([40 + tex, 45 + tex, 38 + tex]) }
        })
    }

    #[test]
    fn the_plan_is_deterministic_and_lifts_the_dark_foreground() {
        let img = scene();
        let (a, _) = plan(&img, None, None, &[]);
        let (b, _) = plan(&img, None, None, &[]);
        assert_eq!(a.iter().map(|p| (p.region, p.values)).collect::<Vec<_>>(), b.iter().map(|p| (p.region, p.values)).collect::<Vec<_>>());
        let dark = a.iter().find(|p| p.region == "dark").expect("a dark area is found");
        assert!(dark.values.shadows > 0);
        assert!(dark.mask[(100 * 160 + 80) as usize] > 0.5, "inside the dark band");
        assert!(dark.mask[(20 * 160 + 80) as usize] < 0.05, "not in the bright top");
    }

    #[test]
    fn a_face_is_never_split_by_a_tone_area() {
        let img = scene();
        // A face sitting across the dark band's top edge.
        let face = [0.45f32, 0.62, 0.1, 0.12];
        let (with, _) = plan(&img, None, None, &[face]);
        let dark = with.iter().find(|p| p.region == "dark").unwrap();
        let (cx, cy) = (((face[0] + face[2] / 2.0) * 160.0) as u32, ((face[1] + face[3] / 2.0) * 120.0) as u32);
        assert!(dark.mask[(cy * 160 + cx) as usize] < 0.05, "the face is left whole, outside the dark area");
    }

    #[test]
    fn a_dark_scene_that_holds_its_detail_is_left_alone() {
        // A lamp-lit room: mostly dark but nothing crushed (no pixel below 0.06).
        let room = RgbImage::from_fn(160, 120, |x, y| {
            let tex = ((x / 2 + y / 2) % 3) as u8 * 6;
            if x > 130 && y < 40 { image::Rgb([250, 240, 200]) } else { image::Rgb([60 + tex, 48 + tex, 36 + tex]) }
        });
        let (chosen, mood) = plan(&room, None, None, &[]);
        assert!(mood);
        assert!(!chosen.iter().any(|p| p.region == "dark" || p.region == "people" || p.region == "whole"));
    }

    #[test]
    fn a_bright_subject_is_not_pulled_down_and_merged_faces_are_not_doubled() {
        let mut l = vec![0.8f32; 100];
        l[0] = 0.5;
        let v = auto_values(&l, &l, &vec![1.0; 100], "people");
        assert_eq!(v.highlights, 0, "a well-lit person is not held back");
        let merged = merge_faces(&[[0.1, 0.1, 0.2, 0.2]], &[[0.12, 0.12, 0.15, 0.15], [0.6, 0.1, 0.1, 0.1]]);
        assert_eq!(merged.len(), 2);
    }
}
