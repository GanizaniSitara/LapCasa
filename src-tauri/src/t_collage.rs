//! Picasa's Create Picture Collage (LAP-049): a Grid / Contact Sheet (every photo fitted into an
//! equal cell, with an optional caption strip under it) or a Mosaic (rows of photos at their own
//! shapes, packed to fill the page). Photos are drawn as LapCasa shows them (saved edits, view
//! rotation); the collage is a new JPEG in the first photo's folder, added to the library.
//! Also Make a Poster: one photo enlarged and cut into overlapping sheets for printing.
use crate::t_sqlite::AFile;
use image::{imageops, Rgb, RgbImage};

/// A tile's place on the page: the photo's box, and its caption strip below it (grid only).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub caption_h: u32,
}

pub const MAX_PHOTOS: usize = 200;

/// Grid / Contact Sheet: the column count that gives the largest picture box for 4:3 photos, every
/// cell the same size, the grid centred on the page; `caption_h` is taken from each cell's bottom.
pub fn grid_layout(n: usize, width: u32, height: u32, spacing: u32, captions: bool) -> Vec<Tile> {
    if n == 0 || width == 0 || height == 0 { return Vec::new(); }
    let (w, h, s) = (width as f32, height as f32, spacing as f32);
    let mut best: Option<(f32, usize, usize, f32, f32, f32)> = None;
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let cell_w = (w - s * (cols as f32 + 1.0)) / cols as f32;
        let cell_h = (h - s * (rows as f32 + 1.0)) / rows as f32;
        if cell_w < 8.0 || cell_h < 8.0 { continue; }
        let caption_h = if captions { (cell_h * 0.14).clamp(14.0, 40.0).min(cell_h / 2.0) } else { 0.0 };
        let pic_h = cell_h - caption_h;
        // the area a 4:3 photo gets in the box
        let fit = (cell_w / 4.0).min(pic_h / 3.0);
        if best.is_none_or(|b| fit > b.0) { best = Some((fit, cols, rows, cell_w, cell_h, caption_h)); }
    }
    let Some((_, cols, rows, cell_w, cell_h, caption_h)) = best else { return Vec::new(); };
    let grid_w = cols as f32 * cell_w + (cols as f32 + 1.0) * s;
    let grid_h = rows as f32 * cell_h + (rows as f32 + 1.0) * s;
    let (ox, oy) = ((w - grid_w) / 2.0, (h - grid_h) / 2.0);
    (0..n).map(|i| {
        let (c, r) = ((i % cols) as f32, (i / cols) as f32);
        Tile {
            x: (ox + s + c * (cell_w + s)).round() as u32,
            y: (oy + s + r * (cell_h + s)).round() as u32,
            w: cell_w.floor() as u32,
            h: (cell_h - caption_h).floor() as u32,
            caption_h: caption_h.floor() as u32,
        }
    }).collect()
}

/// Mosaic: the photos in order, in rows that each fill the page's width at the photos' own shapes
/// (width / height = `aspects`); the row count whose stack is closest to the page's height, scaled
/// down to fit if it is taller and centred.
pub fn mosaic_layout(aspects: &[f32], width: u32, height: u32, spacing: u32) -> Vec<Tile> {
    let n = aspects.len();
    if n == 0 || width == 0 || height == 0 { return Vec::new(); }
    let (w, h, s) = (width as f32, height as f32, spacing as f32);
    let aspects: Vec<f32> = aspects.iter().map(|a| if a.is_finite() && *a > 0.05 { *a } else { 1.0 }).collect();
    let total: f32 = aspects.iter().sum();
    let mut best: Option<(f32, Vec<std::ops::Range<usize>>, f32)> = None;
    for k in 1..=n {
        // k rows, cut where the running aspect sum passes each share of the total
        let mut rows = Vec::new();
        let (mut start, mut acc) = (0usize, 0.0f32);
        for (i, a) in aspects.iter().enumerate() {
            acc += a;
            let left_rows = k - rows.len();
            if left_rows > 1 && acc >= total * (rows.len() + 1) as f32 / k as f32 && n - (i + 1) >= left_rows - 1 {
                rows.push(start..i + 1);
                start = i + 1;
            }
        }
        if start < n { rows.push(start..n); }
        let stack: f32 = rows.iter().map(|r| {
            let sum: f32 = aspects[r.clone()].iter().sum();
            (w - s * (r.len() as f32 + 1.0)) / sum
        }).sum::<f32>() + s * (rows.len() as f32 + 1.0);
        let miss = (stack - h).abs();
        if best.as_ref().is_none_or(|b| miss < b.0) { best = Some((miss, rows, stack)); }
    }
    let Some((_, rows, stack)) = best else { return Vec::new(); };
    let scale = (h / stack).min(1.0);
    let used_h = stack * scale;
    let mut y = (h - used_h) / 2.0 + s * scale;
    let mut tiles = Vec::with_capacity(n);
    for r in rows {
        let sum: f32 = aspects[r.clone()].iter().sum();
        let row_h = (w - s * (r.len() as f32 + 1.0)) / sum * scale;
        let row_w = row_h * sum + s * scale * (r.len() as f32 - 1.0);
        let mut x = (w - row_w) / 2.0;
        for i in r {
            let tw = row_h * aspects[i];
            tiles.push(Tile { x: x.round() as u32, y: y.round() as u32, w: tw.round().max(1.0) as u32, h: row_h.round().max(1.0) as u32, caption_h: 0 });
            x += tw + s * scale;
        }
        y += row_h + s * scale;
    }
    tiles
}

pub fn parse_color(hex: &str) -> [u8; 3] {
    let h = hex.trim().trim_start_matches('#');
    let v = |i: usize| h.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).unwrap_or(255);
    if h.len() == 6 { [v(0), v(2), v(4)] } else { [255, 255, 255] }
}

/// The collage page: each photo fitted (grid: centred in its box; mosaic: its box is its shape),
/// and in the grid its caption under it when there is one.
pub fn compose(photos: &[(RgbImage, String)], tiles: &[Tile], width: u32, height: u32, bg: [u8; 3]) -> RgbImage {
    let mut page = RgbImage::from_pixel(width, height, Rgb(bg));
    let dark_bg = (bg[0] as u32 * 299 + bg[1] as u32 * 587 + bg[2] as u32 * 114) / 1000 < 128;
    let fg = if dark_bg { [225, 225, 225] } else { [40, 40, 40] };
    for ((img, caption), t) in photos.iter().zip(tiles) {
        if t.w == 0 || t.h == 0 || img.width() == 0 || img.height() == 0 { continue; }
        let k = (t.w as f32 / img.width() as f32).min(t.h as f32 / img.height() as f32);
        let (pw, ph) = (((img.width() as f32 * k).round() as u32).clamp(1, t.w), ((img.height() as f32 * k).round() as u32).clamp(1, t.h));
        let fitted = imageops::resize(img, pw, ph, imageops::FilterType::CatmullRom);
        // with a caption, the photo and its caption are centred together in the cell (the caption
        // straight under the photo, as on Picasa's contact sheet)
        let top = t.y + (t.h - ph) / 2;
        imageops::overlay(&mut page, &fitted, (t.x + (t.w - pw) / 2) as i64, top as i64);
        if t.caption_h > 0 && !caption.is_empty() {
            if let Some(strip) = caption_strip(caption, t.w, t.caption_h, bg, fg) {
                imageops::overlay(&mut page, &strip, t.x as i64, (top + ph) as i64);
            }
        }
    }
    page
}

/// A caption in Arial, centred (cut with an ellipsis when too long), drawn by Windows GDI.
#[cfg(windows)]
pub fn caption_strip(text: &str, width: u32, height: u32, bg: [u8; 3], fg: [u8; 3]) -> Option<RgbImage> {
    use windows::core::w;
    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::*;
    if width == 0 || height == 0 { return None; }
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() { return None; }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            return None;
        };
        let old_bitmap = SelectObject(dc, bitmap.into());
        let font = CreateFontW(-((height as f32 * 0.62).round() as i32), 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0,
            DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY,
            (DEFAULT_PITCH.0 | FF_SWISS.0) as u32, w!("Arial"));
        let old_font = SelectObject(dc, font.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00FF_FFFF));
        let mut rect = RECT { left: 2, top: 0, right: width as i32 - 2, bottom: height as i32 };
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        DrawTextW(dc, &mut wide, &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
        let _ = GdiFlush();
        // white text on black: its brightness is the text's coverage
        let px = std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize);
        let strip = RgbImage::from_fn(width, height, |x, y| {
            let a = px[((y * width + x) * 4 + 1) as usize] as f32 / 255.0;
            Rgb(std::array::from_fn(|c| (bg[c] as f32 * (1.0 - a) + fg[c] as f32 * a).round() as u8))
        });
        SelectObject(dc, old_font);
        SelectObject(dc, old_bitmap);
        let _ = DeleteObject(font.into());
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        Some(strip)
    }
}

#[cfg(not(windows))]
pub fn caption_strip(_text: &str, _width: u32, _height: u32, _bg: [u8; 3], _fg: [u8; 3]) -> Option<RgbImage> {
    None
}

/// A photo as LapCasa shows it, at preview size (2048 px): saved edits, or the view rotation.
async fn photo_as_shown(file: &AFile) -> Result<RgbImage, String> {
    let id = file.id.ok_or("Photo has no id")?;
    let data = crate::t_edit_recipe::render(id, false).await?;
    let mut img = image::load_from_memory(&data).map_err(|e| e.to_string())?.to_rgb8();
    let doc = crate::t_edit_recipe::state(file.edits.as_deref());
    let edited = doc["recipe"].is_object() || doc["auto_enhance"].as_bool().unwrap_or(false);
    if !edited {
        img = match file.rotate.unwrap_or(0).rem_euclid(360) {
            90 => imageops::rotate90(&img),
            180 => imageops::rotate180(&img),
            270 => imageops::rotate270(&img),
            _ => img,
        };
    }
    Ok(img)
}

/// Create Picture Collage: `layout` "grid" (Contact Sheet) or "mosaic", a `width` x `height` page.
/// Saves Collage-YYYYMMDD-HHMMSS.jpg in the first photo's folder, adds it to the library, returns
/// its path.
pub async fn create(file_ids: Vec<i64>, layout: &str, width: u32, height: u32, spacing: u32, bg_color: &str, include_captions: bool) -> Result<String, String> {
    if !(320..=8000).contains(&width) || !(320..=8000).contains(&height) { return Err("Page size must be 320 to 8000 px".into()); }
    let files: Vec<AFile> = file_ids.iter().take(MAX_PHOTOS).filter_map(|&id| AFile::get_file_info(id).ok().flatten())
        .filter(|f| matches!(f.file_type, Some(1) | Some(3))).collect();
    if files.is_empty() { return Err("Select one or more photos for the collage".into()); }
    let mut photos = Vec::with_capacity(files.len());
    for f in &files {
        if let Ok(img) = photo_as_shown(f).await { photos.push((img, f.name.clone())); }
    }
    if photos.is_empty() { return Err("None of the photos could be read".into()); }
    let spacing = spacing.min(64);
    let tiles = if layout == "mosaic" {
        mosaic_layout(&photos.iter().map(|(i, _)| i.width() as f32 / i.height() as f32).collect::<Vec<_>>(), width, height, spacing)
    } else {
        grid_layout(photos.len(), width, height, spacing, include_captions)
    };
    let bg = parse_color(bg_color);
    let folder_id = files[0].folder_id;
    let dir = std::path::Path::new(files[0].file_path.as_deref().ok_or("Photo has no path")?)
        .parent().ok_or("Photo has no folder")?.to_path_buf();
    let jpeg = tauri::async_runtime::spawn_blocking(move || {
        let page = compose(&photos, &tiles, width, height, bg);
        crate::t_jpeg::encode_rgb8(&page, 92)
    }).await.map_err(|e| e.to_string())??;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut path = dir.join(format!("Collage-{stamp}.jpg"));
    let mut k = 2;
    while path.exists() { path = dir.join(format!("Collage-{stamp}-{k}.jpg")); k += 1; }
    std::fs::write(&path, jpeg).map_err(|e| format!("Could not save the collage: {e}"))?;
    let path_str = path.to_string_lossy().into_owned();
    AFile::add_to_db(folder_id, &path_str, 1, chrono::Utc::now().timestamp_millis())?;
    Ok(path_str)
}

/// One sheet of a poster: the part of the photo it covers (source pixels, overlap included), its
/// place in the grid (1-based) and the size it is saved at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PosterTile {
    pub row: u32,
    pub col: u32,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub out_w: u32,
    pub out_h: u32,
}

/// The enlargement stops once a sheet's longest edge reaches this (A4 at about 400 dpi).
const POSTER_TILE_MAX: u32 = 4800;

/// Make a Poster's cut: the photo (w x h) enlarged max(cols, rows) times (2 x 2 is a 400% poster)
/// and split into cols x rows sheets, each reaching overlap_px (saved pixels) into its neighbours so
/// the sheets can be trimmed and glued. The enlargement stops where a sheet would pass
/// POSTER_TILE_MAX, and a photo is never shrunk.
pub fn poster_tiles(w: u32, h: u32, cols: u32, rows: u32, overlap_px: u32) -> Vec<PosterTile> {
    if w < cols || h < rows || cols == 0 || rows == 0 { return Vec::new(); }
    let (cell_w, cell_h) = (w as f64 / cols as f64, h as f64 / rows as f64);
    let scale = (cols.max(rows) as f64).min(POSTER_TILE_MAX as f64 / cell_w.max(cell_h)).max(1.0);
    let ov = overlap_px as f64 / scale;
    let span = |i: u32, n: u32, cell: f64, len: u32| {
        let a = (i as f64 * cell - if i > 0 { ov } else { 0.0 }).max(0.0).round() as u32;
        let b = ((i + 1) as f64 * cell + if i + 1 < n { ov } else { 0.0 }).min(len as f64).round() as u32;
        (a, b.max(a + 1))
    };
    let mut tiles = Vec::with_capacity((cols * rows) as usize);
    for r in 0..rows {
        let (y0, y1) = span(r, rows, cell_h, h);
        for c in 0..cols {
            let (x0, x1) = span(c, cols, cell_w, w);
            tiles.push(PosterTile {
                row: r + 1, col: c + 1, x: x0, y: y0, w: x1 - x0, h: y1 - y0,
                out_w: ((x1 - x0) as f64 * scale).round() as u32, out_h: ((y1 - y0) as f64 * scale).round() as u32,
            });
        }
    }
    tiles
}

/// Tools > Make a Poster: the photo as LapCasa shows it, cut into cols x rows enlarged sheets saved
/// as Poster-<stem>-r<R>c<C>.jpg in a new "Poster-<stem>" folder beside the photo ("Poster-<stem>
/// (2)" when that exists, so an earlier poster is never mixed in or overwritten). Returns the sheets.
pub async fn create_poster(file_id: i64, cols: u32, rows: u32, overlap_px: u32) -> Result<Vec<String>, String> {
    if !(1..=8).contains(&cols) || !(1..=8).contains(&rows) || cols * rows < 2 { return Err("A poster is 2 to 64 sheets".into()); }
    let file = AFile::get_file_info(file_id)?.ok_or("Photo not found")?;
    let src = std::path::PathBuf::from(file.file_path.as_deref().filter(|p| !p.is_empty()).ok_or("Photo has no path")?);
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "photo".into());
    let parent = src.parent().ok_or("Photo has no folder")?;
    let mut dir = parent.join(format!("Poster-{stem}"));
    let mut k = 2;
    while dir.exists() { dir = parent.join(format!("Poster-{stem} ({k})")); k += 1; }
    let img = crate::t_edit_recipe::image_as_shown(file_id).await?.to_rgb8();
    let tiles = poster_tiles(img.width(), img.height(), cols, rows, overlap_px.min(400));
    if tiles.is_empty() { return Err("The photo is too small for a poster".into()); }
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
        let mut saved = Vec::with_capacity(tiles.len());
        for t in tiles {
            let part = imageops::crop_imm(&img, t.x, t.y, t.w, t.h).to_image();
            let sheet = imageops::resize(&part, t.out_w, t.out_h, imageops::FilterType::CatmullRom);
            let path = dir.join(format!("Poster-{stem}-r{}c{}.jpg", t.row, t.col));
            std::fs::write(&path, crate::t_jpeg::encode_rgb8(&sheet, 95)?).map_err(|e| format!("Could not save {}: {e}", path.display()))?;
            saved.push(path.to_string_lossy().into_owned());
        }
        Ok(saved)
    }).await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inside(t: &Tile, w: u32, h: u32) -> bool { t.x + t.w <= w && t.y + t.h + t.caption_h <= h }
    fn overlap(a: &Tile, b: &Tile) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h + b.caption_h && b.y < a.y + a.h + a.caption_h
    }

    #[test]
    fn grid_fits_every_photo_in_equal_cells_inside_the_page_with_captions_below() {
        for n in [1usize, 2, 5, 12, 37] {
            for captions in [false, true] {
                let tiles = grid_layout(n, 1920, 1080, 8, captions);
                assert_eq!(tiles.len(), n);
                assert!(tiles.iter().all(|t| inside(t, 1920, 1080)), "{n} {captions}");
                assert!(tiles.iter().all(|t| t.w == tiles[0].w && t.h == tiles[0].h));
                assert!(tiles.iter().all(|t| (t.caption_h > 0) == captions));
                for (i, a) in tiles.iter().enumerate() { for b in &tiles[i + 1..] { assert!(!overlap(a, b), "{n}: {a:?} {b:?}"); } }
            }
        }
        // 12 photos on a 16:9 page: 4 or more columns, not one long strip
        let cols = grid_layout(12, 1920, 1080, 8, false).iter().filter(|t| t.y == grid_layout(12, 1920, 1080, 8, false)[0].y).count();
        assert!((4..=6).contains(&cols), "{cols}");
    }

    #[test]
    fn mosaic_keeps_each_photos_shape_and_fills_the_page() {
        let aspects = [1.5f32, 0.75, 1.333, 1.0, 1.778, 0.667, 1.5, 1.5, 0.8];
        let tiles = mosaic_layout(&aspects, 2400, 1800, 6);
        assert_eq!(tiles.len(), aspects.len());
        for (t, a) in tiles.iter().zip(aspects) {
            assert!(inside(t, 2400, 1800), "{t:?}");
            assert!((t.w as f32 / t.h as f32 - a).abs() < 0.03 * a, "{t:?} {a}");
        }
        for (i, a) in tiles.iter().enumerate() { for b in &tiles[i + 1..] { assert!(!overlap(a, b), "{a:?} {b:?}"); } }
        // the mosaic fills the page one way (rows across it, or, scaled down to fit, its height)
        let (left, right) = (tiles.iter().map(|t| t.x).min().unwrap(), tiles.iter().map(|t| t.x + t.w).max().unwrap());
        let (top, bottom) = (tiles.iter().map(|t| t.y).min().unwrap(), tiles.iter().map(|t| t.y + t.h).max().unwrap());
        assert!(right - left > 2400 - 30 || bottom - top > 1800 - 30, "{left}..{right} x {top}..{bottom}");
        assert!(right - left > 2400 * 7 / 10 && bottom - top > 1800 * 7 / 10, "{left}..{right} x {top}..{bottom}");
    }

    #[test]
    fn compose_draws_photos_on_the_background_and_captions_under_them() {
        let red = RgbImage::from_pixel(400, 300, Rgb([220, 20, 20]));
        let blue = RgbImage::from_pixel(300, 400, Rgb([20, 20, 220]));
        let photos = vec![(red, "IMG_0001.jpg".to_string()), (blue, "IMG_0002.jpg".to_string())];
        let tiles = grid_layout(2, 1000, 600, 10, true);
        let page = compose(&photos, &tiles, 1000, 600, [255, 255, 255]);
        let centre = |t: &Tile| *page.get_pixel(t.x + t.w / 2, t.y + t.h / 2);
        assert_eq!(centre(&tiles[0]), Rgb([220, 20, 20]));
        assert_eq!(centre(&tiles[1]), Rgb([20, 20, 220]));
        assert_eq!(*page.get_pixel(2, 2), Rgb([255, 255, 255]));
        #[cfg(windows)]
        {
            // the first photo's caption: dark text on the white background, straight under the photo
            let t = tiles[0];
            let text: Vec<(u32, u32)> = (t.x..t.x + t.w).flat_map(|x| (t.y..t.y + t.h + t.caption_h).map(move |y| (x, y)))
                .filter(|&(x, y)| { let p = page.get_pixel(x, y); p[0] < 128 && p[1] < 128 }).collect();
            assert!(text.len() > 30, "caption pixels {}", text.len());
            let photo_bottom = (t.y..t.y + t.h).rev().find(|&y| page.get_pixel(t.x + t.w / 2, y)[0] == 220).unwrap();
            let text_top = text.iter().map(|&(_, y)| y).min().unwrap();
            assert!(text_top > photo_bottom && text_top - photo_bottom < t.caption_h, "photo ends {photo_bottom}, caption starts {text_top}");
        }
        assert_eq!(parse_color("#18181b"), [0x18, 0x18, 0x1b]);
        assert_eq!(parse_color("bogus"), [255, 255, 255]);
    }

    #[test]
    fn poster_sheets_cover_the_photo_enlarged_with_overlap_between_neighbours() {
        for (cols, rows) in [(2u32, 2u32), (3, 3), (4, 4)] {
            let tiles = poster_tiles(1200, 900, cols, rows, 60);
            assert_eq!(tiles.len(), (cols * rows) as usize);
            let scale = cols as f64;
            for t in &tiles {
                assert!(t.x + t.w <= 1200 && t.y + t.h <= 900);
                assert!((t.out_w as f64 / t.w as f64 - scale).abs() < 0.05, "{cols}x{rows}: enlarged {scale}x");
                let inner_x = (t.col > 1) as u32 + (t.col < cols) as u32;
                let expect_w = 1200.0 / cols as f64 * scale + inner_x as f64 * 60.0;
                assert!((t.out_w as f64 - expect_w).abs() <= scale + 1.0, "sheet width {} vs {expect_w}", t.out_w);
            }
            // Every source pixel column and row is on some sheet; neighbours share the overlap.
            for x in 0..1200 { assert!(tiles.iter().any(|t| t.row == 1 && t.x <= x && x < t.x + t.w)); }
            for y in 0..900 { assert!(tiles.iter().any(|t| t.col == 1 && t.y <= y && y < t.y + t.h)); }
            let (a, b) = (&tiles[0], &tiles[1]);
            assert_eq!(a.x + a.w - b.x, (2.0 * 60.0 / scale).round() as u32);
        }
        // A large photo is enlarged only until a sheet reaches the cap, and never shrunk.
        let big = poster_tiles(8000, 6000, 2, 2, 0);
        assert!(big.iter().all(|t| t.out_w.max(t.out_h) <= POSTER_TILE_MAX && t.out_w >= t.w));
        let huge = poster_tiles(20000, 15000, 2, 2, 0);
        assert!(huge.iter().all(|t| t.out_w == t.w && t.out_h == t.h));
        assert!(poster_tiles(1, 1, 2, 2, 0).is_empty());
    }
}
