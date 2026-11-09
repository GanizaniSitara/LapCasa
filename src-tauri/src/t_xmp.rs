//! XMP sidecar metadata reader and writer.
//!
//! Provides standard Adobe XMP sidecar generation and parsing for photo
//! ratings and tags/subjects, ensuring compatibility with tools like
//! Adobe Lightroom, Photoshop, Bridge, and Darktable.

use std::fs;
use std::path::{Path, PathBuf};

use crate::t_utils;

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Returns the standard XMP sidecar path for a given file: `{original_filename}.xmp`.
/// E.g. `/photos/image.jpg` -> `/photos/image.jpg.xmp`.
pub fn xmp_path_for_file(file_path: &Path) -> PathBuf {
    let mut p = file_path.as_os_str().to_os_string();
    p.push(".xmp");
    PathBuf::from(p)
}

/// Detects if an XMP sidecar exists next to the photo.
/// Checks `{original_filename}.xmp` (e.g. `img.jpg.xmp`) and `{stem}.xmp` (e.g. `img.xmp`).
/// If both exist, returns the one with the more recent modification time.
pub fn find_xmp_sidecar(file_path: &Path) -> Option<PathBuf> {
    let direct_xmp = xmp_path_for_file(file_path);
    let stem_xmp = file_path.with_extension("xmp");

    let direct_exists = direct_xmp.is_file();
    let stem_exists = stem_xmp.is_file() && stem_xmp != file_path;

    if direct_exists && stem_exists {
        let direct_mtime = direct_xmp.metadata().and_then(|m| m.modified()).ok();
        let stem_mtime = stem_xmp.metadata().and_then(|m| m.modified()).ok();
        if let (Some(dm), Some(sm)) = (direct_mtime, stem_mtime) {
            if sm > dm {
                return Some(stem_xmp);
            }
        }
        Some(direct_xmp)
    } else if direct_exists {
        Some(direct_xmp)
    } else if stem_exists {
        Some(stem_xmp)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// XML escaping / unescaping
// ---------------------------------------------------------------------------

pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn xml_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

// ---------------------------------------------------------------------------
// XMP Generation
// ---------------------------------------------------------------------------

/// Generates a standard Adobe XMP sidecar XML document string containing
/// `xmp:Rating`, `dc:subject`, `lr:hierarchicalSubject`, and optional `dc:description`.
pub fn generate_xmp_xml<S: AsRef<str>>(
    rating: i32,
    tags: &[S],
    comment: Option<&str>,
) -> String {
    let clamped_rating = rating.clamp(0, 5);

    let mut tags_xml = String::new();
    for tag in tags {
        let trimmed = tag.as_ref().trim();
        if !trimmed.is_empty() {
            tags_xml.push_str(&format!("     <rdf:li>{}</rdf:li>\n", xml_escape(trimmed)));
        }
    }

    let subject_block = if tags_xml.is_empty() {
        "   <dc:subject>\n    <rdf:Bag/>\n   </dc:subject>\n   <lr:hierarchicalSubject>\n    <rdf:Bag/>\n   </lr:hierarchicalSubject>\n".to_string()
    } else {
        format!(
            "   <dc:subject>\n    <rdf:Bag>\n{}    </rdf:Bag>\n   </dc:subject>\n   <lr:hierarchicalSubject>\n    <rdf:Bag>\n{}    </rdf:Bag>\n   </lr:hierarchicalSubject>\n",
            tags_xml, tags_xml
        )
    };

    let description_block = match comment {
        Some(c) if !c.trim().is_empty() => {
            format!(
                "   <dc:description>\n    <rdf:Alt>\n     <rdf:li xml:lang=\"x-default\">{}</rdf:li>\n    </rdf:Alt>\n   </dc:description>\n",
                xml_escape(c.trim())
            )
        }
        _ => String::new(),
    };

    format!(
r#"<?xpacket begin="﻿" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="XMP Core 5.6.0">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:lr="http://ns.adobe.com/lightroom/1.0/"
    xmp:Rating="{clamped_rating}">
{subject_block}{description_block}  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#
    )
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Writes standard Adobe XMP sidecar XML given a file path, rating, and list of tags.
/// Outputs to `{original_filename}.xmp`.
pub fn write_xmp_sidecar<P: AsRef<Path>, S: AsRef<str>>(
    file_path: P,
    rating: i32,
    tags: &[S],
) -> Result<PathBuf, String> {
    write_xmp_sidecar_with_comment(file_path, rating, tags, None)
}

/// Writes standard Adobe XMP sidecar XML given a file path, rating, list of tags, and comment.
/// Outputs to `{original_filename}.xmp`.
pub fn write_xmp_sidecar_with_comment<P: AsRef<Path>, S: AsRef<str>>(
    file_path: P,
    rating: i32,
    tags: &[S],
    comment: Option<&str>,
) -> Result<PathBuf, String> {
    let path = file_path.as_ref();
    let xmp_path = xmp_path_for_file(path);

    if let Some(parent) = xmp_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory '{}': {}", parent.display(), e))?;
        }
    }

    let xml = generate_xmp_xml(rating, tags, comment);
    fs::write(&xmp_path, xml.as_bytes())
        .map_err(|e| format!("Failed to write XMP file '{}': {}", xmp_path.display(), e))?;

    Ok(xmp_path)
}

/// Reads current metadata (rating, tags, comments) for `file_id` from the DB and writes to XMP sidecar.
pub fn sync_file_xmp(file_id: i64) -> Result<PathBuf, String> {
    let Some(file) = crate::t_sqlite::AFile::get_file_info(file_id)? else {
        return Err(format!("File id {} not found", file_id));
    };

    let Some(path_str) = file.file_path else {
        return Err(format!("File id {} has no path", file_id));
    };

    let path = Path::new(&path_str);
    if !path.exists() {
        return Ok(xmp_path_for_file(path));
    }

    let tags = crate::t_sqlite::ATag::get_tags_for_file(file_id)?;
    let tag_names: Vec<String> = tags.into_iter().map(|t| t.name).collect();
    let rating = file.rating.unwrap_or(0);

    write_xmp_sidecar_with_comment(path, rating, &tag_names, file.comments.as_deref())
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq, Clone)]
pub struct ParsedXmp {
    pub rating: Option<i32>,
    pub tags: Option<Vec<String>>,
    pub comment: Option<String>,
}

/// Parses metadata from XMP XML content.
pub fn parse_xmp(content: &str) -> ParsedXmp {
    let rating = extract_xmp_rating(content);
    let tags = extract_xmp_tags(content);
    let comment = extract_xmp_comment(content);

    ParsedXmp {
        rating,
        tags,
        comment,
    }
}

fn extract_xmp_rating(content: &str) -> Option<i32> {
    // 1. Check for child element <xmp:Rating> or <Rating>
    for tag_name in &["xmp:Rating", "Rating"] {
        let open_tag = format!("<{}", tag_name);
        if let Some(start_idx) = content.find(&open_tag) {
            let after_open = &content[start_idx + open_tag.len()..];
            if let Some(tag_end) = after_open.find('>') {
                let inner = &after_open[tag_end + 1..];
                let close_tag = format!("</{}>", tag_name);
                if let Some(close_idx) = inner.find(&close_tag) {
                    let value_str = inner[..close_idx].trim();
                    if let Ok(val) = value_str.parse::<f64>() {
                        return Some((val.round() as i32).clamp(0, 5));
                    }
                }
            }
        }
    }

    // 2. Check for attribute xmp:Rating="..." or Rating="..."
    for attr in &["xmp:Rating", "Rating"] {
        for quote in &["\"", "'"] {
            let pattern = format!("{attr}={quote}");
            if let Some(pos) = content.find(&pattern) {
                let val_start = pos + pattern.len();
                if let Some(end_pos) = content[val_start..].find(quote) {
                    let val_str = content[val_start..val_start + end_pos].trim();
                    if let Ok(val) = val_str.parse::<f64>() {
                        return Some((val.round() as i32).clamp(0, 5));
                    }
                }
            }
        }
    }

    None
}

fn extract_xmp_tags(content: &str) -> Option<Vec<String>> {
    // Check dc:subject or subject
    for tag_name in &["dc:subject", "subject"] {
        let open_tag = format!("<{}", tag_name);
        if let Some(start_idx) = content.find(&open_tag) {
            let after_open = &content[start_idx + open_tag.len()..];
            if let Some(tag_end) = after_open.find('>') {
                let tag_header = &after_open[..tag_end];
                if tag_header.ends_with('/') {
                    return Some(Vec::new());
                }

                let inner = &after_open[tag_end + 1..];
                let close_tag = format!("</{}>", tag_name);
                if let Some(close_idx) = inner.find(&close_tag) {
                    let subject_body = &inner[..close_idx];
                    return Some(extract_li_elements(subject_body));
                }
            }
        }
    }

    // Fallback: check lr:hierarchicalSubject
    if let Some(start_idx) = content.find("<lr:hierarchicalSubject") {
        let after_open = &content[start_idx + "<lr:hierarchicalSubject".len()..];
        if let Some(tag_end) = after_open.find('>') {
            let tag_header = &after_open[..tag_end];
            if tag_header.ends_with('/') {
                return Some(Vec::new());
            }
            let inner = &after_open[tag_end + 1..];
            if let Some(close_idx) = inner.find("</lr:hierarchicalSubject>") {
                let body = &inner[..close_idx];
                return Some(extract_li_elements(body));
            }
        }
    }

    None
}

fn extract_li_elements(body: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut remaining = body;
    while let Some(li_start) = remaining.find("<rdf:li").or_else(|| remaining.find("<li")) {
        let after_li = &remaining[li_start..];
        let tag_name = if after_li.starts_with("<rdf:li") { "rdf:li" } else { "li" };
        let open_tag_len = tag_name.len() + 1; // '<' + name
        let after_tag_name = &after_li[open_tag_len..];
        if let Some(close_bracket) = after_tag_name.find('>') {
            let inner = &after_tag_name[close_bracket + 1..];
            let close_tag = format!("</{}>", tag_name);
            if let Some(close_li) = inner.find(&close_tag) {
                let tag_text = xml_unescape(&inner[..close_li]).trim().to_string();
                if !tag_text.is_empty() && !tags.contains(&tag_text) {
                    tags.push(tag_text);
                }
                remaining = &inner[close_li + close_tag.len()..];
                continue;
            }
        }
        remaining = &after_li[1..];
    }
    tags
}

fn extract_xmp_comment(content: &str) -> Option<String> {
    for tag_name in &["dc:description", "description"] {
        let open_tag = format!("<{}", tag_name);
        if let Some(start_idx) = content.find(&open_tag) {
            let after_open = &content[start_idx + open_tag.len()..];
            if let Some(tag_end) = after_open.find('>') {
                let inner = &after_open[tag_end + 1..];
                let close_tag = format!("</{}>", tag_name);
                if let Some(close_idx) = inner.find(&close_tag) {
                    let body = &inner[..close_idx];
                    let lis = extract_li_elements(body);
                    if let Some(first) = lis.into_iter().next() {
                        return Some(first);
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Captions embedded in JPEG files (LAP-047)
// ---------------------------------------------------------------------------

/// Text camera firmware writes into the description fields: not a caption.
fn is_camera_boilerplate(text: &str) -> bool {
    let t = text.split_whitespace().collect::<Vec<_>>().join(" ").to_uppercase();
    const EXACT: &[&str] = &["OLYMPUS DIGITAL CAMERA", "SONY DSC", "DIGITAL CAMERA", "KONICA MINOLTA DIGITAL CAMERA",
        "MINOLTA DIGITAL CAMERA", "KODAK DIGITAL STILL CAMERA", "DIGITAL STILL CAMERA", "SAMSUNG DIGITAL CAMERA",
        "SAMSUNG CAMERA PICTURES", "EXIF_JPEG_PICTURE", "JPEG", "JPG", "IMAGE", "PICTURE", "PHOTO", "DEFAULT",
        "UNTITLED", "NONE", "DCIM", "CAMERA", "LEICA", "SANYO DIGITAL CAMERA", "FUJIFILM DIGITAL CAMERA", "VIVITAR DIGITAL CAMERA"];
    if EXACT.contains(&t.as_str()) { return true; }
    // "<MAKER> DIGITAL CAMERA", "<MAKER> DSC" and the like: a few words ending in a camera word.
    let words = t.split(' ').count();
    words <= 4 && (t.ends_with(" DIGITAL CAMERA") || t.ends_with(" DSC") || t.ends_with(" STILL CAMERA"))
        || t.chars().all(|c| c == '?' || !c.is_alphanumeric())
}

fn clean_caption(raw: &str) -> Option<String> {
    let t = raw.trim_matches(|c: char| c == '\0' || c.is_whitespace());
    (!t.is_empty() && !is_camera_boilerplate(t)).then(|| t.to_string())
}

/// IPTC-IIM Caption-Abstract (2:120) in a Photoshop APP13 block ("Photoshop 3.0" 8BIM resources;
/// the IPTC record is resource 0x0404). UTF-8 when 1:90 says so or the bytes are valid UTF-8,
/// else Latin-1.
fn iptc_caption(app13: &[u8]) -> Option<String> {
    let mut i = app13.strip_prefix(b"Photoshop 3.0\0".as_slice()).map(|_| 14)?;
    while i + 12 <= app13.len() && &app13[i..i + 4] == b"8BIM" {
        let id = u16::from_be_bytes([app13[i + 4], app13[i + 5]]);
        let name_len = app13[i + 6] as usize;
        let mut j = i + 7 + name_len;
        if (name_len + 1) % 2 == 1 { j += 1; }               // the Pascal name is padded to even
        if j + 4 > app13.len() { return None; }
        let size = u32::from_be_bytes([app13[j], app13[j + 1], app13[j + 2], app13[j + 3]]) as usize;
        let data = app13.get(j + 4..j + 4 + size)?;
        if id == 0x0404 {
            let (mut k, mut utf8, mut caption) = (0, false, None);
            while k + 5 <= data.len() && data[k] == 0x1C {
                let (rec, ds) = (data[k + 1], data[k + 2]);
                let len = u16::from_be_bytes([data[k + 3], data[k + 4]]) as usize;
                if len & 0x8000 != 0 { break; }               // extended lengths: never a caption
                let value = data.get(k + 5..k + 5 + len)?;
                if rec == 1 && ds == 90 && value.windows(3).any(|w| w == b"\x1b%G") { utf8 = true; }
                if rec == 2 && ds == 120 { caption = Some(value.to_vec()); }
                k += 5 + len;
            }
            return caption.map(|v| match std::str::from_utf8(&v) {
                Ok(text) => text.to_string(),
                Err(_) if utf8 => String::from_utf8_lossy(&v).into_owned(),
                Err(_) => v.iter().map(|&b| b as char).collect(),
            });
        }
        i = j + 4 + size + size % 2;
    }
    None
}

/// The APP13 (IPTC) block and the XMP packet of a JPEG, reading only its header segments.
fn jpeg_metadata_segments(path: &Path) -> (Option<Vec<u8>>, Option<String>) {
    use std::io::{Read, Seek, SeekFrom};
    let (mut app13, mut xmp) = (None, None);
    let Ok(file) = fs::File::open(path) else { return (None, None) };
    let mut f = std::io::BufReader::new(file);
    let mut head = [0u8; 2];
    if f.read_exact(&mut head).is_err() || head != [0xFF, 0xD8] { return (None, None); }
    for _ in 0..64 {
        let mut marker = [0u8; 2];
        if f.read_exact(&mut marker).is_err() || marker[0] != 0xFF { break; }
        let m = marker[1];
        if m == 0xD8 || m == 0x01 || (0xD0..=0xD7).contains(&m) { continue; }
        if m == 0xDA || m == 0xD9 { break; }                  // image data: no metadata after this
        let mut len = [0u8; 2];
        if f.read_exact(&mut len).is_err() { break; }
        let body_len = (u16::from_be_bytes(len) as usize).saturating_sub(2);
        const XMP_ID: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
        if m == 0xED || m == 0xE1 {
            let mut body = vec![0u8; body_len];
            if f.read_exact(&mut body).is_err() { break; }
            if m == 0xED && body.starts_with(b"Photoshop 3.0\0") { app13 = Some(body); }
            else if m == 0xE1 && body.starts_with(XMP_ID) { xmp = Some(String::from_utf8_lossy(&body[XMP_ID.len()..]).into_owned()); }
        } else if f.seek(SeekFrom::Current(body_len as i64)).is_err() { break; }
    }
    (app13, xmp)
}

/// A caption written into a JPEG (by Picasa and most photo programs), for `afiles.comments`:
/// IPTC Caption-Abstract, else XMP dc:description, else the EXIF ImageDescription the scan has
/// already read. Camera boilerplate ("OLYMPUS DIGITAL CAMERA", "SONY DSC") is not a caption.
/// Other files: None, without opening them.
pub fn embedded_caption(path: &str, exif_description: Option<&str>) -> Option<String> {
    let p = Path::new(path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !matches!(ext.as_str(), "jpg" | "jpeg" | "jpe") { return None; }
    let (app13, xmp) = jpeg_metadata_segments(p);
    app13.as_deref().and_then(iptc_caption).and_then(|c| clean_caption(&c))
        .or_else(|| xmp.as_deref().and_then(extract_xmp_comment).and_then(|c| clean_caption(&c)))
        .or_else(|| exif_description.and_then(clean_caption))
}

// ---------------------------------------------------------------------------
// Indexing synchronization
// ---------------------------------------------------------------------------

/// Checks if an XMP sidecar exists next to `file_path`. If it exists and its modification
/// date is newer than `db_modified_at`, parses `xmp:Rating` and `dc:subject` from it,
/// and updates the database records for `file_id`.
/// Returns Ok(true) if database was updated, Ok(false) otherwise.
pub fn check_and_sync_xmp_sidecar(
    file_id: i64,
    file_path: &str,
    db_modified_at: Option<i64>,
) -> Result<bool, String> {
    let path = Path::new(file_path);
    let Some(xmp_path) = find_xmp_sidecar(path) else {
        return Ok(false);
    };

    let Ok(metadata) = fs::metadata(&xmp_path) else {
        return Ok(false);
    };

    let Some(xmp_mtime) = metadata
        .modified()
        .ok()
        .and_then(|t| t_utils::systemtime_to_timestamp(Some(t)))
    else {
        return Ok(false);
    };

    let db_mtime = db_modified_at.unwrap_or(0);
    if xmp_mtime <= db_mtime {
        return Ok(false);
    }

    let Ok(content) = fs::read_to_string(&xmp_path) else {
        return Ok(false);
    };

    let parsed = parse_xmp(&content);
    let mut updated = false;

    if let Some(rating) = parsed.rating {
        let clamped = rating.clamp(0, 5);
        crate::t_sqlite::AFile::update_column(file_id, "rating", &clamped)
            .map_err(|e| format!("Failed to update rating from XMP: {}", e))?;
        updated = true;
    }

    if let Some(tags) = parsed.tags {
        crate::t_sqlite::ATag::sync_tags_for_file(file_id, &tags)
            .map_err(|e| format!("Failed to sync tags from XMP: {}", e))?;
        updated = true;
    }

    if let Some(comment) = parsed.comment {
        if !comment.trim().is_empty() {
            let _ = crate::t_sqlite::AFile::update_column(file_id, "comments", &comment);
        }
    }

    Ok(updated)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // LAP-047: a minimal JPEG with the given header segments, then image data.
    fn jpeg_with(segments: &[(u8, Vec<u8>)], image_data: &[u8]) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8];
        for (marker, body) in segments {
            out.extend([0xFF, *marker]);
            out.extend(((body.len() + 2) as u16).to_be_bytes());
            out.extend(body);
        }
        out.extend([0xFF, 0xDA, 0x00, 0x02]);
        out.extend(image_data);
        out.extend([0xFF, 0xD9]);
        out
    }

    fn app13_caption(text: &[u8]) -> Vec<u8> {
        let mut iptc = vec![0x1C, 0x02, 0x00, 0x00, 0x02, 0x00, 0x04];       // 2:0 record version
        iptc.extend([0x1C, 0x02, 120]);
        iptc.extend((text.len() as u16).to_be_bytes());
        iptc.extend(text);
        let mut body = b"Photoshop 3.0\0".to_vec();
        body.extend(b"8BIM"); body.extend(0x0404u16.to_be_bytes()); body.extend([0, 0]);   // empty, padded name
        body.extend((iptc.len() as u32).to_be_bytes()); body.extend(&iptc);
        if iptc.len() % 2 == 1 { body.push(0); }
        body
    }

    fn write_temp(name: &str, bytes: &[u8]) -> String {
        let dir = std::env::temp_dir().join(format!("lap047_caption_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn embedded_caption_reads_iptc_then_xmp_then_exif_and_skips_camera_text() {
        let iptc = write_temp("iptc.jpg", &jpeg_with(&[(0xED, app13_caption(b"A picnic by the lake.\0"))], b"data"));
        assert_eq!(embedded_caption(&iptc, Some("SONY DSC")).as_deref(), Some("A picnic by the lake."));
        let mut xmp_body = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
        xmp_body.extend(br#"<x:xmpmeta><rdf:RDF><rdf:Description><dc:description><rdf:Alt><rdf:li xml:lang="x-default">Lunch &amp; a walk</rdf:li></rdf:Alt></dc:description></rdf:Description></rdf:RDF></x:xmpmeta>"#);
        let xmp = write_temp("xmp.jpg", &jpeg_with(&[(0xE1, b"Exif\0\0II*\0".to_vec()), (0xE1, xmp_body)], b"data"));
        assert_eq!(embedded_caption(&xmp, None).as_deref(), Some("Lunch & a walk"));
        let plain = write_temp("plain.jpg", &jpeg_with(&[(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0".to_vec())], b"data"));
        assert_eq!(embedded_caption(&plain, Some("Preparing the picnic")).as_deref(),
            Some("Preparing the picnic"));
        for junk in ["OLYMPUS DIGITAL CAMERA", "SONY DSC", "Jpeg", "  ", "KODAK DIGITAL STILL CAMERA", "NIKON DSC", "????"] {
            assert_eq!(embedded_caption(&plain, Some(junk)), None, "{junk}");
        }
        let junk_iptc = write_temp("junk_iptc.jpg", &jpeg_with(&[(0xED, app13_caption(b"OLYMPUS DIGITAL CAMERA"))], b"data"));
        assert_eq!(embedded_caption(&junk_iptc, Some("A real one")).as_deref(), Some("A real one"));
    }

    #[test]
    fn embedded_caption_ignores_caption_like_bytes_in_image_data_and_non_jpegs() {
        // 1C 02 78 inside the compressed data (seen in a real file) is not an IPTC caption.
        let mut data = vec![0x11; 50];
        data.extend([0x1C, 0x02, 0x78, 0x00, 0x05]);
        data.extend(b"hello");
        let path = write_temp("noise.jpg", &jpeg_with(&[], &data));
        assert_eq!(embedded_caption(&path, None), None);
        let png = write_temp("caption.png", &jpeg_with(&[(0xED, app13_caption(b"not read"))], b"data"));
        assert_eq!(embedded_caption(&png, None), None);
        assert_eq!(embedded_caption("C:/no/such/file.jpg", Some("SONY DSC")), None);
    }

    #[test]
    #[ignore = "Requires explicitly selected disposable caption fixtures"]
    fn embedded_caption_reads_explicit_fixtures() {
        for index in [1, 2] {
            let path=std::env::var(format!("LAP_XMP_FIXTURE_{index}")).expect("Select a disposable JPEG fixture");
            let want=std::env::var(format!("LAP_XMP_EXPECTED_{index}")).expect("Set its expected caption");
            assert_eq!(embedded_caption(&path, None).as_deref(), Some(want.as_str()));
        }
    }

    #[test]
    fn test_xml_escape_and_unescape() {
        let original = "Cats & Dogs <'Best' \"Photos\">";
        let escaped = xml_escape(original);
        assert_eq!(escaped, "Cats &amp; Dogs &lt;&apos;Best&apos; &quot;Photos&quot;&gt;");
        let unescaped = xml_unescape(&escaped);
        assert_eq!(unescaped, original);
    }

    #[test]
    fn test_xmp_path_for_file() {
        let p = Path::new("C:/photos/vacation.jpg");
        assert_eq!(xmp_path_for_file(p), PathBuf::from("C:/photos/vacation.jpg.xmp"));
    }

    #[test]
    fn test_generate_and_parse_xmp() {
        let tags = vec!["vacation".to_string(), "summer & sun".to_string(), "beach".to_string()];
        let xml = generate_xmp_xml(4, &tags, Some("A sunny day"));

        assert!(xml.contains("xmp:Rating=\"4\""));
        assert!(xml.contains("<dc:subject>"));
        assert!(xml.contains("<rdf:li>vacation</rdf:li>"));
        assert!(xml.contains("<rdf:li>summer &amp; sun</rdf:li>"));
        assert!(xml.contains("<lr:hierarchicalSubject>"));
        assert!(xml.contains("<dc:description>"));

        let parsed = parse_xmp(&xml);
        assert_eq!(parsed.rating, Some(4));
        assert_eq!(parsed.tags, Some(vec![
            "vacation".to_string(),
            "summer & sun".to_string(),
            "beach".to_string(),
        ]));
        assert_eq!(parsed.comment.as_deref(), Some("A sunny day"));
    }

    #[test]
    fn test_parse_element_style_rating() {
        let xml = r#"
        <x:xmpmeta xmlns:x="adobe:ns:meta/">
          <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/">
              <xmp:Rating>5</xmp:Rating>
            </rdf:Description>
          </rdf:RDF>
        </x:xmpmeta>
        "#;
        let parsed = parse_xmp(xml);
        assert_eq!(parsed.rating, Some(5));
    }

    #[test]
    fn test_parse_empty_subject_bag() {
        let xml = r#"
        <x:xmpmeta xmlns:x="adobe:ns:meta/">
          <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/">
              <dc:subject>
                <rdf:Bag/>
              </dc:subject>
            </rdf:Description>
          </rdf:RDF>
        </x:xmpmeta>
        "#;
        let parsed = parse_xmp(xml);
        assert_eq!(parsed.tags, Some(Vec::new()));
    }

    #[test]
    fn test_write_xmp_sidecar_file() {
        let temp_dir = std::env::temp_dir().join(format!("xmp_test_{}", std::process::id()));
        fs::create_dir_all(&temp_dir).unwrap();
        let photo_path = temp_dir.join("test_img.jpg");
        fs::write(&photo_path, b"fake image").unwrap();

        let tags = vec!["nature".to_string(), "landscape".to_string()];
        let written_path = write_xmp_sidecar(&photo_path, 3, &tags).unwrap();

        assert_eq!(written_path, temp_dir.join("test_img.jpg.xmp"));
        assert!(written_path.exists());

        let content = fs::read_to_string(&written_path).unwrap();
        let parsed = parse_xmp(&content);
        assert_eq!(parsed.rating, Some(3));
        assert_eq!(parsed.tags, Some(tags));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_find_xmp_sidecar() {
        let temp_dir = std::env::temp_dir().join(format!("xmp_find_test_{}", std::process::id()));
        fs::create_dir_all(&temp_dir).unwrap();
        let photo_path = temp_dir.join("photo.jpg");
        fs::write(&photo_path, b"img").unwrap();

        // 1. Neither exists
        assert_eq!(find_xmp_sidecar(&photo_path), None);

        // 2. Only photo.jpg.xmp exists
        let direct_xmp = temp_dir.join("photo.jpg.xmp");
        fs::write(&direct_xmp, b"direct").unwrap();
        assert_eq!(find_xmp_sidecar(&photo_path), Some(direct_xmp.clone()));

        // 3. Only photo.xmp exists
        fs::remove_file(&direct_xmp).unwrap();
        let stem_xmp = temp_dir.join("photo.xmp");
        fs::write(&stem_xmp, b"stem").unwrap();
        assert_eq!(find_xmp_sidecar(&photo_path), Some(stem_xmp.clone()));

        // 4. Both exist
        fs::write(&direct_xmp, b"direct").unwrap();
        assert!(find_xmp_sidecar(&photo_path).is_some());

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
