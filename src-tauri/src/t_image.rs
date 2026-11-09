/**
 * Image processing utilities.
 * project: Lap
 * author:  julyx10
 * date:    2024-08-08
 */
use arboard::Clipboard;
use exif::{In, Reader, Tag};
use fast_image_resize as fir;
use image::{DynamicImage, GenericImageView, ImageFormat, ImageReader, RgbImage};
use little_exif::exif_tag::ExifTag;
use little_exif::filetype::FileExtension;
use little_exif::ifd::ExifTagGroup;
use little_exif::metadata::Metadata as LittleExifMetadata;
use once_cell::sync::Lazy;

use rusqlite::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::Mutex;
use std::time::UNIX_EPOCH;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::{t_jxl, t_libraw, t_utils};
#[allow(unused_imports)]
pub use crate::t_enhance::apply_auto_enhance;

#[derive(Default)]
pub struct CaptureSettings {
    pub exposure_time: Option<String>,
    pub f_number: Option<String>,
    pub focal_length: Option<String>,
    pub iso_speed: Option<String>,
}

/// Reads the core capture settings with the same EXIF reader used when image
/// edits preserve metadata. This covers JPEGs that little_exif accepts but
/// kamadak-exif cannot decode.
pub fn read_capture_settings_with_little_exif(file_path: &str) -> CaptureSettings {
    if !is_jpeg_path(file_path) {
        return CaptureSettings::default();
    }

    panic::catch_unwind(AssertUnwindSafe(|| {
        let metadata = match LittleExifMetadata::new_from_path(Path::new(file_path)) {
            Ok(metadata) => metadata,
            Err(_) => return CaptureSettings::default(),
        };

        CaptureSettings {
            exposure_time: metadata
                .get_tag_by_hex(0x829a, Some(ExifTagGroup::EXIF))
                .next()
                .and_then(little_exif_rational_value)
                .and_then(format_little_exif_shutter_speed),
            f_number: metadata
                .get_tag_by_hex(0x829d, Some(ExifTagGroup::EXIF))
                .next()
                .and_then(little_exif_rational_value)
                .map(|value| format!("f/{value}")),
            focal_length: metadata
                .get_tag_by_hex(0x920a, Some(ExifTagGroup::EXIF))
                .next()
                .and_then(little_exif_rational_value)
                .map(|value| format!("{value} mm")),
            iso_speed: metadata
                .get_tag_by_hex(0x8827, Some(ExifTagGroup::EXIF))
                .next()
                .and_then(little_exif_iso_value)
                .filter(|value| value != "0")
                .or_else(|| {
                    metadata
                        .get_tag_by_hex(0x8833, Some(ExifTagGroup::EXIF))
                        .next()
                        .and_then(little_exif_iso_value)
                        .filter(|value| value != "0")
                }),
        }
    }))
    .unwrap_or_default()
}

fn little_exif_rational_value(tag: &ExifTag) -> Option<f64> {
    let values = match tag {
        ExifTag::ExposureTime(values) | ExifTag::FNumber(values) | ExifTag::FocalLength(values) => {
            values
        }
        _ => return None,
    };
    let value = values.first()?;
    (value.denominator != 0).then(|| value.nominator as f64 / value.denominator as f64)
}

fn little_exif_iso_value(tag: &ExifTag) -> Option<String> {
    match tag {
        ExifTag::ISO(values) => values.first().map(ToString::to_string),
        ExifTag::ISOSpeed(values) => values.first().map(ToString::to_string),
        _ => None,
    }
}

fn format_little_exif_shutter_speed(value: f64) -> Option<String> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    if value >= 1.0 {
        Some(format!("{value} s"))
    } else {
        Some(format!("1/{} s", 1.0 / value))
    }
}

/// Quick probing of image dimensions without loading the entire file
pub fn get_image_dimensions(file_path: &str) -> Result<(u32, u32), String> {
    if t_jxl::is_jxl_path(file_path) {
        return t_jxl::get_jxl_dimensions(file_path);
    }

    if is_ffmpeg_backed_image_path(file_path) {
        let metadata = crate::t_video::get_video_metadata(file_path)?;
        return Ok((metadata.width, metadata.height));
    }

    if is_heic_path(file_path) {
        if let Ok(dimensions) = crate::t_heif::get_heif_dimensions(file_path) {
            return Ok(dimensions);
        }
        if let Ok(metadata) = crate::t_video::get_video_metadata(file_path) {
            if metadata.width > 0 && metadata.height > 0 {
                return Ok((metadata.width, metadata.height));
            }
        }
    }

    // Catch potential panics in the third-party imagesize crate
    let result = panic::catch_unwind(|| imagesize::size(file_path));

    match result {
        Ok(Ok(dimensions)) => {
            let width = dimensions.width as u32;
            let height = dimensions.height as u32;

            if crate::t_libraw::is_tiff_path(file_path) {
                if let Ok((raw_width, raw_height)) = crate::t_libraw::get_raw_dimensions(file_path)
                {
                    if raw_width > width || raw_height > height {
                        return Ok((raw_width, raw_height));
                    }
                }
            }

            Ok((width, height))
        }
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => {
            eprintln!("Panic caught while getting dimensions for: {}", file_path);
            Err(
                "Failed to parse image dimensions due to panic (corrupt or invalid file)"
                    .to_string(),
            )
        }
    }
}

fn get_raw_dimensions_from_exif(file_path: &str) -> Result<Option<(u32, u32)>, String> {
    let exif = match read_exif_permissive(file_path) {
        Some(exif) => exif,
        None => return Ok(None),
    };

    let dimension_tag_pairs = [
        (Tag::PixelXDimension, Tag::PixelYDimension),
        (Tag::ImageWidth, Tag::ImageLength),
    ];

    for (width_tag, height_tag) in dimension_tag_pairs {
        let width = exif
            .get_field(width_tag, In::PRIMARY)
            .and_then(|field| field.value.get_uint(0));
        let height = exif
            .get_field(height_tag, In::PRIMARY)
            .and_then(|field| field.value.get_uint(0));

        if let (Some(width), Some(height)) = (width, height) {
            if width > 0 && height > 0 {
                return Ok(Some((width, height)));
            }
        }
    }

    Ok(None)
}

pub fn read_exif_from_bytes_permissive(data: &[u8]) -> Option<exif::Exif> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut cursor = Cursor::new(data);
        if let Some(exif) = read_exif_container_with_recovery(&mut cursor) {
            return Some(exif);
        }
        if let Some(pos) = data.windows(6).position(|w| w == b"Exif\0\0") {
            let exif_start = pos + 6;
            if exif_start < data.len() {
                if let Some(exif) = read_exif_raw_with_recovery(data[exif_start..].to_vec()) {
                    return Some(exif);
                }
            }
        }
        for sig in [b"II\x2a\x00", b"MM\x00\x2a"] {
            if let Some(pos) = data.windows(4).position(|w| w == sig) {
                if let Some(exif) = read_exif_raw_with_recovery(data[pos..].to_vec()) {
                    return Some(exif);
                }
            }
        }
        None
    }))
    .unwrap_or_else(|_| None)
}

fn read_exif_container_with_recovery(cursor: &mut Cursor<&[u8]>) -> Option<exif::Exif> {
    let mut reader = Reader::new();
    reader.continue_on_error(true);
    read_exif_result_with_recovery(reader.read_from_container(cursor))
}

fn read_exif_raw_with_recovery(data: Vec<u8>) -> Option<exif::Exif> {
    let mut reader = Reader::new();
    reader.continue_on_error(true);
    read_exif_result_with_recovery(reader.read_raw(data))
}

fn read_exif_result_with_recovery(result: Result<exif::Exif, exif::Error>) -> Option<exif::Exif> {
    result
        .or_else(|error| error.distill_partial_result(|_| {}))
        .ok()
}

/// A very aggressive binary scanner that looks for the EXIF Orientation tag (0x0112)
/// directly in the byte stream. This is used as a final fallback for non-standard devices.
pub fn scan_orientation_binary(data: &[u8]) -> Option<i32> {
    // Orientation tag is 0x0112. In TIFF, it's a Short (3) with Count 1.
    // Little Endian: 12 01 03 00 01 00 00 00 [Value] 00 00 00
    // Big Endian:    01 12 00 03 00 00 00 01 00 [Value] 00 00

    // Little Endian search
    if let Some(pos) = data
        .windows(12)
        .position(|w| w[0..8] == [0x12, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00])
    {
        let val = data[pos + 8] as i32;
        if (1..=8).contains(&val) {
            return Some(val);
        }
    }

    // Big Endian search
    if let Some(pos) = data
        .windows(12)
        .position(|w| w[0..8] == [0x01, 0x12, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01])
    {
        let val = data[pos + 9] as i32;
        if (1..=8).contains(&val) {
            return Some(val);
        }
    }

    None
}

pub fn read_exif_permissive(file_path: &str) -> Option<exif::Exif> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        use std::io::{Read, Seek};
        let mut file = File::open(file_path).ok()?;
        let mut header = [0u8; 2];
        file.read_exact(&mut header).ok()?;

        if header != [0xFF, 0xD8] {
            return None;
        }

        loop {
            let mut marker = [0u8; 2];
            if file.read_exact(&mut marker).is_err() {
                break;
            }
            if marker[0] != 0xFF {
                break;
            }
            if marker[1] == 0xD9 || marker[1] == 0xDA {
                break;
            }

            let mut len_bytes = [0u8; 2];
            if file.read_exact(&mut len_bytes).is_err() {
                break;
            }
            let len = u16::from_be_bytes(len_bytes) as usize;
            if len < 2 {
                break;
            }
            let segment_len = len - 2;

            if marker[1] == 0xE1 {
                // APP1
                let mut segment_data = vec![0u8; segment_len];
                if file.read_exact(&mut segment_data).is_err() {
                    break;
                }
                if segment_data.starts_with(b"Exif\0\0") {
                    if let Some(exif) = read_exif_raw_with_recovery(segment_data[6..].to_vec()) {
                        return Some(exif);
                    }
                }
            } else {
                if file
                    .seek(std::io::SeekFrom::Current(segment_len as i64))
                    .is_err()
                {
                    break;
                }
            }
        }
        None
    }))
    .unwrap_or_else(|_| None)
    .or_else(|| {
        let mut file = File::open(file_path).ok()?;
        let mut buffer = vec![0u8; 128 * 1024];
        let n = file.read(&mut buffer).unwrap_or(0);
        read_exif_from_bytes_permissive(&buffer[..n])
    })
}

fn primary_image_orientation(exif: &exif::Exif) -> i32 {
    exif.get_field(Tag::Orientation, In::PRIMARY)
        .and_then(|field| field.value.get_uint(0))
        .filter(|value| (1..=8).contains(value))
        .unwrap_or(1) as i32
}

pub(crate) fn is_rim_camera(exif: &exif::Exif) -> bool {
    let make = exif.get_field(Tag::Make, In::PRIMARY)
        .map(|f| f.display_value().to_string()).unwrap_or_default();
    make.contains("Research In Motion") || make.contains("BlackBerry")
}

// Only this firmware's already-landscape pixels carry the known bogus tag.
pub(crate) fn has_rim_orientation_defect(exif: &exif::Exif, width: u32, height: u32) -> bool {
    let text = |tag| exif.get_field(tag, In::PRIMARY)
        .map(|f| f.display_value().to_string()).unwrap_or_default();
    primary_image_orientation(exif) == 6 && width > height
        && text(Tag::Software).contains("Rim Exif Version1.00a")
        && is_rim_camera(exif)
}

pub(crate) fn normalized_image_orientation(exif: &exif::Exif, width: u32, height: u32) -> i32 {
    if has_rim_orientation_defect(exif, width, height) { 1 } else { primary_image_orientation(exif) }
}

// The firmware repair invalidates its verified sources in migration 20. Keep
// unaffected thumbnails compatible instead of decoding the entire library again.
pub const ORIENTATION_ALGORITHM_VERSION: u32 = 2;
pub const ORIENTATION_CACHE_PREFIX: &str = "o2-";

pub fn get_image_orientation(file_path: &str) -> i32 {
    let data = match File::open(file_path) {
        Ok(mut f) => {
            let mut buf = vec![0u8; 128 * 1024];
            let n = f.read(&mut buf).unwrap_or(0);
            buf.truncate(n);
            buf
        }
        Err(_) => return 1,
    };

    // 1. Try modern logic
    if let Some(exif) = read_exif_from_bytes_permissive(&data) {
        // IFD1 describes the embedded thumbnail, not the main photo. Missing
        // main-image orientation means upright even when the thumbnail rotates.
        if primary_image_orientation(&exif) != 6 { return primary_image_orientation(&exif); }
        let (width, height) = image::image_dimensions(file_path).unwrap_or((0, 0));
        return normalized_image_orientation(&exif, width, height);
    }

    // 2. Industry Fallback: Binary Scan
    // This handles K800i and other phones with broken IFD chains
    scan_orientation_binary(&data).unwrap_or(1)
}

#[cfg(test)]
mod orientation_tests {
    use super::*;

    fn orientation_exif(primary: Option<u16>, thumbnail: u16) -> exif::Exif {
        let mut data = vec![b'I', b'I', 42, 0, 8, 0, 0, 0];
        data.extend_from_slice(&(u16::from(primary.is_some())).to_le_bytes());
        if let Some(value) = primary {
            data.extend_from_slice(&[0x12, 0x01, 3, 0, 1, 0, 0, 0]);
            data.extend_from_slice(&value.to_le_bytes());
            data.extend_from_slice(&[0, 0]);
        }
        let next_ifd = (data.len() + 4) as u32;
        data.extend_from_slice(&next_ifd.to_le_bytes());
        data.extend_from_slice(&[1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0]);
        data.extend_from_slice(&thumbnail.to_le_bytes());
        data.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        read_exif_raw_with_recovery(data).unwrap()
    }

    #[test]
    fn embedded_thumbnail_orientation_does_not_rotate_main_photo() {
        let exif = orientation_exif(None, 8);
        assert!(exif.get_field(Tag::Orientation, In::THUMBNAIL).is_some());
        assert_eq!(primary_image_orientation(&exif), 1);
        assert_eq!(primary_image_orientation(&orientation_exif(Some(6), 8)), 6);
        assert_eq!(primary_image_orientation(&orientation_exif(Some(1), 8)), 1);
    }

    #[test]
    fn lap106_rim_exception_requires_firmware_make_and_landscape_pixels() {
        let make = exif::Field { tag: Tag::Make, ifd_num: In::PRIMARY,
            value: exif::Value::Ascii(vec![b"Research In Motion".to_vec()]) };
        let firmware = exif::Field { tag: Tag::Software, ifd_num: In::PRIMARY,
            value: exif::Value::Ascii(vec![b"Rim Exif Version1.00a".to_vec()]) };
        let orientation = exif::Field { tag: Tag::Orientation, ifd_num: In::PRIMARY,
            value: exif::Value::Short(vec![6]) };
        let parse = |fields: &[&exif::Field]| {
            let mut writer = exif::experimental::Writer::new();
            for f in fields { writer.push_field(f); }
            let mut raw = Cursor::new(Vec::new());
            writer.write(&mut raw, true).unwrap();
            read_exif_raw_with_recovery(raw.into_inner()).unwrap()
        };
        let rim = parse(&[&make, &firmware, &orientation]);
        assert_eq!(normalized_image_orientation(&rim, 960, 720), 1);
        assert_eq!(normalized_image_orientation(&rim, 720, 960), 6);
        assert_eq!(normalized_image_orientation(&rim, 960, 960), 6);
        assert_eq!(normalized_image_orientation(&parse(&[&make, &orientation]), 960, 720), 6);
        assert_eq!(normalized_image_orientation(&parse(&[&firmware, &orientation]), 960, 720), 6);
        assert_eq!(normalized_image_orientation(&orientation_exif(Some(6), 1), 960, 720), 6);
    }

    #[test]
    #[ignore = "Explicit read-only TEST-library corpus header audit; no image decoding or AI"]
    fn audit_test_library_primary_orientation() {
        crate::t_config::set_app_identifier("com.lapcasa.dev");
        let config = crate::t_config::load_app_config().unwrap();
        let root = std::path::PathBuf::from(config.db_storage_dir.unwrap());
        assert!(root.to_string_lossy().replace('\\', "/").ends_with("/test-library"));
        assert_eq!(config.current_library_id, "library");
        assert!(!config.process_library_in_background);
        let conn = rusqlite::Connection::open_with_flags(root.join("library.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let mut query = conn.prepare("SELECT a.id, d.path, a.name, a.e_orientation, a.width, a.height,
            COALESCE(fc.n, 0) FROM afiles a JOIN afolders d ON d.id=a.folder_id
            LEFT JOIN (SELECT file_id, COUNT(*) n FROM faces WHERE duplicate_of IS NULL GROUP BY file_id) fc
            ON fc.file_id=a.id WHERE a.file_type=1 ORDER BY a.id").unwrap();
        let rows = query.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?,
            row.get::<_, String>(2)?, row.get::<_, Option<i32>>(3)?, row.get::<_, Option<u32>>(4)?,
            row.get::<_, Option<u32>>(5)?, row.get::<_, i64>(6)?))).unwrap();
        let mut scanned = 0usize;
        let mut parsed = 0usize;
        let mut changed = Vec::new();
        let mut errors = Vec::new();
        let mut data = vec![0u8; 128 * 1024];
        let start = std::time::Instant::now();
        for row in rows {
            let (id, folder, name, stored, width, height, faces) = row.unwrap();
            let path = std::path::Path::new(&folder).join(&name);
            scanned += 1;
            let read = File::open(&path).and_then(|mut file| file.read(&mut data));
            let n = match read {
                Ok(n) => n,
                Err(error) => { errors.push(serde_json::json!({"id":id,"path":path,"error":error.to_string()})); continue; }
            };
            if let Some(exif) = read_exif_from_bytes_permissive(&data[..n]) {
                parsed += 1;
                let primary = exif.get_field(Tag::Orientation, In::PRIMARY)
                    .and_then(|field| field.value.get_uint(0));
                let old = exif.get_field(Tag::Orientation, In::PRIMARY)
                    .or_else(|| exif.fields().find(|field| field.tag == Tag::Orientation))
                    .and_then(|field| field.value.get_uint(0)).map(|value| value as i32)
                    .unwrap_or_else(|| scan_orientation_binary(&data[..n]).unwrap_or(1));
                let corrected = primary_image_orientation(&exif);
                if old != corrected {
                    changed.push(serde_json::json!({"id":id,"path":path,"old_orientation":old,
                        "corrected_orientation":corrected,"primary_orientation":primary,
                        "thumbnail_orientation":exif.get_field(Tag::Orientation,In::THUMBNAIL)
                            .and_then(|field|field.value.get_uint(0)),
                        "stored_orientation":stored,"stored_width":width,"stored_height":height,
                        "actual_dimensions":image::image_dimensions(&path).ok(),"faces":faces}));
                }
            }
            if scanned % 10000 == 0 { eprintln!("ORIENTATION AUDIT: {} headers, {} affected, {:?}", scanned, changed.len(), start.elapsed()); }
        }
        let face_photos = changed.iter().filter(|row| row["faces"].as_i64().unwrap_or(0) > 0).count();
        let report = serde_json::json!({"scanned_images":scanned,"parsed_exif":parsed,
            "affected_images":changed.len(),"affected_face_photos":face_photos,
            "unreadable":errors.len(),"elapsed_seconds":start.elapsed().as_secs_f64(),
            "changes":changed,"errors":errors});
        std::fs::write(root.join("primary-orientation-corpus-audit-20261005.json"),
            serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        eprintln!("ORIENTATION AUDIT COMPLETE: {}", serde_json::json!({"scanned":scanned,
            "affected":report["affected_images"],"affected_face_photos":face_photos,"unreadable":errors.len()}));
    }
}

pub(crate) fn apply_orientation(img: DynamicImage, orientation: i32) -> DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

/// Minimum short edge as a fraction of `thumbnail_size` (1/4).
const MIN_SHORT_EDGE_DIVISOR: u32 = 4;

/// Computes thumbnail dimensions, keeping the short edge >= `thumbnail_size / 4`
/// (never upscaling); the long edge may exceed `thumbnail_size`.
fn compute_thumbnail_dimensions(width: u32, height: u32, thumbnail_size: u32) -> (u32, u32) {
    if width == 0 || height == 0 || thumbnail_size == 0 {
        return (1, 1);
    }

    if width <= thumbnail_size && height <= thumbnail_size {
        return (width.max(1), height.max(1));
    }

    let long = width.max(height) as f32;
    let short = width.min(height) as f32;
    let min_short_edge = (thumbnail_size / MIN_SHORT_EDGE_DIVISOR).max(1) as f32;
    let fit_scale = thumbnail_size as f32 / long;
    let fit_short = short * fit_scale;
    // Keep the short edge at least min_short_edge, but never upscale it.
    let scale = if fit_short < min_short_edge {
        (short.min(min_short_edge)) / short
    } else {
        fit_scale
    };
    let dst_w = ((width as f32) * scale).round().max(1.0) as u32;
    let dst_h = ((height as f32) * scale).round().max(1.0) as u32;
    (dst_w, dst_h)
}

fn encode_jpeg_rgb8(rgb: &image::RgbImage) -> Result<Vec<u8>, String> {
    crate::t_jpeg::encode_rgb8(rgb, 85)
        .map_err(|e| format!("Failed to encode JPEG thumbnail: {}", e))
}

fn resize_rgb_image_to_jpeg(rgb: image::RgbImage, thumbnail_size: u32) -> Result<Vec<u8>, String> {
    let (src_w, src_h) = rgb.dimensions();
    let (dst_w, dst_h) = compute_thumbnail_dimensions(src_w, src_h, thumbnail_size);

    if src_w == dst_w && src_h == dst_h {
        return encode_jpeg_rgb8(&rgb);
    }

    let src_image =
        fir::images::Image::from_vec_u8(src_w, src_h, rgb.into_raw(), fir::PixelType::U8x3)
            .map_err(|e| format!("Failed to prepare RGB source image for resize: {}", e))?;
    let mut dst_image = fir::images::Image::new(dst_w, dst_h, fir::PixelType::U8x3);
    let mut resizer = fir::Resizer::new();
    let options = fir::ResizeOptions::new()
        .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Bilinear));

    resizer
        .resize(&src_image, &mut dst_image, &options)
        .map_err(|e| format!("Failed to resize RGB thumbnail: {}", e))?;

    let resized = image::RgbImage::from_raw(dst_w, dst_h, dst_image.into_vec())
        .ok_or_else(|| "Failed to build resized RGB image".to_string())?;
    encode_jpeg_rgb8(&resized)
}

fn resize_rgba_image_to_png(
    rgba: image::RgbaImage,
    thumbnail_size: u32,
) -> Result<Vec<u8>, String> {
    let (src_w, src_h) = rgba.dimensions();
    let (dst_w, dst_h) = compute_thumbnail_dimensions(src_w, src_h, thumbnail_size);

    let resized = if src_w == dst_w && src_h == dst_h {
        rgba
    } else {
        let src_image =
            fir::images::Image::from_vec_u8(src_w, src_h, rgba.into_raw(), fir::PixelType::U8x4)
                .map_err(|e| format!("Failed to prepare PNG source image for resize: {}", e))?;
        let mut dst_image = fir::images::Image::new(dst_w, dst_h, fir::PixelType::U8x4);
        let mut resizer = fir::Resizer::new();
        let options = fir::ResizeOptions::new()
            .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Bilinear));

        resizer
            .resize(&src_image, &mut dst_image, &options)
            .map_err(|e| format!("Failed to resize PNG thumbnail: {}", e))?;

        image::RgbaImage::from_raw(dst_w, dst_h, dst_image.into_vec())
            .ok_or_else(|| "Failed to build resized PNG thumbnail".to_string())?
    };

    let mut output = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(resized)
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|e| format!("Failed to encode PNG thumbnail: {}", e))?;
    Ok(output.into_inner())
}

pub(crate) fn should_use_png_thumbnail(file_path: &str) -> bool {
    Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "png" | "gif"))
}

pub fn is_jpeg_path(file_path: &str) -> bool {
    Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "jpe" | "jfif"))
        .unwrap_or(false)
}

fn decode_scaled_jpeg_image(
    file_path: &str,
    _orientation: i32,
    thumbnail_size: u32,
) -> Result<Option<DynamicImage>, String> {
    if !is_jpeg_path(file_path) || thumbnail_size == 0 {
        return Ok(None);
    }

    // Logic: We pass the thumbnail size to libjpeg-turbo, which picks the best 1/8, 1/4, 1/2 scale.
    match crate::t_jpeg::decode_rgb8_scaled(file_path, thumbnail_size, thumbnail_size) {
        Ok((pixels, w, h)) => {
            let img = RgbImage::from_raw(w, h, pixels)
                .ok_or_else(|| "Failed to build RGB image from turbo pixels".to_string())?;
            Ok(Some(DynamicImage::ImageRgb8(img)))
        }
        Err(e) => {
            eprintln!(
                "libjpeg-turbo scaled decode failed for {}: {}",
                file_path, e
            );
            Ok(None) // Fallback to standard decode
        }
    }
}

pub(crate) fn resize_dynamic_image_to_jpeg(
    img: DynamicImage,
    orientation: i32,
    thumbnail_size: u32,
) -> Result<Vec<u8>, String> {
    let adjusted = apply_orientation(img, orientation);

    if !adjusted.color().has_alpha() {
        return resize_rgb_image_to_jpeg(adjusted.to_rgb8(), thumbnail_size);
    }

    let rgba = adjusted.to_rgba8();
    let (src_w, src_h) = rgba.dimensions();
    let (dst_w, dst_h) = compute_thumbnail_dimensions(src_w, src_h, thumbnail_size);

    if src_w == dst_w && src_h == dst_h {
        return encode_jpeg_rgb8(&DynamicImage::ImageRgba8(rgba).to_rgb8());
    }

    let src_image =
        fir::images::Image::from_vec_u8(src_w, src_h, rgba.into_raw(), fir::PixelType::U8x4)
            .map_err(|e| format!("Failed to prepare source image for resize: {}", e))?;
    let mut dst_image = fir::images::Image::new(dst_w, dst_h, fir::PixelType::U8x4);
    let mut resizer = fir::Resizer::new();
    let options = fir::ResizeOptions::new()
        .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Bilinear));

    resizer
        .resize(&src_image, &mut dst_image, &options)
        .map_err(|e| format!("Failed to resize thumbnail: {}", e))?;

    let resized = image::RgbaImage::from_raw(dst_w, dst_h, dst_image.into_vec())
        .ok_or_else(|| "Failed to build resized RGBA image".to_string())?;
    encode_jpeg_rgb8(&DynamicImage::ImageRgba8(resized).to_rgb8())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchThumbnailStats {
    pub processed: usize,
    pub succeeded: usize,
    pub failed: usize,
}

pub fn generate_directory_thumbnails(
    dir_path: &str,
    output_dir: &str,
    thumbnail_size: u32,
) -> Result<BatchThumbnailStats, String> {
    let dir_root = Path::new(dir_path);
    let files: Vec<PathBuf> = WalkDir::new(dir_path)
        .into_iter()
        .filter_entry(crate::t_utils::is_visible_or_root)
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .collect();

    fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create thumbnail output directory: {}", e))?;

    let processed = files.len();
    let results: Vec<bool> = files
        .iter()
        .map(|path| {
            crate::scheduler::cooperative_yield_blocking();
            let path_str = path.to_string_lossy().to_string();
            let Some(file_type) = t_utils::get_file_type(&path_str) else {
                return false;
            };
            if file_type != 1 && file_type != 3 {
                return false;
            }

            let orientation = get_image_orientation(&path_str);
            let thumb = if file_type == 3 {
                get_raw_thumbnail(&path_str, orientation, thumbnail_size, false)
            } else {
                get_image_thumbnail(&path_str, orientation, thumbnail_size)
            };

            let Ok(Some(data)) = thumb else {
                return false;
            };

            let relative_path = path.strip_prefix(dir_root).ok().unwrap_or(path.as_path());
            let output_path = Path::new(output_dir)
                .join(relative_path)
                .with_extension("jpg");
            if let Some(parent) = output_path.parent() {
                if fs::create_dir_all(parent).is_err() {
                    return false;
                }
            }
            fs::write(output_path, data).is_ok()
        })
        .collect();

    let succeeded = results.iter().filter(|ok| **ok).count();
    Ok(BatchThumbnailStats {
        processed,
        succeeded,
        failed: processed.saturating_sub(succeeded),
    })
}

/// Get a thumbnail from an image file path
pub fn get_image_thumbnail(
    file_path: &str,
    orientation: i32,
    thumbnail_size: u32,
) -> Result<Option<Vec<u8>>, String> {
    if t_jxl::is_jxl_path(file_path) {
        return t_jxl::get_jxl_thumbnail(file_path, thumbnail_size);
    }

    if is_ffmpeg_backed_image_path(file_path) {
        return crate::t_video::get_video_thumbnail_sync(file_path, thumbnail_size, None, None);
    }

    if crate::t_libraw::is_tiff_path(file_path) {
        if let Ok(Some(data)) = crate::t_libraw::get_raw_thumbnail(file_path, thumbnail_size, false) {
            return Ok(Some(data));
        }
    }

    let result = panic::catch_unwind(|| {
        let img =
            if let Some(img) = decode_scaled_jpeg_image(file_path, orientation, thumbnail_size)? {
                img
            } else {
                let img_reader = ImageReader::open(file_path)
                    .map_err(|e| format!("Failed to open image: {}", e))?;

                match img_reader.decode() {
                    Ok(img) => img,
                    Err(e) => {
                        // Some formats/variants (notably AVIF) may fail to decode via `image` depending on
                        // the underlying codec support. On macOS, fall back to `sips` which supports
                        // more system formats and returns a JPEG directly.
                        #[cfg(target_os = "macos")]
                        if let Ok(Some(data)) = get_thumbnail_with_sips(file_path, thumbnail_size) {
                            return Ok(Some(data));
                        }
                        // On other platforms, fall back to the bundled FFmpeg sidecar when available.
                        // This is already used for HEIC/HEIF on non-macOS and tends to support more
                        // real-world AVIF variants than the pure-Rust decode path.
                        #[cfg(not(target_os = "macos"))]
                        {
                            if let Ok(Some(data)) = crate::t_video::get_video_thumbnail_sync(
                                file_path,
                                thumbnail_size,
                                None,
                                None,
                            ) {
                                return Ok(Some(data));
                            }
                        }
                        return Err(format!("Failed to decode image: {}", e));
                    }
                }
            };
        if should_use_png_thumbnail(file_path) {
            resize_rgba_image_to_png(
                apply_orientation(img, orientation).to_rgba8(),
                thumbnail_size,
            )
            .map(Some)
        } else {
            resize_dynamic_image_to_jpeg(img, orientation, thumbnail_size).map(Some)
        }
    });

    match result {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "Panic caught while creating image thumbnail for: {}",
                file_path
            );
            Ok(None)
        }
    }
}

fn select_embedded_jpeg(
    file_path: &str,
    thumbnail_size: Option<u32>,
) -> Result<Option<crate::t_embedded_jpeg::Preview>, String> {
    let mut file = File::open(file_path).map_err(|e| e.to_string())?;
    let mut header = [0; 4];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if header == *b"II\x2a\x00" || header == *b"MM\x00\x2a" {
        // TIFF offsets refer to the file, not the bounded metadata buffer.
        let exif = crate::t_embedded_jpeg::read_tiff_metadata(&mut file)?;
        crate::t_embedded_jpeg::select(&mut file, &exif, thumbnail_size)
    } else {
        let exif = match read_exif_permissive(file_path) {
            Some(exif) => exif,
            None => return Ok(None),
        };
        crate::t_embedded_jpeg::select(&mut Cursor::new(exif.buf()), &exif, thumbnail_size)
    }
}

fn select_embedded_jpeg_for_preview(
    file_path: &str,
) -> Result<Option<crate::t_embedded_jpeg::Preview>, String> {
    select_embedded_jpeg(file_path, None)
}

fn select_embedded_jpeg_for_thumbnail(
    file_path: &str,
    thumbnail_size: u32,
) -> Result<Option<crate::t_embedded_jpeg::Preview>, String> {
    select_embedded_jpeg(file_path, Some(thumbnail_size))
}

pub fn get_raw_preview_image(
    file_path: &str,
    prefer_embedded_jpeg: bool,
) -> Result<Option<Vec<u8>>, String> {
    if let Ok(Some(data)) = t_libraw::get_raw_preview_image(file_path, prefer_embedded_jpeg) {
        return Ok(Some(data));
    }

    // Final fallback for unsupported RAW renderers or files without a usable
    // processed output. The selected source still controls the normal path.
    if let Ok(Some(preview)) = select_embedded_jpeg_for_preview(file_path) {
        // A readable JPEG header does not guarantee a decodable payload.
        // Preserve the remaining fallbacks if the embedded image is damaged.
        if let Ok(image) = image::load_from_memory(&preview.data) {
            let image = apply_orientation(image, preview.orientation);
            let buf = crate::t_jpeg::encode_rgb8(&image.to_rgb8(), 85)
                .map_err(|e| format!("Failed to encode embedded RAW preview: {}", e))?;
            return Ok(Some(buf));
        }
    }

    #[cfg(target_os = "macos")]
    if let Ok(Some(data)) = get_thumbnail_with_sips(file_path, 4096) {
        return Ok(Some(data));
    }

    let orientation = get_image_orientation(file_path);

    // Final fallback for formats that can be decoded directly by `image`.
    if let Ok(Some(data)) = get_image_thumbnail(file_path, orientation, 4096) {
        return Ok(Some(data));
    }

    Ok(None)
}

pub fn get_raw_dimensions(file_path: &str) -> Result<(u32, u32), String> {
    if let Ok((width, height, _raw_flip)) = t_libraw::get_raw_dimensions_with_flip(file_path) {
        if width > 0 && height > 0 {
            return Ok((width, height));
        }
    }

    if let Ok((width, height)) = get_image_dimensions(file_path) {
        if width > 0 && height > 0 {
            return Ok((width, height));
        }
    }

    if let Ok(Some((width, height))) = get_raw_dimensions_from_exif(file_path) {
        return Ok((width, height));
    }

    #[cfg(target_os = "macos")]
    if let Ok(Some((width, height))) = get_dimensions_with_sips(file_path) {
        return Ok((width, height));
    }

    if let Ok(Some(preview)) = select_embedded_jpeg_for_preview(file_path) {
        return Ok((preview.width, preview.height));
    }

    Err("Failed to resolve RAW dimensions".to_string())
}

pub fn get_raw_thumbnail(
    file_path: &str,
    orientation: i32,
    thumbnail_size: u32,
    prefer_embedded_jpeg: bool,
) -> Result<Option<Vec<u8>>, String> {
    if let Ok(Some(data)) = t_libraw::get_raw_thumbnail(
        file_path,
        thumbnail_size,
        prefer_embedded_jpeg,
    ) {
        return Ok(Some(data));
    }

    // Fallback: EXIF-based embedded JPEG extraction
    if let Ok(Some(preview)) = select_embedded_jpeg_for_thumbnail(file_path, thumbnail_size) {
        if let Ok(img) = image::load_from_memory(&preview.data) {
            return resize_dynamic_image_to_jpeg(img, preview.orientation, thumbnail_size).map(Some);
        }
    }

    #[cfg(target_os = "macos")]
    if let Ok(Some(data)) = get_thumbnail_with_sips(file_path, thumbnail_size) {
        return Ok(Some(data));
    }

    // Fallback for formats that can be decoded directly by `image`.
    get_image_thumbnail(file_path, orientation, thumbnail_size)
}

/// edit image impl

/// crop data
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CropData {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// resize data
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResizeData {
    width: Option<u32>,
    height: Option<u32>,
}

/// edit params
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EditParams {
    #[serde(rename = "sourceFilePath")]
    source_file_path: String,
    #[serde(rename = "destFilePath")]
    dest_file_path: String,
    #[serde(rename = "outputFormat")]
    output_format: String,
    orientation: i32, // exif orientation value
    #[serde(rename = "flipHorizontal")]
    flip_horizontal: bool,
    #[serde(rename = "flipVertical")]
    flip_vertical: bool,
    rotate: i32,
    #[serde(rename = "cropAngle", default)]
    crop_angle: f32,
    crop: CropData,
    resize: ResizeData,
    quality: Option<u8>,
    // New adjustments
    filter: Option<String>,  // "grayscale", "sepia", "invert"
    brightness: Option<i32>, // -100 to 100
    contrast: Option<f32>,   // -100.0 to 100.0
    blur: Option<f32>,       // sigma > 0
    hue_rotate: Option<i32>, // degrees
    saturation: Option<f32>, // multiplier, 1.0 is normal
    #[serde(rename = "autoEnhance", default)]
    pub(crate) auto_enhance: Option<bool>,
    #[serde(rename = "autoEnhancePreset", default)]
    pub(crate) auto_enhance_preset: Option<u8>,
    #[serde(rename = "redEye", default)]
    red_eye: bool,
    #[serde(rename = "sourceFileId", default)]
    pub(crate) source_file_id: Option<i64>,
    #[serde(default, alias = "tools")]
    picasa: Option<serde_json::Value>,
}

impl EditParams {
    pub(crate) fn scaled_to_preview(&self, raw_w: u32, raw_h: u32, preview_w: u32, preview_h: u32) -> Self {
        let mut scaled = self.clone();
        if raw_w > 0 && raw_h > 0 && preview_w > 0 && preview_h > 0 {
            let (oriented_raw_w, oriented_raw_h) = if (5..=8).contains(&self.orientation) {
                (raw_h as f64, raw_w as f64)
            } else {
                (raw_w as f64, raw_h as f64)
            };
            if (preview_w as f64 - oriented_raw_w).abs() > 1.0 || (preview_h as f64 - oriented_raw_h).abs() > 1.0 {
                let sx = preview_w as f64 / oriented_raw_w;
                let sy = preview_h as f64 / oriented_raw_h;

                if scaled.crop.width > 0 && scaled.crop.height > 0 {
                    scaled.crop.x = ((scaled.crop.x as f64) * sx).round() as u32;
                    scaled.crop.y = ((scaled.crop.y as f64) * sy).round() as u32;
                    scaled.crop.width = (((scaled.crop.width as f64) * sx).round() as u32)
                        .min(preview_w.saturating_sub(scaled.crop.x));
                    scaled.crop.height = (((scaled.crop.height as f64) * sy).round() as u32)
                        .min(preview_h.saturating_sub(scaled.crop.y));
                }
            }
        }
        scaled
    }
}

pub fn export_requires_confirmation(params: &EditParams) -> bool {
    Path::new(&params.dest_file_path).exists()
        || params.source_file_path.replace('\\', "/").to_lowercase() == params.dest_file_path.replace('\\', "/").to_lowercase()
}

/// edit an image and save to dest file
pub async fn edit_image(params: EditParams) -> bool {
    if let Ok(img) = get_edited_image(&params).await {
        let path = Path::new(&params.dest_file_path);
        if path == Path::new(&params.source_file_path)
            && ensure_picasaoriginals_backup(path).is_err() {
            return false;
        }
        let format = match params.output_format.as_str() {
            "png" => image::ImageFormat::Png,
            "webp" => image::ImageFormat::WebP,
            _ => image::ImageFormat::Jpeg,
        };

        // Snapshot original metadata before overwriting the file.
        // For overwrite (source == dest) we must copy the original to a
        // temp location first — once we File::create the destination the
        // original EXIF is gone. For save-as-new the source is untouched.
        let metadata_backup_path = if format == image::ImageFormat::Jpeg
            || format == image::ImageFormat::WebP
        {
            match prepare_metadata_backup_path(&params.source_file_path, &params.dest_file_path) {
                Ok(path) => path,
                Err(_) => return false,
            }
        } else {
            None
        };

        let metadata_source = metadata_backup_path
            .as_ref()
            .map(|p| p.as_path())
            .unwrap_or_else(|| Path::new(&params.source_file_path));

        let quality = params.quality.unwrap_or(80);
        let save_ok = if format == image::ImageFormat::Jpeg {
            if let Ok(file) = std::fs::File::create(path) {
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(file, quality);
                encoder.encode_image(&img).is_ok()
            } else {
                false
            }
        } else {
            img.save_with_format(path, format).is_ok()
        };

        if !save_ok {
            cleanup_metadata_backup(&metadata_backup_path);
            return false;
        }

        if format == image::ImageFormat::Jpeg || format == image::ImageFormat::WebP {
            if let Err(_) = copy_metadata_to_output(metadata_source, path) {
                if metadata_backup_path.is_some() {
                    let _ = fs::copy(metadata_source, path);
                } else {
                    let _ = fs::remove_file(path);
                }
                cleanup_metadata_backup(&metadata_backup_path);
                return false;
            }

            cleanup_metadata_backup(&metadata_backup_path);
        }

        return true;
    }
    false
}

fn prepare_metadata_backup_path(
    source_file_path: &str,
    dest_file_path: &str,
) -> Result<Option<PathBuf>, String> {
    if source_file_path != dest_file_path {
        return Ok(None);
    }

    let source_path = Path::new(source_file_path);
    let extension = source_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("tmp");
    let backup_path = std::env::temp_dir().join(format!(
        "lap-edit-metadata-{}.{}",
        Uuid::new_v4(),
        extension
    ));

    fs::copy(source_path, &backup_path)
        .map_err(|e| format!("Failed to create metadata backup: {}", e))?;

    Ok(Some(backup_path))
}

fn cleanup_metadata_backup(path: &Option<PathBuf>) {
    if let Some(path) = path {
        let _ = fs::remove_file(path);
    }
}

/// Copies metadata from source to destination.
/// Prefers little_exif for JPEGs and falls back to kamadak-exif for RAW formats.
fn copy_metadata_to_output(source_path: &Path, dest_path: &Path) -> Result<(), String> {
    let source_path_buf = source_path.to_path_buf();

    // Check file type to detect RAW formats
    let file_type =
        crate::t_utils::get_file_type(source_path.to_str().unwrap_or_default()).unwrap_or(0);
    let is_raw = file_type == 3;

    let mut little_exif_worked = false;
    let mut little_exif_error = String::new();

    if !is_raw {
        // Use little_exif for standard formats (JPEG/WebP).
        // Wrapped in catch_unwind as little_exif can panic on malformed data.
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            LittleExifMetadata::new_from_path(&source_path_buf)
        }));

        match result {
            Ok(Ok(mut metadata)) => {
                sanitize_edit_output_metadata(&mut metadata);
                if let Err(e) = metadata.write_to_file(dest_path) {
                    little_exif_error = format!("little_exif write failed: {}", e);
                } else {
                    little_exif_worked = true;
                }
            }
            Ok(Err(e)) => {
                little_exif_error = format!("little_exif read failed: {}", e);
            }
            Err(_) => {
                little_exif_error = "little_exif panicked".to_string();
            }
        }
    }

    if little_exif_worked {
        return Ok(());
    }

    // Fallback: use kamadak-exif which has broader RAW support
    match copy_metadata_from_raw_to_jpeg(source_path, dest_path) {
        Ok(()) => Ok(()),
        Err(raw_error) => {
            if is_raw {
                Err(format!("RAW metadata extraction failed: {}", raw_error))
            } else {
                Err(format!(
                    "Metadata copy failed. little_exif: {}; kamadak: {}",
                    little_exif_error, raw_error
                ))
            }
        }
    }
}

/// Removes tags that shouldn't be copied to the edited output (like original orientation and dimensions).
fn sanitize_edit_output_metadata(metadata: &mut LittleExifMetadata) {
    metadata.remove_tag_by_hex_group(0x0112, ExifTagGroup::GENERIC); // Orientation
    metadata.remove_tag_by_hex_group(0x0100, ExifTagGroup::GENERIC); // ImageWidth
    metadata.remove_tag_by_hex_group(0x0101, ExifTagGroup::GENERIC); // ImageLength
    metadata.remove_tag_by_hex_group(0xA002, ExifTagGroup::EXIF); // PixelXDimension
    metadata.remove_tag_by_hex_group(0xA003, ExifTagGroup::EXIF); // PixelYDimension
    metadata.remove_tag_by_hex_group(0x0201, ExifTagGroup::GENERIC); // JPEGInterchangeFormat
    metadata.remove_tag_by_hex_group(0x0202, ExifTagGroup::GENERIC); // JPEGInterchangeFormatLength
}

/// Filter for EXIF fields to copy.
/// We mainly copy PRIMARY IFD and exclude pointers or hardware-specific tags that
/// might be invalidated by the image edit (like StripOffsets or Orientation).
fn should_copy_exif_field(field: &exif::Field) -> bool {
    if field.ifd_num != In::PRIMARY {
        return false;
    }

    !matches!(
        field.tag,
        Tag::ExifIFDPointer
            | Tag::GPSInfoIFDPointer
            | Tag::InteropIFDPointer
            | Tag::StripOffsets
            | Tag::StripByteCounts
            | Tag::TileOffsets
            | Tag::TileByteCounts
            | Tag::JPEGInterchangeFormat
            | Tag::JPEGInterchangeFormatLength
            | Tag::Orientation
            | Tag::ImageWidth
            | Tag::ImageLength
            | Tag::PixelXDimension
            | Tag::PixelYDimension
    )
}

/// Extracts EXIF from a (potentially RAW) source and injects it into a JPEG destination.
/// Includes a three-pass reduction logic to ensure metadata fits within the 64KB JPEG segment limit.
fn copy_metadata_from_raw_to_jpeg(source_path: &Path, dest_path: &Path) -> Result<(), String> {
    let file = File::open(source_path)
        .map_err(|e| format!("Failed to open source metadata file: {}", e))?;
    let mut reader = BufReader::new(file);

    // Try read_from_container first (handles JPEG/TIFF/RAW-TIFF)
    let exif = match Reader::new().read_from_container(&mut reader) {
        Ok(exif) => exif,
        Err(_) => {
            // Fallback: try raw TIFF read (some RAWs are just TIFF structures)
            let data = fs::read(source_path).map_err(|e| e.to_string())?;
            match Reader::new().read_raw(data) {
                Ok(exif) => exif,
                Err(_) => return Ok(()), // Truly no metadata found or unreadable, skip
            }
        }
    };

    // Helper to attempt encoding a set of fields and check if it fits in 64KB
    let encode_and_check = |fields: Vec<&exif::Field>| -> Option<Vec<u8>> {
        let mut writer = exif::experimental::Writer::new();
        for field in fields {
            writer.push_field(field);
        }
        let mut tiff_cursor = Cursor::new(Vec::new());
        if writer.write(&mut tiff_cursor, exif.little_endian()).is_ok() {
            let data = tiff_cursor.into_inner();
            if data.len() <= 65527 {
                // JPEG APP1 max is 65535, minus 8 bytes header
                return Some(data);
            }
        }
        None
    };

    // Pass 1: Attempt to copy all standard fields
    let initial_fields: Vec<&exif::Field> = exif
        .fields()
        .filter(|f| should_copy_exif_field(f))
        .collect();
    let mut exif_data = encode_and_check(initial_fields);

    // Pass 2: If too large, strip typically large vendor blocks (MakerNote, UserComment)
    if exif_data.is_none() {
        let reduced_fields: Vec<&exif::Field> = exif
            .fields()
            .filter(|f| should_copy_exif_field(f))
            .filter(|f| !matches!(f.tag, Tag::MakerNote | Tag::UserComment))
            .collect();
        exif_data = encode_and_check(reduced_fields);
    }

    // Pass 3: If still too large, keep only the most essential photography and GPS tags
    if exif_data.is_none() {
        let essential_fields: Vec<&exif::Field> = exif
            .fields()
            .filter(|f| should_copy_exif_field(f))
            .filter(|f| {
                matches!(
                    f.tag,
                    Tag::Make
                        | Tag::Model
                        | Tag::DateTimeOriginal
                        | Tag::DateTimeDigitized
                        | Tag::ExposureTime
                        | Tag::FNumber
                        | Tag::PhotographicSensitivity
                        | Tag::FocalLength
                        | Tag::LensMake
                        | Tag::LensModel
                        | Tag::ExposureBiasValue
                        | Tag::GPSLatitudeRef
                        | Tag::GPSLatitude
                        | Tag::GPSLongitudeRef
                        | Tag::GPSLongitude
                        | Tag::GPSAltitudeRef
                        | Tag::GPSAltitude
                )
            })
            .collect();
        exif_data = encode_and_check(essential_fields);
    }

    if let Some(data) = exif_data {
        write_jpeg_exif_block(dest_path, &data)
    } else {
        eprintln!("EXIF metadata still too large even after stripping, skipping");
        Ok(())
    }
}

/// Manually injects a TIFF-formatted EXIF block into a JPEG file's APP1 segment.
/// This is used when high-level libraries (like little_exif) fail or are not applicable.
fn write_jpeg_exif_block(dest_path: &Path, exif_tiff_data: &[u8]) -> Result<(), String> {
    let mut file_buffer =
        fs::read(dest_path).map_err(|e| format!("Failed to read destination JPEG: {}", e))?;

    // Clear existing metadata using little_exif if possible
    let _ = LittleExifMetadata::clear_metadata(&mut file_buffer, FileExtension::JPEG);

    if file_buffer.len() < 2 || file_buffer[0] != 0xFF || file_buffer[1] != 0xD8 {
        return Err("Destination file is not a valid JPEG".to_string());
    }

    // Prepare APP1 segment: FF E1 + Length + "Exif\0\0" + TIFF data
    let app1_length = (2 + 6 + exif_tiff_data.len()) as u16;
    let mut app1_segment = Vec::with_capacity(2 + 2 + 6 + exif_tiff_data.len());
    app1_segment.extend_from_slice(&[0xFF, 0xE1]);
    app1_segment.extend_from_slice(&app1_length.to_be_bytes());
    app1_segment.extend_from_slice(b"Exif\0\0");
    app1_segment.extend_from_slice(exif_tiff_data);

    // Reconstruct file: FF D8 + New APP1 + Remainder of file
    let mut output = Vec::with_capacity(file_buffer.len() + app1_segment.len());
    output.extend_from_slice(&file_buffer[..2]);
    output.extend_from_slice(&app1_segment);
    output.extend_from_slice(&file_buffer[2..]);

    fs::write(dest_path, output).map_err(|e| format!("Failed to write destination JPEG: {}", e))
}

/// copy an image to clipboard
pub fn copy_image_to_clipboard(img: DynamicImage) -> bool {
    let (width, height) = img.dimensions();
    let rgba = img.to_rgba8();
    let bytes = rgba.into_raw();

    if let Ok(mut clipboard) = Clipboard::new() {
        let image_data = arboard::ImageData {
            width: width as usize,
            height: height as usize,
            bytes: std::borrow::Cow::Owned(bytes),
        };
        return clipboard.set_image(image_data).is_ok();
    }
    false
}

pub(crate) fn is_heic_path(file_path: &str) -> bool {
    matches!(
        t_utils::get_file_extension(file_path)
            .unwrap_or_default()
            .to_lowercase()
            .as_str(),
        "heic" | "heif" | "hif"
    )
}

fn is_avif_path(file_path: &str) -> bool {
    matches!(
        t_utils::get_file_extension(file_path)
            .unwrap_or_default()
            .to_lowercase()
            .as_str(),
        "avif"
    )
}

pub(crate) fn is_ffmpeg_backed_image_path(file_path: &str) -> bool {
    t_utils::get_file_extension(file_path).is_some_and(|extension| {
        crate::t_common::FFMPEG_BACKED_IMGS
            .iter()
            .any(|item| item.eq_ignore_ascii_case(&extension))
    })
}

fn should_generate_preview_for_file(file_path: &str, file_type: i64) -> bool {
    file_type == 3
        || crate::t_libraw::is_tiff_path(file_path)
        || t_jxl::is_jxl_path(file_path)
        || is_heic_path(file_path)
        || is_ffmpeg_backed_image_path(file_path)
        || is_avif_path(file_path)
}

async fn get_generated_preview_bytes(file_path: &str) -> Result<Option<Vec<u8>>, String> {
    let file_type = t_utils::get_file_type(file_path).unwrap_or(0);

    if file_type == 3 {
        return get_raw_preview_image(file_path, false);
    }

    if t_jxl::is_jxl_path(file_path) {
        return t_jxl::get_jxl_preview_image(file_path, 4096);
    }

    if crate::t_libraw::is_tiff_path(file_path) {
        return match get_raw_preview_image(file_path, false) {
            Ok(Some(data)) => Ok(Some(data)),
            _ => {
                #[cfg(target_os = "macos")]
                {
                    get_thumbnail_with_sips(file_path, 4096)
                }
                #[cfg(not(target_os = "macos"))]
                {
                    Ok(None)
                }
            }
        };
    }

    if is_heic_path(file_path) {
        return crate::t_heif::get_heif_preview(
            file_path,
            get_image_orientation(file_path),
            4096,
        );
    }

    if is_ffmpeg_backed_image_path(file_path) {
        return crate::t_video::get_video_thumbnail(file_path, 4096, None, None).await;
    }

    if is_avif_path(file_path) {
        return get_image_thumbnail(file_path, get_image_orientation(file_path), 4096);
    }

    Ok(None)
}

async fn clipboard_preview_png(file_paths: &[String]) -> Option<Vec<u8>> {
    for file_path in file_paths {
        let file_type = t_utils::get_file_type(file_path).unwrap_or(0);
        if file_type != 1 && file_type != 3 {
            continue;
        }
        let img = if should_generate_preview_for_file(file_path, file_type) {
            let Some(preview) = get_generated_preview_bytes(file_path).await.ok().flatten() else {
                continue;
            };
            let Ok(img) = image::load_from_memory(&preview) else {
                continue;
            };
            img
        } else {
            let Ok(img) = image::open(Path::new(file_path)) else {
                continue;
            };
            img
        };
        let mut png = std::io::Cursor::new(Vec::new());
        if img.write_to(&mut png, image::ImageFormat::Png).is_ok() {
            return Some(png.into_inner());
        }
    }
    None
}

pub async fn copy_files_to_clipboard(
    app_handle: &tauri::AppHandle,
    file_paths: Vec<String>,
) -> Result<usize, String> {
    let file_paths = file_paths
        .into_iter()
        .filter(|path| Path::new(path).is_file())
        // The UI limits copying to 10 content items. RAW+JPEG pairs expand
        // one item into two physical files, so retain both components.
        .take(20)
        .collect::<Vec<_>>();
    if file_paths.is_empty() {
        return Err("No valid files to copy".to_string());
    }
    let preview = clipboard_preview_png(&file_paths).await;
    crate::t_pasteboard::copy_files_and_image(app_handle, &file_paths, preview.as_deref()).await?;
    Ok(file_paths.len())
}

/// copy an edited image to clipboard
pub async fn copy_edited_image_to_clipboard(params: EditParams) -> bool {
    if let Ok(img) = get_edited_image(&params).await {
        return copy_image_to_clipboard(img);
    }
    false
}

/// Rotate an image by an arbitrary angle (degrees), producing an axis-aligned
/// bounding-box output with transparent corners, using bilinear interpolation.
/// Takes `img` by value and uses `into_rgba8` so an already-RGBA source reuses
/// its buffer instead of allocating a second full-image copy.
fn rotate_arbitrary(img: DynamicImage, angle_deg: f32) -> DynamicImage {
    let angle = angle_deg.to_radians();
    let src = img.into_rgba8();
    let (sw, sh) = (src.width() as f32, src.height() as f32);
    let cos = angle.cos().abs();
    let sin = angle.sin().abs();
    let bb_w = (sw * cos + sh * sin).ceil() as u32;
    let bb_h = (sh * cos + sw * sin).ceil() as u32;

    let (dw, dh) = (bb_w as f32, bb_h as f32);
    let cos_a = angle.cos();
    let sin_a = angle.sin();

    let mut dst = image::RgbaImage::new(bb_w, bb_h);
    for (x, y, px) in dst.enumerate_pixels_mut() {
        let dx = x as f32 - dw / 2.0;
        let dy = y as f32 - dh / 2.0;
        // Inverse rotation maps this output pixel back into source space.
        let sx = dx * cos_a + dy * sin_a + sw / 2.0;
        let sy = -dx * sin_a + dy * cos_a + sh / 2.0;

        if sx < 0.0 || sy < 0.0 || sx > sw - 1.0 || sy > sh - 1.0 {
            *px = image::Rgba([0, 0, 0, 0]);
            continue;
        }

        let x0 = sx.floor() as u32;
        let y0 = sy.floor() as u32;
        let x1 = (x0 + 1).min(src.width() - 1);
        let y1 = (y0 + 1).min(src.height() - 1);
        let fx = sx - x0 as f32;
        let fy = sy - y0 as f32;

        let p00 = src.get_pixel(x0, y0).0;
        let p10 = src.get_pixel(x1, y0).0;
        let p01 = src.get_pixel(x0, y1).0;
        let p11 = src.get_pixel(x1, y1).0;

        let mut out = [0u8; 4];
        for i in 0..4 {
            let top = p00[i] as f32 * (1.0 - fx) + p10[i] as f32 * fx;
            let bottom = p01[i] as f32 * (1.0 - fx) + p11[i] as f32 * fx;
            out[i] = (top * (1.0 - fy) + bottom * fy).round() as u8;
        }
        *px = image::Rgba(out);
    }

    DynamicImage::ImageRgba8(dst)
}

/// get an edited image
pub(crate) async fn get_edited_image(params: &EditParams) -> Result<DynamicImage, String> {
    let file_type = t_utils::get_file_type(&params.source_file_path).unwrap_or(0);
    let img = if should_generate_preview_for_file(&params.source_file_path, file_type) {
        let preview = get_generated_preview_bytes(&params.source_file_path)
            .await?
            .ok_or_else(|| "Failed to resolve editable preview image".to_string())?;
        let img = image::load_from_memory(&preview)
            .map_err(|e| format!("Failed to decode editable preview image: {}", e))?;

        img
    } else {
        let path = Path::new(&params.source_file_path);
        let mut img = image::open(path).map_err(|e| e.to_string())?;
        // orientation adjustment based on exif orientation value
        img = apply_orientation(img, params.orientation);
        img
    };

    let params = params.clone();
    tauri::async_runtime::spawn_blocking(move || apply_edit_params(img, &params))
        .await.map_err(|e| e.to_string())?
}

pub(crate) fn apply_edit_params(mut img: DynamicImage, params: &EditParams) -> Result<DynamicImage, String> {
    // Correct the oriented source before crop/resize, using face coordinates
    // in that same space. Display, face crops and export share this pipeline.
    if params.auto_enhance.unwrap_or(false) {
        crate::t_enhance::apply_auto_enhance_preset(&mut img, params.auto_enhance_preset.unwrap_or(1));
    }
    // Red-eye only for the Redeye tool: iAuto 2019 (I'm Feeling Lucky's default) has no red-eye
    // step (that is its "+ RE" variant), and running it on every enhanced photo turned reddish
    // skin around the eyes green (LAP-043).
    if params.red_eye {
        if let Some(file_id)=params.source_file_id {crate::t_enhance::correct_red_eye_for_file(&mut img,file_id)?;}
    }
    if let Some(tools)=&params.picasa {img=crate::t_edit_tools::apply_for(img,tools,Some(Path::new(&params.source_file_path)))?;}
    // 1. Flip
    if params.flip_horizontal {
        img = img.fliph();
    }
    if params.flip_vertical {
        img = img.flipv();
    }

    // 2. Rotate (90° multiples)
    match params.rotate.rem_euclid(360) {
        90 => img = img.rotate90(),
        180 => img = img.rotate180(),
        270 => img = img.rotate270(),
        _ => {}
    }

    // 2b. Arbitrary-angle rotation (straighten), producing an axis-aligned
    // bounding box that the subsequent crop indexes into. Offloaded to a
    // blocking thread: it is a CPU-heavy full-image pass that should not occupy
    // the async executor (matches the project's spawn_blocking convention).
    if params.crop_angle != 0.0 {
        let angle = params.crop_angle;
        img = rotate_arbitrary(img, angle);
    }

    // 3. Crop
    if params.crop.width > 0 && params.crop.height > 0 {
        if params.crop.x.saturating_add(params.crop.width) > img.width()
            || params.crop.y.saturating_add(params.crop.height) > img.height() {
            return Err("Crop is outside the image".into());
        }
        img = img.crop_imm(
            params.crop.x,
            params.crop.y,
            params.crop.width,
            params.crop.height,
        );
    }

    // 4. Resize
    if let (Some(w), Some(h)) = (params.resize.width, params.resize.height) {
        if w > 0 && h > 0 {
            img = img.resize_exact(w, h, image::imageops::FilterType::Lanczos3);
        }
    }

    // 5. Adjustments & Filters
    // NOTE: These implementations match CSS filter spec so preview == saved result.

    // Brightness: CSS brightness(X%) multiplies each channel by X/100.
    // Frontend sends -100..100, meaning factor = (100 + b) / 100.
    if let Some(b) = params.brightness {
        if b != 0 {
            let factor = (100 + b) as f32 / 100.0;
            let mut rgba = img.to_rgba8();
            for pixel in rgba.pixels_mut() {
                pixel[0] = (pixel[0] as f32 * factor).clamp(0.0, 255.0) as u8;
                pixel[1] = (pixel[1] as f32 * factor).clamp(0.0, 255.0) as u8;
                pixel[2] = (pixel[2] as f32 * factor).clamp(0.0, 255.0) as u8;
            }
            img = DynamicImage::ImageRgba8(rgba);
        }
    }

    // Contrast: CSS contrast(X%) scales deviation from 128 gray by X/100.
    // Frontend sends -100..100, meaning factor = (100 + c) / 100.
    if let Some(c) = params.contrast {
        if c != 0.0 {
            let factor = (100.0 + c) / 100.0;
            let mut rgba = img.to_rgba8();
            for pixel in rgba.pixels_mut() {
                pixel[0] = ((pixel[0] as f32 - 128.0) * factor + 128.0).clamp(0.0, 255.0) as u8;
                pixel[1] = ((pixel[1] as f32 - 128.0) * factor + 128.0).clamp(0.0, 255.0) as u8;
                pixel[2] = ((pixel[2] as f32 - 128.0) * factor + 128.0).clamp(0.0, 255.0) as u8;
            }
            img = DynamicImage::ImageRgba8(rgba);
        }
    }

    // Blur
    if let Some(sigma) = params.blur {
        if sigma > 0.0 {
            img = img.blur(sigma);
        }
    }

    // Hue Rotate
    if let Some(hue) = params.hue_rotate {
        if hue != 0 {
            img = img.huerotate(hue);
        }
    }

    // Saturation
    if let Some(saturation) = params.saturation {
        if (saturation - 1.0).abs() > f32::EPSILON {
            let mut rgba = img.to_rgba8();
            for pixel in rgba.pixels_mut() {
                let r = pixel[0] as f32;
                let g = pixel[1] as f32;
                let b = pixel[2] as f32;

                // Simple saturation: blend pixel with its grayscale value
                // Luma = 0.299 * R + 0.587 * G + 0.114 * B
                let luma = 0.299 * r + 0.587 * g + 0.114 * b;

                let new_r = (luma + saturation * (r - luma)).clamp(0.0, 255.0) as u8;
                let new_g = (luma + saturation * (g - luma)).clamp(0.0, 255.0) as u8;
                let new_b = (luma + saturation * (b - luma)).clamp(0.0, 255.0) as u8;

                pixel[0] = new_r;
                pixel[1] = new_g;
                pixel[2] = new_b;
            }
            img = DynamicImage::ImageRgba8(rgba);
        }
    }

    // Filter
    if let Some(filter) = &params.filter {
        match filter.as_str() {
            "grayscale" => {
                img = DynamicImage::ImageLuma8(img.to_luma8());
            }
            "invert" => {
                img.invert();
            }
            "sepia" => {
                // Manual Sepia implementation since standard image crate might not have it exposed simply
                // Formula:
                // R = (r * 0.393) + (g * 0.769) + (b * 0.189)
                // G = (r * 0.349) + (g * 0.686) + (b * 0.168)
                // B = (r * 0.272) + (g * 0.534) + (b * 0.131)
                let mut rgba = img.to_rgba8();
                for pixel in rgba.pixels_mut() {
                    let r = pixel[0] as f32;
                    let g = pixel[1] as f32;
                    let b = pixel[2] as f32;

                    let new_r = (r * 0.393 + g * 0.769 + b * 0.189).min(255.0) as u8;
                    let new_g = (r * 0.349 + g * 0.686 + b * 0.168).min(255.0) as u8;
                    let new_b = (r * 0.272 + g * 0.534 + b * 0.131).min(255.0) as u8;

                    pixel[0] = new_r;
                    pixel[1] = new_g;
                    pixel[2] = new_b;
                    // alpha unchanged
                }
                img = DynamicImage::ImageRgba8(rgba);
            }
            _ => {}
        }
    }

    Ok(img)
}

#[cfg(target_os = "macos")]
pub fn get_thumbnail_with_sips(
    file_path: &str,
    thumbnail_size: u32,
) -> Result<Option<Vec<u8>>, String> {
    use std::fs;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    let temp_dir = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("System clock error: {}", e))?
        .subsec_nanos();
    let temp_file = temp_dir.join(format!("thumb_{}.jpg", nanos));
    let temp_output = temp_file.to_str().ok_or("Invalid temp path")?;

    let output = Command::new("sips")
        .arg("--resampleHeight")
        .arg(thumbnail_size.to_string())
        .arg("-s")
        .arg("format")
        .arg("jpeg")
        .arg(file_path)
        .arg("--out")
        .arg(temp_output)
        .output()
        .map_err(|e| format!("Failed to run sips: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "sips failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let data = fs::read(&temp_file).map_err(|e| format!("Failed to read temp file: {}", e))?;
    let _ = fs::remove_file(temp_file);

    Ok(Some(data))
}

#[cfg(target_os = "macos")]
pub fn get_dimensions_with_sips(file_path: &str) -> Result<Option<(u32, u32)>, String> {
    let output = Command::new("sips")
        .arg("-g")
        .arg("pixelWidth")
        .arg("-g")
        .arg("pixelHeight")
        .arg(file_path)
        .output()
        .map_err(|e| format!("Failed to run sips for dimensions: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "sips dimension probe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;

    for line in stdout.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("pixelWidth:") {
            width = value.trim().parse::<u32>().ok();
        } else if let Some(value) = line.strip_prefix("pixelHeight:") {
            height = value.trim().parse::<u32>().ok();
        }
    }

    match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => Ok(Some((width, height))),
        _ => Ok(None),
    }
}

const FILE_IMAGE_RESULT_CACHE_MAX: usize = 8;

#[derive(Clone)]
struct FileImageCacheEntry {
    signature: (u64, u128),
    prefer_embedded_raw_preview: bool,
    data: Vec<u8>,
}

struct FileImageResultCache {
    entries: HashMap<String, FileImageCacheEntry>,
    order: VecDeque<String>,
}

impl FileImageResultCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(
        &mut self,
        file_path: &str,
        signature: (u64, u128),
        prefer_embedded_raw_preview: bool,
    ) -> Option<Vec<u8>> {
        let entry = self.entries.get(file_path)?;
        if entry.signature != signature
            || entry.prefer_embedded_raw_preview != prefer_embedded_raw_preview
        {
            self.entries.remove(file_path);
            self.order.retain(|item| item != file_path);
            return None;
        }

        self.order.retain(|item| item != file_path);
        self.order.push_back(file_path.to_string());
        Some(entry.data.clone())
    }

    fn insert(
        &mut self,
        file_path: String,
        signature: (u64, u128),
        prefer_embedded_raw_preview: bool,
        data: Vec<u8>,
    ) {
        self.entries.insert(
            file_path.clone(),
            FileImageCacheEntry {
                signature,
                prefer_embedded_raw_preview,
                data,
            },
        );
        self.order.retain(|item| item != &file_path);
        self.order.push_back(file_path);

        while self.order.len() > FILE_IMAGE_RESULT_CACHE_MAX {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
}

static FILE_IMAGE_RESULT_CACHE: Lazy<Mutex<FileImageResultCache>> =
    Lazy::new(|| Mutex::new(FileImageResultCache::new()));

fn get_file_signature(file_path: &str) -> Result<(u64, u128), String> {
    let metadata = fs::metadata(file_path)
        .map_err(|e| format!("Failed to read file metadata for cache: {}", e))?;
    let modified = metadata
        .modified()
        .map_err(|e| format!("Failed to read file modified time for cache: {}", e))?
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Invalid file modified time for cache: {}", e))?
        .as_millis();
    Ok((metadata.len(), modified))
}

pub async fn get_file_image_bytes_cached(
    file_path: &str,
    prefer_embedded_raw_preview: bool,
) -> Result<Vec<u8>, String> {
    let file_type = t_utils::get_file_type(file_path).unwrap_or(0);
    let cache_signature = if should_generate_preview_for_file(file_path, file_type) {
        Some(get_file_signature(file_path)?)
    } else {
        None
    };

    if let Some(signature) = cache_signature {
        if let Ok(mut cache) = FILE_IMAGE_RESULT_CACHE.lock() {
            if let Some(cached) = cache.get(file_path, signature, prefer_embedded_raw_preview) {
                return Ok(cached);
            }
        }
    }

    let image_data = if file_type == 3 {
        get_raw_preview_image(file_path, prefer_embedded_raw_preview)?
            .ok_or_else(|| format!("Failed to resolve RAW preview image: {}", file_path))?
    } else if t_jxl::is_jxl_path(file_path) {
        t_jxl::get_jxl_preview_image(file_path, 4096)?
            .ok_or_else(|| format!("Failed to resolve JXL preview image: {}", file_path))?
    } else if is_heic_path(file_path) {
        crate::t_heif::get_heif_preview(file_path, get_image_orientation(file_path), 4096)?
            .ok_or_else(|| format!("Failed to resolve HEIC preview image: {}", file_path))?
    } else if is_ffmpeg_backed_image_path(file_path) {
        crate::t_video::get_video_thumbnail(file_path, 4096, None, None)
            .await?
            .ok_or_else(|| {
                format!(
                    "Failed to resolve FFmpeg-backed preview image: {}",
                    file_path
                )
            })?
    } else if cfg!(target_os = "linux") && is_avif_path(file_path) {
        get_image_thumbnail(file_path, get_image_orientation(file_path), 4096)?
            .ok_or_else(|| format!("Failed to resolve AVIF preview image: {}", file_path))?
    } else if crate::t_libraw::is_tiff_path(file_path) {
        match get_raw_preview_image(file_path, false) {
            Ok(Some(data)) => data,
            _ => tokio::fs::read(file_path)
                .await
                .map_err(|e| format!("Failed to read the image: {}", e))?,
        }
    } else {
        tokio::fs::read(file_path)
            .await
            .map_err(|e| format!("Failed to read the image: {}", e))?
    };

    if let Some(signature) = cache_signature {
        if let Ok(mut cache) = FILE_IMAGE_RESULT_CACHE.lock() {
            cache.insert(
                file_path.to_string(),
                signature,
                prefer_embedded_raw_preview,
                image_data.clone(),
            );
        }
    }

    Ok(image_data)
}

pub const PREVIEW_MAX_DIMENSION: u32 = 2048;

type PreviewBaseKey = (String, u64, u128, u32);
static PREVIEW_BASE_CACHE: Lazy<Mutex<VecDeque<(PreviewBaseKey, DynamicImage)>>> =
    Lazy::new(|| Mutex::new(VecDeque::new()));

/// Returns an independent, oriented screen-fit base before any edits.
fn load_screen_fit_base(file_path: &str, max_edge: u32) -> Result<DynamicImage, String> {
    if max_edge == 0 { return Err("Preview edge must be positive".into()); }
    let path = fs::canonicalize(file_path).map_err(|e| e.to_string())?;
    let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
    let modified = metadata.modified().map_err(|e| e.to_string())?
        .duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
    let key = (path.to_string_lossy().into_owned(), metadata.len(), modified, max_edge);
    // Serialize misses too, avoiding duplicate simultaneous decodes.
    let mut cache = PREVIEW_BASE_CACHE.lock().map_err(|e| e.to_string())?;
    if let Some(index) = cache.iter().position(|(cached, _)| cached == &key) {
        let entry = cache.remove(index).unwrap();
        let img = entry.1.clone();
        cache.push_back(entry);
        return Ok(img);
    }
    let orientation = get_image_orientation(file_path);
    let img = if is_jpeg_path(file_path) {
        match decode_scaled_jpeg_image(file_path, orientation, max_edge)? {
            Some(img) => img,
            None => image::open(path).map_err(|e| e.to_string())?,
        }
    } else {
        image::open(path).map_err(|e| e.to_string())?
    };
    let mut img = apply_orientation(img, orientation);
    if img.width().max(img.height()) > max_edge {
        img = img.resize(max_edge, max_edge, image::imageops::FilterType::Lanczos3);
    }
    cache.push_back((key, img.clone()));
    while cache.len() > 4 { cache.pop_front(); }
    Ok(img)
}

fn picasaoriginals_path(photo_path: &Path) -> Result<PathBuf, String> {
    let filename = photo_path.file_name().ok_or("Photo path has no filename")?;
    Ok(photo_path.parent().unwrap_or_else(|| Path::new("."))
        .join(".picasaoriginals").join(filename))
}

static PICASAORIGINALS_LOCK: Mutex<()> = Mutex::new(());

/// Preserve the first original, including on volumes without hard-link support.
pub fn ensure_picasaoriginals_backup(photo_path: &Path) -> Result<PathBuf, String> {
    let _guard = PICASAORIGINALS_LOCK.lock().map_err(|e| e.to_string())?;
    let backup = picasaoriginals_path(photo_path)?;
    if backup.try_exists().map_err(|e| e.to_string())? { return Ok(backup); }
    let folder = backup.parent().unwrap();
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let mut source = File::open(photo_path).map_err(|e| e.to_string())?;
    let mut destination = match fs::OpenOptions::new().write(true).create_new(true).open(&backup) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(backup),
        Err(e) => return Err(e.to_string()),
    };
    let result = (|| {
        std::io::copy(&mut source, &mut destination).map_err(|e| e.to_string())?;
        destination.sync_all().map_err(|e| e.to_string())
    })();
    drop(destination);
    if result.is_err() { let _ = fs::remove_file(&backup); }
    result?;
    Ok(backup)
}

/// Whether `.picasaoriginals/<filename>` next to the photo holds its original (LAP-RE-03).
pub fn has_picasaoriginals_backup(photo_path: &Path) -> Result<bool, String> {
    picasaoriginals_path(photo_path)?.try_exists().map_err(|e| e.to_string())
}

pub fn restore_from_picasaoriginals(photo_path: &Path) -> Result<bool, String> {
    let _guard = PICASAORIGINALS_LOCK.lock().map_err(|e| e.to_string())?;
    let backup = picasaoriginals_path(photo_path)?;
    if !backup.try_exists().map_err(|e| e.to_string())? { return Ok(false); }
    fs::copy(backup, photo_path).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Generate an intermediate screen-fit preview buffer (~2048px maximum dimension)
/// suitable for locked 120 FPS slider updates, browser canvas display, and fast thumbnailing.
pub fn generate_screen_fit_preview(
    file_path: &str,
    orientation: i32,
) -> Result<Option<Vec<u8>>, String> {
    let file_type = t_utils::get_file_type(file_path).unwrap_or(0);

    if file_type == 3 {
        if let Ok(Some(data)) = get_raw_preview_image(file_path, true) {
            if let Ok(img) = image::load_from_memory(&data) {
                if img.width() <= PREVIEW_MAX_DIMENSION && img.height() <= PREVIEW_MAX_DIMENSION {
                    return Ok(Some(data));
                }
                return resize_dynamic_image_to_jpeg(img, orientation, PREVIEW_MAX_DIMENSION).map(Some);
            }
            return Ok(Some(data));
        }
    }

    if is_heic_path(file_path) {
        if let Ok(Some(data)) = crate::t_heif::get_heif_preview(file_path, orientation, PREVIEW_MAX_DIMENSION) {
            return Ok(Some(data));
        }
    }

    if t_jxl::is_jxl_path(file_path) {
        if let Ok(Some(data)) = t_jxl::get_jxl_preview_image(file_path, PREVIEW_MAX_DIMENSION) {
            return Ok(Some(data));
        }
    }

    if is_ffmpeg_backed_image_path(file_path) {
        if let Ok(Some(data)) = crate::t_video::get_video_thumbnail_sync(file_path, PREVIEW_MAX_DIMENSION, None, None) {
            return Ok(Some(data));
        }
    }

    if crate::t_libraw::is_tiff_path(file_path) {
        if let Ok(Some(data)) = crate::t_libraw::get_raw_thumbnail(file_path, PREVIEW_MAX_DIMENSION, false) {
            return Ok(Some(data));
        }
    }

    // The in-memory decoded tier complements the persistent encoded preview tier.
    if is_jpeg_path(file_path) {
        let img = load_screen_fit_base(file_path, PREVIEW_MAX_DIMENSION)?;
        return crate::t_jpeg::encode_rgb8(&img.to_rgb8(), 88).map(Some);
    }

    // General fallback for all other formats
    let img_reader = ImageReader::open(file_path)
        .map_err(|e| format!("Failed to open image for preview: {}", e))?;
    let img = img_reader
        .decode()
        .map_err(|e| format!("Failed to decode image for preview: {}", e))?;

    resize_dynamic_image_to_jpeg(img, orientation, PREVIEW_MAX_DIMENSION).map(Some)
}

/// Retrieve or compute a screen-fit preview buffer from the cache.
pub async fn get_or_create_preview_image(
    file_id: i64,
    file_path: &str,
    orientation: i32,
) -> Result<Vec<u8>, String> {
    // JPEG drafts use the metadata-validated decoded tier, including after a
    // source is replaced; the persistent tier remains available to other formats.
    if file_id > 0 && !is_jpeg_path(file_path) {
        if let Some(cached) = crate::t_thumb_cache::get_preview(file_id) {
            return Ok(cached);
        }
    }

    let path_owned = file_path.to_string();
    let preview_bytes = tauri::async_runtime::spawn_blocking(move || {
        generate_screen_fit_preview(&path_owned, orientation)
    })
    .await
    .map_err(|e| e.to_string())??
    .ok_or_else(|| format!("Failed to generate preview for {}", file_path))?;

    if file_id > 0 {
        let _ = crate::t_thumb_cache::put_preview(file_id, &preview_bytes);
    }
    Ok(preview_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn save_in_place_preserves_the_first_original() {
        let root = std::env::temp_dir().join(format!("lap089-save-{}",Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let path = root.join("photo.png");
        RgbImage::from_pixel(8,8,image::Rgb([40,80,120])).save(&path).unwrap();
        let pristine = fs::read(&path).unwrap();
        let params: EditParams = serde_json::from_value(serde_json::json!({
            "sourceFilePath":path.to_str().unwrap(),"destFilePath":path.to_str().unwrap(),"outputFormat":"png",
            "orientation":1,"flipHorizontal":false,"flipVertical":false,"rotate":0,
            "crop":{"x":0,"y":0,"width":0,"height":0},"resize":{"width":null,"height":null},"brightness":20
        })).unwrap();
        assert!(edit_image(params.clone()).await);
        assert_ne!(fs::read(&path).unwrap(),pristine);
        assert!(edit_image(params).await);
        assert_eq!(fs::read(root.join(".picasaoriginals/photo.png")).unwrap(),pristine);
        assert!(restore_from_picasaoriginals(&path).unwrap());
        assert_eq!(fs::read(&path).unwrap(),pristine);
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn preview_cache_hit_and_identical_pixels() {
        let root = std::env::temp_dir().join(format!("lap089-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let photo = root.join("photo.jpg");
        RgbImage::from_fn(80,60,|x,y| image::Rgb([x as u8,y as u8,90])).save(&photo).unwrap();
        let field = exif::Field { tag: Tag::Orientation, ifd_num: In::PRIMARY,
            value: exif::Value::Short(vec![6]) };
        let mut writer = exif::experimental::Writer::new();
        writer.push_field(&field);
        let mut tiff = Cursor::new(Vec::new());
        writer.write(&mut tiff,true).unwrap();
        write_jpeg_exif_block(&photo,&tiff.into_inner()).unwrap();
        let path = photo.to_str().unwrap();
        let first = load_screen_fit_base(path,40).unwrap();
        assert_eq!(first.dimensions(),(30,40));
        let canonical = fs::canonicalize(&photo).unwrap().to_string_lossy().into_owned();
        {
            let mut cache = PREVIEW_BASE_CACHE.lock().unwrap();
            cache.iter_mut().find(|(key,_)| key.0 == canonical).unwrap().1 =
                DynamicImage::ImageRgb8(RgbImage::from_pixel(40,30,image::Rgb([7,8,9])));
        }
        assert_eq!(load_screen_fit_base(path,40).unwrap().to_rgb8().get_pixel(0,0).0,[7,8,9]);
        {
            let mut cache = PREVIEW_BASE_CACHE.lock().unwrap();
            cache.iter_mut().find(|(key,_)| key.0 == canonical).unwrap().1 = first.clone();
        }
        assert_eq!(first.to_rgba8(),load_screen_fit_base(path,40).unwrap().to_rgba8());
        assert_eq!(generate_screen_fit_preview(path,1).unwrap(),generate_screen_fit_preview(path,1).unwrap());
        // Exercise the same preview loader called by render_edit_draft, then
        // apply edits twice: neither call may mutate the cached base.
        let params: EditParams = serde_json::from_value(serde_json::json!({
            "sourceFilePath": path, "destFilePath": "", "outputFormat": "jpeg",
            "orientation": 1, "flipHorizontal": false, "flipVertical": false,
            "rotate": 90, "crop": {"x":0,"y":0,"width":0,"height":0},
            "resize": {"width":null,"height":null}, "filter":"sepia"
        })).unwrap();
        let a = get_or_create_preview_image(0,path,1).await.unwrap();
        let b = get_or_create_preview_image(0,path,1).await.unwrap();
        assert_eq!(apply_edit_params(image::load_from_memory(&a).unwrap(),&params).unwrap().to_rgba8(),
            apply_edit_params(image::load_from_memory(&b).unwrap(),&params).unwrap().to_rgba8());
        assert_eq!(first.to_rgba8(),load_screen_fit_base(path,40).unwrap().to_rgba8());
        // Refresh one entry, then insert beyond capacity and check true LRU eviction.
        for edge in [20,21] { load_screen_fit_base(path,edge).unwrap(); }
        load_screen_fit_base(path,40).unwrap();
        load_screen_fit_base(path,22).unwrap();
        load_screen_fit_base(path,23).unwrap();
        {
            let cache = PREVIEW_BASE_CACHE.lock().unwrap();
            assert_eq!(cache.len(),4);
            assert!(cache.iter().any(|(key,_)| key.0 == canonical && key.3 == 40));
            assert!(!cache.iter().any(|(key,_)| key.0 == canonical && key.3 == 20));
        }
        // A changed file length must invalidate the base even without a timestamp tick.
        RgbImage::from_pixel(11,9,image::Rgb([200,10,20])).save(&photo).unwrap();
        assert_eq!(load_screen_fit_base(path,40).unwrap().dimensions(),(11,9));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn picasaoriginals_preserves_and_restores_pristine_bytes() {
        let root = std::env::temp_dir().join(format!("lap089-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let photo = root.join("photo.jpg");
        fs::write(&photo,b"original").unwrap();
        assert!(!restore_from_picasaoriginals(&photo).unwrap());
        let backup = ensure_picasaoriginals_backup(&photo).unwrap();
        fs::write(&photo,b"edited").unwrap();
        assert_eq!(ensure_picasaoriginals_backup(&photo).unwrap(),backup);
        assert_eq!(fs::read(backup).unwrap(),b"original");
        assert!(restore_from_picasaoriginals(&photo).unwrap());
        assert_eq!(fs::read(photo).unwrap(),b"original");
        fs::remove_dir_all(root).unwrap();
    }
}
