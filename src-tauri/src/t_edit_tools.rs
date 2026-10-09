//! Editing tool recipes applied to an in-memory image, never to source files.
use base64::Engine;
use image::{DynamicImage, GenericImageView, Rgb};
use serde_json::Value;

fn number(v: &Value, key: &str, default: f32) -> f32 {
    v[key].as_f64().unwrap_or(default as f64) as f32
}
fn channel(v: f32) -> u8 {
    v.clamp(0.0, 255.0).round() as u8
}
pub(crate) fn apply(mut img: DynamicImage, tools: &Value) -> Result<DynamicImage, String> {
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
            let l = (p[0] as f32 * 0.299 + p[1] as f32 * 0.587 + p[2] as f32 * 0.114) / 255.0;
            let highlight_delta = if highlights < 0.0 {
                // Compress only the top 20%, joining with slope 1 at 80%.
                // This monotonic shoulder darkens highlights without inventing detail.
                let top = (l - 0.8).max(0.0);
                let strength = -highlights * 4.0;
                255.0 * (top / (1.0 + strength * top / 0.2) - top)
            } else {
                highlights * 90.0 * l.powi(2)
            };
            let delta = fill * 90.0 * (1.0 - l).powi(2) + highlight_delta
                - shadows * 90.0 * (1.0 - l).powi(2);
            for c in 0..3 {
                p[c] = channel(
                    p[c] as f32 * gains[c]
                        + delta
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
            img = apply_effect(img, effect)?;
        }
    }
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

fn apply_effect(img: DynamicImage, effect: &Value) -> Result<DynamicImage, String> {
    let name = effect["name"].as_str().ok_or("Missing effect name")?;
    let amount = number(effect, "amount", 0.5).clamp(0.0, 1.0);
    if name == "Sharpen" {
        return Ok(img.unsharpen(1.0 + amount * 2.0, (amount * 20.0) as i32));
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
    fn every_effect_is_repeatable_and_preserves_geometry() {
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
