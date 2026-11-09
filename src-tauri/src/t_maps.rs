//! Read-only local raster MBTiles. This module never makes network requests.
use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use std::{path::{Path, PathBuf}, sync::Mutex, time::SystemTime};

const MAX_TILE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapPackInfo {
    name: String,
    attribution: String,
    min_zoom: u32,
    max_zoom: u32,
}

struct LocalPack {
    path: PathBuf,
    stamp: (u64, SystemTime),
    connection: Connection,
    info: MapPackInfo,
}

static PACK: Mutex<Option<LocalPack>> = Mutex::new(None);

fn local_path(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        let value = path.as_os_str().to_string_lossy().replace('/', "\\");
        let bytes = value.as_bytes();
        // Reject UNC/device/relative paths lexically, before even statting them.
        if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1..3] != *b":\\" {
            return Err("Choose a map on a local volume; network and device paths are not supported".into());
        }
        #[link(name = "kernel32")]
        unsafe extern "system" { fn GetDriveTypeW(root: *const u16) -> u32; }
        let root: Vec<u16> = value[..3].encode_utf16().chain(Some(0)).collect();
        if !matches!(unsafe { GetDriveTypeW(root.as_ptr()) }, 2 | 3 | 5 | 6) {
            return Err("Choose a map on a local volume; mapped network drives are not supported".into());
        }
        // Check each parent before its children so a local junction cannot
        // silently redirect a persisted pack path to an SMB share.
        let mut prefix = PathBuf::new();
        for component in path.components() {
            prefix.push(component.as_os_str());
            if prefix.as_os_str().len() <= 3 { continue; }
            let metadata = std::fs::symlink_metadata(&prefix).map_err(|e| e.to_string())?;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("Map files and their parent folders must not be links or reparse points".into());
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    { let _ = path; Err("Detailed local map packs currently require Windows; the bundled overview works offline".into()) }
}

fn open_pack(path: &Path) -> Result<LocalPack, String> {
    local_path(path)?;
    if !path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("mbtiles")) {
        return Err("Choose a raster .mbtiles map file".into());
    }
    let metadata = path.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() { return Err("Choose a local map file".into()); }
    let connection = Connection::open_with_flags(path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|e| e.to_string())?;
    connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA query_only=ON;").map_err(|e| e.to_string())?;
    let value = |key: &str| -> Result<Option<String>, String> {
        connection.query_row("SELECT value FROM metadata WHERE name=?1 LIMIT 1", [key], |r| r.get(0))
            .optional().map_err(|e| e.to_string())
    };
    if !matches!(value("format")?.as_deref(), Some("png" | "jpg" | "jpeg" | "webp")) {
        return Err("This map needs PNG, JPEG or WebP raster tiles; vector packs are not supported".into());
    }
    let zoom = |key, default| -> Result<u32, String> {
        match value(key)? {
            Some(v) => v.parse().map_err(|_| format!("Invalid map {key}")),
            None => Ok(default),
        }
    };
    let info = MapPackInfo {
        name: value("name")?.unwrap_or_else(|| "Local map".into()),
        attribution: value("attribution")?.unwrap_or_default(),
        min_zoom: zoom("minzoom", 0)?, max_zoom: zoom("maxzoom", 22)?,
    };
    if info.min_zoom > info.max_zoom || info.max_zoom > 22 {
        return Err("Map zoom range must be between 0 and 22".into());
    }
    // Prepare now so a malformed archive fails at selection, before map display.
    connection.prepare("SELECT tile_data FROM tiles WHERE zoom_level=?1 AND tile_column=?2 AND tile_row=?3 LIMIT 1")
        .map_err(|e| e.to_string())?;
    Ok(LocalPack { path: path.to_path_buf(), stamp: (metadata.len(), metadata.modified().map_err(|e| e.to_string())?), connection, info })
}

fn read_tile(connection: &Connection, z: u32, x: u32, y: u32) -> Result<Option<String>, String> {
    if z > 22 || x >= (1 << z) || y >= (1 << z) { return Err("Invalid map tile coordinates".into()); }
    // MBTiles uses TMS rows, opposite to Leaflet's XYZ rows.
    let row = (1u32 << z) - 1 - y;
    let bytes: Option<Vec<u8>> = connection.query_row(
        "SELECT tile_data FROM tiles WHERE zoom_level=?1 AND tile_column=?2 AND tile_row=?3 AND length(tile_data)<=?4 LIMIT 1",
        rusqlite::params![z, x, row, MAX_TILE_BYTES], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
    let Some(bytes) = bytes else { return Ok(None); };
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { "image/png" }
        else if bytes.starts_with(b"\xff\xd8\xff") { "image/jpeg" }
        else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") { "image/webp" }
        else { return Err("Map tile is not a supported raster image".into()); };
    Ok(Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes))))
}

#[tauri::command]
pub async fn local_map_info(path: String) -> Result<MapPackInfo, String> {
    tokio::task::spawn_blocking(move || open_pack(Path::new(&path)).map(|p| p.info))
        .await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn local_map_tile(path: String, z: u32, x: u32, y: u32) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        local_path(Path::new(&path))?;
        let path = Path::new(&path).canonicalize().map_err(|e| e.to_string())?;
        // Windows canonicalize adds an extended-length prefix. Only strip it
        // from our resolved local path; user-supplied device paths stay rejected.
        #[cfg(target_os = "windows")]
        let path = {
            let value = path.to_string_lossy();
            PathBuf::from(value.strip_prefix(r"\\?\").unwrap_or(&value))
        };
        local_path(&path)?;
        let metadata = path.metadata().map_err(|e| e.to_string())?;
        let stamp = (metadata.len(), metadata.modified().map_err(|e| e.to_string())?);
        let mut cache = PACK.lock().map_err(|e| e.to_string())?;
        if !cache.as_ref().is_some_and(|p| p.path == path && p.stamp == stamp) {
            *cache = Some(open_pack(&path)?);
        }
        read_tile(&cache.as_ref().unwrap().connection, z, x, y)
    }).await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tms_rows_missing_tiles_and_raster_only() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE tiles(zoom_level INTEGER,tile_column INTEGER,tile_row INTEGER,tile_data BLOB);").unwrap();
        let png = b"\x89PNG\r\n\x1a\nlocal-test";
        conn.execute("INSERT INTO tiles VALUES(2,1,3,?1)", [png.as_slice()]).unwrap();
        assert_eq!(read_tile(&conn, 2, 1, 0).unwrap(), Some(format!("data:image/png;base64,{}", STANDARD.encode(png))));
        assert!(read_tile(&conn, 2, 1, 1).unwrap().is_none());
        assert!(read_tile(&conn, 23, 0, 0).is_err());
        assert!(read_tile(&conn, 2, 4, 0).is_err());
        conn.execute("INSERT INTO tiles VALUES(0,0,0,?1)", [b"<svg onload='alert(1)'/>".as_slice()]).unwrap();
        assert!(read_tile(&conn, 0, 0, 0).is_err());
        conn.execute("UPDATE tiles SET tile_data=?1 WHERE zoom_level=0", [vec![0u8; MAX_TILE_BYTES+1]]).unwrap();
        assert!(read_tile(&conn, 0, 0, 0).unwrap().is_none());
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn selection_is_read_only_and_rejects_vector_archives() {
        let path = std::env::temp_dir().join(format!("lap-map-{}.mbtiles", uuid::Uuid::new_v4()));
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE metadata(name TEXT,value TEXT); INSERT INTO metadata VALUES('format','png'); CREATE TABLE tiles(zoom_level INTEGER,tile_column INTEGER,tile_row INTEGER,tile_data BLOB);").unwrap();
        }
        let before = std::fs::read(&path).unwrap();
        let pack = open_pack(&path).unwrap();
        assert!(pack.connection.execute("INSERT INTO metadata VALUES('name','changed')", []).is_err());
        drop(pack);
        assert_eq!(before, std::fs::read(&path).unwrap());
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute("UPDATE metadata SET value='pbf' WHERE name='format'", []).unwrap();
        }
        assert!(open_pack(&path).err().unwrap().contains("vector"));
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    #[cfg(target_os = "windows")]
    async fn ordinary_local_path_reads_through_the_command() {
        let path = std::env::temp_dir().join(format!("lap-map-command-{}.mbtiles", uuid::Uuid::new_v4()));
        let png = b"\x89PNG\r\n\x1a\nlocal-test";
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE metadata(name TEXT,value TEXT); INSERT INTO metadata VALUES('format','png'); CREATE TABLE tiles(zoom_level INTEGER,tile_column INTEGER,tile_row INTEGER,tile_data BLOB);").unwrap();
            conn.execute("INSERT INTO tiles VALUES(0,0,0,?1)", [png.as_slice()]).unwrap();
        }
        assert_eq!(local_map_tile(path.to_string_lossy().into_owned(), 0, 0, 0).await.unwrap(),
            Some(format!("data:image/png;base64,{}", STANDARD.encode(png))));
        PACK.lock().unwrap().take();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn local_pack_platform_limit_is_explicit() {
        assert!(open_pack(Path::new("/map.mbtiles")).err().unwrap().contains("require Windows"));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn rejects_network_and_device_paths_before_filesystem_access() {
        for path in [r"\\must-not-contact.invalid\share\map.mbtiles", r"\\?\UNC\must-not-contact.invalid\share\map.mbtiles",
                     r"\\?\C:\map.mbtiles", "relative.mbtiles"] {
            assert!(local_path(Path::new(path)).unwrap_err().contains("local volume"));
        }
    }
}
