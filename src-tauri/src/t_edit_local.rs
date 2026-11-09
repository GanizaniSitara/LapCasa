//! Automask (LAP-107): local curves, a tone curve applied to one region of the photo.
//!
//! The recipe keeps them as settings, never as baked pixels, so each can be reopened, switched off
//! or changed later: `picasa.local` is a list, applied in order, of
//! `{"region": "sky", "mask": "<id>" | null, "enabled": true, "shadows": 0, "highlights": 0,
//!   "whites": 0, "blacks": 0, "curve": [[0,0],[1,1]]}`.
//! The four sliders (-100..100) bend the curve: Shadows peaks at a third of the way up, Highlights
//! at two thirds, Blacks moves the black end and Whites the white end. The curve points the user
//! draws (the Curves editor, `t_curves`) apply on top of the bent curve. No mask (or region
//! "whole") means the whole photo; a mask is a `t_edit_mask` id, feathered and blended as a masked
//! effect is. The curve moves brightness only and the colour follows (`tone_gain`), pixel by pixel,
//! so there are no halos and nothing clips.
//!
//! `"detail": true` (Auto Balance's entries, and new ones made by hand) renders the entry the way HDR
//! tone-mapping does: Blacks below 0 sets a black point on each colour channel (up to 0.25 at -100;
//! clears haze and brings back its colour), and the curve moves an edge-aware BASE layer of the
//! brightness (a two-scale guided filter of log brightness) while the detail rides on it unchanged, at
//! most 2.2x brighter, with a soft shoulder above 0.92. The guided filter stops at strong edges, so a
//! lifted shadow does not spill a glow onto its bright neighbour. Entries without it render as before.
use image::{DynamicImage, Rgb, RgbImage};
use serde_json::Value;

const STEPS: usize = 1024;

fn slider(v: &Value, key: &str) -> f32 {
    (v[key].as_f64().unwrap_or(0.0) as f32 / 100.0).clamp(-1.0, 1.0)
}

/// The sliders' bend of x (0..1), before it is made monotonic. Identity when all four are 0.
pub fn slider_curve(x: f32, shadows: f32, highlights: f32, whites: f32, blacks: f32) -> f32 {
    let x1 = 1.0 - x;
    x + shadows * 0.30 * 6.75 * x * x1 * x1 + highlights * 0.30 * 6.75 * x * x * x1
        + blacks * 0.12 * x1.powi(4) + whites * 0.12 * x.powi(4)
}

/// Detail mode's black point on each channel (0..1) for an entry, 0 when Blacks is not below 0.
pub const BLACK_POINT_MAX: f32 = 0.25;
fn detail(adj: &Value) -> bool { adj["detail"].as_bool() == Some(true) }
fn black_point(adj: &Value) -> f32 {
    if detail(adj) { (-slider(adj, "blacks")).max(0.0) * BLACK_POINT_MAX } else { 0.0 }
}

/// The adjustment's whole curve sampled at STEPS points over 0..1: sliders, then the user's points.
/// In detail mode a negative Blacks is the channel black point instead, so it is left out here.
pub fn table(adj: &Value) -> Result<Vec<f32>, String> {
    let b = if detail(adj) { slider(adj, "blacks").max(0.0) } else { slider(adj, "blacks") };
    let (s, h, w) = (slider(adj, "shadows"), slider(adj, "highlights"), slider(adj, "whites"));
    let points: Option<[u8; 256]> = match adj.get("curve") {
        Some(c) if !c.is_null() => {
            let points: Vec<[f64; 2]> = serde_json::from_value(c.clone()).map_err(|e| format!("Local curve: {e}"))?;
            Some(crate::t_curves::lut(&points)?)
        }
        _ => None,
    };
    let mut out = Vec::with_capacity(STEPS);
    let mut last = 0.0f32;
    for i in 0..STEPS {
        let x = i as f32 / (STEPS - 1) as f32;
        // Monotonic: tones keep their order whatever the sliders do together.
        let mut y = slider_curve(x, s, h, w, b).clamp(0.0, 1.0).max(last);
        last = y;
        if let Some(lut) = &points {
            let f = y * 255.0;
            let (i0, t) = (f.floor() as usize, f.fract());
            let i1 = (i0 + 1).min(255);
            y = (lut[i0.min(255)] as f32 * (1.0 - t) + lut[i1] as f32 * t) / 255.0;
        }
        out.push(y);
    }
    Ok(out)
}

/// Whether an adjustment changes anything: off, or every slider 0 with a straight curve, is a no-op.
fn active(adj: &Value) -> bool {
    if adj["enabled"].as_bool() == Some(false) { return false; }
    let sliders = ["shadows", "highlights", "whites", "blacks"].iter().any(|k| slider(adj, k) != 0.0);
    let curve = adj["curve"].as_array().is_some_and(|c| c.iter().any(|p| p[0] != p[1]));
    sliders || curve
}

/// The curve on every pixel of `img` (the whole photo; the mask is applied by the caller).
pub fn curve_image(img: &DynamicImage, table: &[f32]) -> DynamicImage {
    let mut px = img.to_rgb8();
    let last = (table.len() - 1) as f32;
    for p in px.pixels_mut() {
        let rgb: [f32; 3] = std::array::from_fn(|c| p[c] as f32);
        let l = ((rgb[0] * 0.299 + rgb[1] * 0.587 + rgb[2] * 0.114) / 255.0).clamp(0.0, 1.0);
        let f = l * last;
        let (i0, t) = (f.floor() as usize, f.fract());
        let tone = table[i0] * (1.0 - t) + table[(i0 + 1).min(table.len() - 1)] * t;
        let out = crate::t_edit_tools::tone_gain(rgb, l, tone);
        *p = Rgb(out.map(|v| v.clamp(0.0, 255.0).round() as u8));
    }
    DynamicImage::ImageRgb8(px)
}

// ---- detail mode ----

/// Mean of `src` (w x h) over the (2r+1)^2 window around each pixel, the window clipped to the image.
fn box_mean(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut sat = vec![0.0f64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0.0f64;
        for x in 0..w {
            row += src[y * w + x] as f64;
            sat[(y + 1) * (w + 1) + x + 1] = sat[y * (w + 1) + x + 1] + row;
        }
    }
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(w));
            let sum = sat[y1 * (w + 1) + x1] - sat[y0 * (w + 1) + x1] - sat[y1 * (w + 1) + x0] + sat[y0 * (w + 1) + x0];
            out[y * w + x] = (sum / ((y1 - y0) * (x1 - x0)) as f64) as f32;
        }
    }
    out
}

/// He et al.'s guided filter: `p` smoothed so its edges follow those of the guide `i`.
pub fn guided(i: &[f32], p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let mul = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).collect::<Vec<f32>>();
    let (mi, mp) = (box_mean(i, w, h, r), box_mean(p, w, h, r));
    let (mip, mii) = (box_mean(&mul(i, p), w, h, r), box_mean(&mul(i, i), w, h, r));
    let a: Vec<f32> = (0..w * h).map(|k| (mip[k] - mi[k] * mp[k]) / (mii[k] - mi[k] * mi[k] + eps)).collect();
    let b: Vec<f32> = (0..w * h).map(|k| mp[k] - a[k] * mi[k]).collect();
    let (ma, mb) = (box_mean(&a, w, h, r), box_mean(&b, w, h, r));
    (0..w * h).map(|k| ma[k] * i[k] + mb[k]).collect()
}

/// The broad light of brightness `l` (0..1, w x h): an edge-aware base layer, the mean of guided
/// filters of log brightness at 1.2% and 5% of the long side. Scale-free, so a preview and a full-size
/// export get the same base.
pub fn base_layer(l: &[f32], w: usize, h: usize) -> Vec<f32> {
    let ly: Vec<f32> = l.iter().map(|v| (v + 1e-3).log2()).collect();
    let long = w.max(h) as f32;
    let (r1, r2) = (((long * 0.012).round() as usize).max(4), ((long * 0.05).round() as usize).max(8));
    let (b1, b2) = (guided(&ly, &ly, w, h, r1, 0.02), guided(&ly, &ly, w, h, r2, 0.05));
    b1.iter().zip(&b2).map(|(a, b)| ((0.5 * a + 0.5 * b).exp2() - 1e-3).clamp(0.0, 1.0)).collect()
}

/// One entry in detail mode on the whole of `img` (the caller applies the mask).
pub fn detail_image(img: &RgbImage, adj: &Value) -> Result<RgbImage, String> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let bp = black_point(adj) * 255.0;
    let rgb: Vec<[f32; 3]> = img.pixels().map(|p| std::array::from_fn(|c| {
        let v = p[c] as f32;
        if bp > 0.0 { ((v - bp) / (1.0 - bp / 255.0)).clamp(0.0, 255.0) } else { v }
    })).collect();
    let l: Vec<f32> = rgb.iter().map(|c| ((c[0] * 0.299 + c[1] * 0.587 + c[2] * 0.114) / 255.0).clamp(0.0, 1.0)).collect();
    let base = base_layer(&l, w, h);
    let t = table(adj)?;
    let last = (t.len() - 1) as f32;
    let mut out = RgbImage::new(img.width(), img.height());
    for (k, p) in out.pixels_mut().enumerate() {
        let f = base[k] * last;
        let (i0, fr) = (f.floor() as usize, f.fract());
        let nb = t[i0] * (1.0 - fr) + t[(i0 + 1).min(t.len() - 1)] * fr;
        // The base moves, the detail rides on it; at most 2.2x brighter (deep shadows open without
        // blowing up their noise), with a soft shoulder so nothing clips.
        let mut o = l[k] * ((nb + 0.01) / (base[k] + 0.01)).min(2.2);
        if o > 0.92 { o = 0.92 + 0.08 * ((o - 0.92) / 0.08).tanh(); }
        let v = crate::t_edit_tools::tone_gain(rgb[k], l[k], o.clamp(0.0, 1.0));
        *p = Rgb(v.map(|x| x.clamp(0.0, 255.0).round() as u8));
    }
    Ok(out)
}

/// Every enabled local adjustment of `list`, in order. `photo` locates the masks.
pub fn apply(mut img: DynamicImage, list: &Value, photo: Option<&std::path::Path>) -> Result<DynamicImage, String> {
    let Some(list) = list.as_array() else { return Ok(img) };
    for adj in list.iter().filter(|a| active(a)) {
        let curved = if detail(adj) { DynamicImage::ImageRgb8(detail_image(&img.to_rgb8(), adj)?) } else { curve_image(&img, &table(adj)?) };
        img = match adj["mask"].as_str() {
            Some(id) if adj["region"].as_str() != Some("whole") => crate::t_edit_mask::apply(img, curved, photo, id),
            _ => curved,
        };
    }
    Ok(img)
}

/// Checks a recipe's local list before it is saved (the same rule as the global curves).
pub fn validate(list: &Value) -> Result<(), String> {
    for adj in list.as_array().into_iter().flatten() {
        table(adj)?;
        if let Some(id) = adj["mask"].as_str() {
            if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) { return Err(format!("Invalid mask id {id}")); }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ramp() -> RgbImage {
        RgbImage::from_fn(64, 64, |x, y| Rgb([(x * 4) as u8, (y * 4) as u8, ((x + y) * 2) as u8]))
    }
    fn luma(p: &Rgb<u8>) -> f32 { p[0] as f32 * 0.299 + p[1] as f32 * 0.587 + p[2] as f32 * 0.114 }

    #[test]
    fn neutral_adjustments_leave_the_photo_byte_identical() {
        let src = DynamicImage::ImageRgb8(ramp());
        let list = json!([{"region": "whole", "shadows": 0, "highlights": 0, "whites": 0, "blacks": 0, "curve": [[0, 0], [1, 1]]},
                          {"region": "whole", "shadows": 80, "enabled": false}]);
        assert_eq!(apply(src.clone(), &list, None).unwrap().to_rgb8(), src.to_rgb8());
        assert_eq!(apply(src.clone(), &json!(null), None).unwrap().to_rgb8(), src.to_rgb8());
    }

    #[test]
    fn sliders_bend_the_curve_where_they_say_and_it_stays_monotonic_and_in_range() {
        let lift = table(&json!({"shadows": 100})).unwrap();
        assert!(lift[256] > 0.25 + 0.15, "shadows lift the dark third: {}", lift[256]);
        assert!((lift[1023] - 1.0).abs() < 1e-4 && lift[0].abs() < 1e-4, "ends stay put");
        let hold = table(&json!({"highlights": -100})).unwrap();
        assert!(hold[768] < 0.75 - 0.15, "highlights hold back the bright third: {}", hold[768]);
        assert!((hold[100] - 100.0 / 1023.0).abs() < 0.03, "and barely touch the darks");
        let ends = table(&json!({"blacks": 100, "whites": -100})).unwrap();
        assert!(ends[0] > 0.1 && ends[1023] < 0.9);
        let all = table(&json!({"shadows": 100, "highlights": -100, "whites": 100, "blacks": -100})).unwrap();
        assert!(all.windows(2).all(|p| p[1] >= p[0]) && all.iter().all(|v| (0.0..=1.0).contains(v)));
    }

    #[test]
    fn user_points_apply_on_top_of_the_sliders() {
        let only_points = table(&json!({"curve": [[0, 0], [0.5, 0.75], [1, 1]]})).unwrap();
        assert!((only_points[512] - 0.75).abs() < 0.02);
        let both = table(&json!({"shadows": 50, "curve": [[0, 0], [0.5, 0.75], [1, 1]]})).unwrap();
        assert!(both[300] > only_points[300]);
        assert!(table(&json!({"curve": [[0, 0], [0, 0.5], [1, 1]]})).is_err());
    }

    #[test]
    fn lifting_shadows_keeps_hue_and_never_clips() {
        let src = RgbImage::from_fn(16, 1, |x, _| Rgb([40 + x as u8 * 13, 20 + x as u8 * 8, 10 + x as u8 * 5]));
        let out = apply(DynamicImage::ImageRgb8(src.clone()), &json!([{"region": "whole", "shadows": 100}]), None).unwrap().to_rgb8();
        for (a, b) in src.pixels().zip(out.pixels()) {
            assert!(luma(b) > luma(a), "brighter");
            assert!(b[0] >= b[1] && b[1] >= b[2], "same channel order (hue): {a:?} -> {b:?}");
        }
        let white = RgbImage::from_pixel(4, 4, Rgb([250, 252, 255]));
        let out = apply(DynamicImage::ImageRgb8(white), &json!([{"whites": 100}]), None).unwrap().to_rgb8();
        assert!(out.pixels().all(|p| p[2] == 255 && p[0] <= 255));
    }

    #[test]
    fn detail_mode_lifts_a_shadow_keeping_its_texture_and_stops_at_the_edge() {
        // Left half a dark textured area, right half bright: a strong edge at x = 64.
        let src = RgbImage::from_fn(128, 64, |x, y| {
            let tex = if (x / 2 + y / 2) % 2 == 0 { 6 } else { 0 };
            if x < 64 { Rgb([30 + tex, 28 + tex, 26 + tex]) } else { Rgb([200, 200, 200]) }
        });
        let lift = json!({"region": "whole", "detail": true, "shadows": 80});
        let out = detail_image(&src, &lift).unwrap();
        let mean = |im: &RgbImage, x0: u32, x1: u32| { let mut s = 0.0; for y in 0..64 { for x in x0..x1 { s += luma(im.get_pixel(x, y)); } } s / (64.0 * (x1 - x0) as f32) };
        assert!(mean(&out, 8, 48) > mean(&src, 8, 48) + 10.0, "the dark side is lifted");
        // The bright side right next to the edge changes as the bright side far from it does (no glow
        // along the edge): within 3 levels.
        let (near, far) = (mean(&out, 66, 72) - mean(&src, 66, 72), mean(&out, 110, 126) - mean(&src, 110, 126));
        assert!((near - far).abs() < 3.0, "no halo: near the edge {near}, far from it {far}");
        // Texture kept: the checker's contrast on the dark side grows with the lift, it does not flatten.
        let contrast = |im: &RgbImage| (luma(im.get_pixel(20, 20)) - luma(im.get_pixel(22, 20))).abs();
        assert!(contrast(&out) >= contrast(&src));
        // Neutral detail entries are no-ops; old entries without the flag are untouched by this mode.
        let neutral = apply(DynamicImage::ImageRgb8(src.clone()), &json!([{"detail": true}]), None).unwrap().to_rgb8();
        assert_eq!(neutral, src);
    }

    #[test]
    fn detail_black_point_clears_a_veil_and_brings_colour_back() {
        // A colourful scene under a grey veil of 60.
        let src = RgbImage::from_fn(64, 64, |x, _| if x < 32 { Rgb([60 + 100, 60 + 20, 60 + 10]) } else { Rgb([60 + 10, 60 + 40, 60 + 90]) });
        let clear = json!({"detail": true, "blacks": -100});
        let out = detail_image(&src, &clear).unwrap();
        let sat = |p: &Rgb<u8>| p.0.iter().max().unwrap() - p.0.iter().min().unwrap();
        assert!(sat(out.get_pixel(5, 5)) > sat(src.get_pixel(5, 5)), "more colour once the veil is gone");
        assert!(out.get_pixel(5, 5)[2] < src.get_pixel(5, 5)[2], "the veil comes off the darkest channel");
        // The same entry without detail mode keeps the old (bend) meaning of Blacks.
        assert_ne!(table(&json!({"blacks": -100})).unwrap(), table(&clear).unwrap());
    }

    #[test]
    fn a_masked_adjustment_changes_only_its_region_and_two_stack() {
        let dir = std::env::temp_dir().join(format!("lap_local_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let photo = dir.join("IMG_0001.jpg");
        let (w, h) = (64u32, 64u32);
        let top = image::GrayImage::from_fn(w, h, |_, y| image::Luma([if y < h / 2 { 255 } else { 0 }]));
        let mut png = Vec::new();
        DynamicImage::ImageLuma8(top).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let sky = crate::t_edit_mask::save(&photo, &png, (w, h), 0.0, false, json!({"kind": "region"})).unwrap();
        let ground = crate::t_edit_mask::save(&photo, &png, (w, h), 0.0, true, json!({"kind": "region"})).unwrap();
        let src = RgbImage::from_pixel(w, h, Rgb([120, 110, 100]));
        let list = json!([{"region": "sky", "mask": sky, "highlights": -100, "shadows": -60},
                          {"region": "background", "mask": ground, "shadows": 100}]);
        let out = apply(DynamicImage::ImageRgb8(src.clone()), &list, Some(&photo)).unwrap().to_rgb8();
        assert!(luma(out.get_pixel(10, 5)) < luma(src.get_pixel(10, 5)), "sky darker");
        assert!(luma(out.get_pixel(10, 60)) > luma(src.get_pixel(10, 60)), "ground lighter");
        // Switching the first off leaves the sky exactly as it was.
        let mut off = list.clone();
        off[0]["enabled"] = json!(false);
        let out = apply(DynamicImage::ImageRgb8(src.clone()), &off, Some(&photo)).unwrap().to_rgb8();
        assert_eq!(out.get_pixel(10, 5), src.get_pixel(10, 5));
        assert!(validate(&list).is_ok());
        assert!(validate(&json!([{"mask": "../x"}])).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
