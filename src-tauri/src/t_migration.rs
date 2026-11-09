use rusqlite::Connection;

struct Migration {
    version: i32,
    description: &'static str,
    sql: &'static str,
}

fn get_migrations() -> Vec<Migration> {
    vec![
        // v0.1.0 — initial release schema
        Migration {
            version: 1,
            description: "Create deduplication tables",
            sql: "
                CREATE TABLE IF NOT EXISTS file_hashes (
                    file_id INTEGER PRIMARY KEY,
                    hash TEXT NOT NULL,
                    file_size INTEGER NOT NULL,
                    mtime INTEGER NOT NULL,
                    computed_at INTEGER NOT NULL,
                    FOREIGN KEY (file_id) REFERENCES afiles(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_file_hashes_hash_size ON file_hashes(hash, file_size);
                CREATE INDEX IF NOT EXISTS idx_file_hashes_mtime ON file_hashes(mtime);

                CREATE TABLE IF NOT EXISTS duplicate_groups (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    hash TEXT NOT NULL,
                    file_size INTEGER NOT NULL,
                    file_count INTEGER NOT NULL,
                    total_size INTEGER NOT NULL,
                    reviewed INTEGER NOT NULL DEFAULT 0,
                    updated_at INTEGER NOT NULL
                );
                CREATE UNIQUE INDEX IF NOT EXISTS uidx_duplicate_groups_hash_size ON duplicate_groups(hash, file_size);

                CREATE TABLE IF NOT EXISTS duplicate_group_items (
                    group_id INTEGER NOT NULL,
                    file_id INTEGER NOT NULL,
                    is_keep INTEGER NOT NULL DEFAULT 0,
                    is_selected INTEGER NOT NULL DEFAULT 0,
                    score REAL NOT NULL DEFAULT 0,
                    PRIMARY KEY (group_id, file_id),
                    FOREIGN KEY (group_id) REFERENCES duplicate_groups(id) ON DELETE CASCADE,
                    FOREIGN KEY (file_id) REFERENCES afiles(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_dup_items_group ON duplicate_group_items(group_id);
                CREATE INDEX IF NOT EXISTS idx_dup_items_file ON duplicate_group_items(file_id);
            ",
        },
        // v0.1.x — RAW / format_label support
        Migration {
            version: 2,
            description: "Add afiles.format_label column",
            sql: "",
        },
        // v0.2.2 — thumbnail cache
        Migration {
            version: 3,
            description: "Add thumbnail cache metadata columns",
            sql: "",
        },
        // v0.2.2 — incremental scan foundation
        Migration {
            version: 4,
            description: "Add last_scan_time for file and album sync",
            sql: "",
        },
        // v0.2.4 — folder exclusion, inode, search exclusion
        Migration {
            version: 5,
            description: "Post v0.2.2 schema updates",
            sql: "",
        },
        // Post v0.2.4 — collections, unique folder names, Live Photo,
        // case-insensitive index, folder scan state
        Migration {
            version: 9,
            description: "Post v0.2.4 schema updates",
            sql: "",
        },
        Migration {
            version: 10,
            description: "Create visual similarity review tables",
            sql: "
                CREATE TABLE IF NOT EXISTS similarity_scans (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    scope_key TEXT NOT NULL UNIQUE,
                    source_version INTEGER NOT NULL,
                    status TEXT NOT NULL,
                    file_count INTEGER NOT NULL DEFAULT 0,
                    group_count INTEGER NOT NULL DEFAULT 0,
                    created_at INTEGER NOT NULL,
                    completed_at INTEGER
                );
                CREATE TABLE IF NOT EXISTS similarity_groups (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    scan_id INTEGER NOT NULL,
                    representative_file_id INTEGER NOT NULL,
                    file_count INTEGER NOT NULL,
                    latest_taken_date INTEGER NOT NULL DEFAULT 0,
                    min_score REAL NOT NULL DEFAULT 0,
                    max_score REAL NOT NULL DEFAULT 0,
                    FOREIGN KEY (scan_id) REFERENCES similarity_scans(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_similarity_groups_scan ON similarity_groups(scan_id);
                CREATE TABLE IF NOT EXISTS similarity_group_items (
                    group_id INTEGER NOT NULL,
                    file_id INTEGER NOT NULL,
                    score REAL NOT NULL,
                    is_keep INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (group_id, file_id),
                    FOREIGN KEY (group_id) REFERENCES similarity_groups(id) ON DELETE CASCADE,
                    FOREIGN KEY (file_id) REFERENCES afiles(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_similarity_items_group ON similarity_group_items(group_id);
                CREATE INDEX IF NOT EXISTS idx_similarity_items_file ON similarity_group_items(file_id);
            ",
        },
        Migration {
            version: 11,
            description: "Add file culling status",
            sql: "",
        },
        Migration {
            version: 12,
            description: "Cache direct subfolder state on folders",
            sql: "",
        },
        Migration {
            version: 13,
            description: "Persist folder filesystem identity",
            sql: "",
        },
        Migration {
            version: 14,
            description: "Store scan totals",
            sql: "",
        },
        Migration {
            version: 15,
            description: "Persist visual similarity keep state",
            sql: "",
        },
        Migration {
            version: 16,
            description: "Add motion photo offset",
            sql: "",
        },
        Migration {
            version: 17,
            description: "Add tag groups and persistent ordering",
            sql: "",
        },
        Migration {
            version: 18,
            description: "Album scan scope and pixel filters",
            sql: "",
        },
        Migration {
            version: 20,
            description: "Repair verified RIM firmware orientation and cached dimensions",
            sql: "",
        },
    ]
}

fn migrate_unique_album_files(conn: &Connection) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Migration 9 (deduplicate album files) failed starting transaction: {}", e))?;

    tx.execute_batch(
        "
        CREATE TEMP TABLE duplicate_afile_map (
            old_id INTEGER PRIMARY KEY,
            new_id INTEGER NOT NULL
        );

        INSERT INTO duplicate_afile_map (old_id, new_id)
        SELECT a.id, duplicates.new_id
        FROM afiles a
        JOIN (
            SELECT folder_id, name, MAX(id) AS new_id
            FROM afiles
            GROUP BY folder_id, name
            HAVING COUNT(*) > 1
        ) duplicates
          ON duplicates.folder_id = a.folder_id
         AND duplicates.name = a.name
        WHERE a.id <> duplicates.new_id;

        UPDATE albums
        SET cover_file_id = (
            SELECT new_id
            FROM duplicate_afile_map
            WHERE old_id = albums.cover_file_id
        )
        WHERE cover_file_id IN (SELECT old_id FROM duplicate_afile_map);

        UPDATE persons
        SET cover_face_id = NULL
        WHERE cover_face_id IN (
            SELECT faces.id
            FROM faces
            JOIN duplicate_afile_map ON duplicate_afile_map.old_id = faces.file_id
        );

        DELETE FROM afiles
        WHERE id IN (SELECT old_id FROM duplicate_afile_map);

        DROP INDEX IF EXISTS idx_afiles_folder_id_name;
        DROP INDEX IF EXISTS uidx_afiles_folder_id_name;
        CREATE UNIQUE INDEX uidx_afiles_folder_id_name
            ON afiles(folder_id, name);

        DROP TABLE duplicate_afile_map;
        ",
    )
    .map_err(|e| format!("Migration 9 (deduplicate album files) failed: {}", e))?;

    tx.commit()
        .map_err(|e| format!("Migration 9 (deduplicate album files) failed committing transaction: {}", e))
}

fn table_has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, String> {
    let pragma = format!("PRAGMA table_info({})", table);
    let mut stmt = conn.prepare(&pragma).map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;

    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let name: String = row.get(1).map_err(|e| e.to_string())?;
        if name.eq_ignore_ascii_case(column) {
            return Ok(true);
        }
    }

    Ok(false)
}

pub fn check_and_migrate(conn: &Connection) -> Result<(), String> {
    check_and_migrate_using(conn, crate::t_thumb_cache::invalidate_orientation)
}

fn check_and_migrate_using(conn: &Connection,
    mut invalidate: impl FnMut(&[(i64, String)]) -> Result<(), String>) -> Result<(), String> {
    let current_version: i32 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;

    println!("Current DB version: {}", current_version);

    // A stranded v19 library may already have been stamped v20 by main without
    // receiving the scan-scope columns. Repair their presence, preserving values.
    if current_version >= 18 {
        migrate_album_scan_scope(conn)?;
    }

    let migrations = get_migrations();
    let mut new_version = current_version;

    for migration in &migrations {
        if migration.version > current_version {
            println!(
                "Applying migration {}: {}",
                migration.version, migration.description
            );

            // Execute the migration logic
            if migration.version == 2 {
                if !table_has_column(conn, "afiles", "format_label")? {
                    conn.execute("ALTER TABLE afiles ADD COLUMN format_label TEXT", [])
                        .map_err(|e| format!("Migration {} failed: {}", migration.version, e))?;
                }
            } else if migration.version == 3 {
                if !table_has_column(conn, "athumbs", "thumb_key")? {
                    conn.execute("ALTER TABLE athumbs ADD COLUMN thumb_key TEXT", [])
                        .map_err(|e| format!("Migration {} failed: {}", migration.version, e))?;
                }
                if !table_has_column(conn, "athumbs", "thumb_mtime")? {
                    conn.execute("ALTER TABLE athumbs ADD COLUMN thumb_mtime INTEGER", [])
                        .map_err(|e| format!("Migration {} failed: {}", migration.version, e))?;
                }
                if !table_has_column(conn, "athumbs", "thumb_size")? {
                    conn.execute("ALTER TABLE athumbs ADD COLUMN thumb_size INTEGER", [])
                        .map_err(|e| format!("Migration {} failed: {}", migration.version, e))?;
                }
                if !table_has_column(conn, "athumbs", "updated_at")? {
                    conn.execute("ALTER TABLE athumbs ADD COLUMN updated_at INTEGER", [])
                        .map_err(|e| format!("Migration {} failed: {}", migration.version, e))?;
                }
                conn.execute(
                    "CREATE INDEX IF NOT EXISTS idx_athumbs_thumb_key ON athumbs(thumb_key)",
                    [],
                )
                .map_err(|e| format!("Migration {} failed: {}", migration.version, e))?;
            } else if migration.version == 4 {
                if !table_has_column(conn, "afiles", "last_scan_time")? {
                    conn.execute(
                        "ALTER TABLE afiles ADD COLUMN last_scan_time INTEGER DEFAULT 0",
                        [],
                    )
                    .map_err(|e| {
                        format!(
                            "Migration {} failed adding last_scan_time: {}",
                            migration.version, e
                        )
                    })?;
                }
                if !table_has_column(conn, "albums", "last_scan_time")? {
                    conn.execute(
                        "ALTER TABLE albums ADD COLUMN last_scan_time INTEGER DEFAULT 0",
                        [],
                    )
                    .map_err(|e| {
                        format!(
                            "Migration {} failed adding last_scan_time: {}",
                            migration.version, e
                        )
                    })?;
                }
                conn.execute(
                    "CREATE INDEX IF NOT EXISTS idx_afiles_last_scan_time ON afiles(last_scan_time)",
                    [],
                )
                .map_err(|e| format!("Migration {} failed adding last_scan_time index: {}", migration.version, e))?;
            } else if migration.version == 5 {
                if !table_has_column(conn, "afolders", "is_excluded_from_search")? {
                    conn.execute(
                        "ALTER TABLE afolders ADD COLUMN is_excluded_from_search INTEGER DEFAULT 0",
                        [],
                    )
                    .map_err(|e| {
                        format!(
                            "Migration {} failed adding is_excluded_from_search: {}",
                            migration.version, e
                        )
                    })?;
                }
                if !table_has_column(conn, "afiles", "inode")? {
                    conn.execute("ALTER TABLE afiles ADD COLUMN inode INTEGER", [])
                        .map_err(|e| {
                            format!("Migration {} failed adding inode: {}", migration.version, e)
                        })?;
                }
                conn.execute(
                    "CREATE INDEX IF NOT EXISTS idx_afolders_is_excluded_from_search ON afolders(is_excluded_from_search)",
                    [],
                )
                .map_err(|e| {
                    format!(
                        "Migration {} failed adding is_excluded_from_search index: {}",
                        migration.version, e
                    )
                })?;
            } else if migration.version == 9 {
                // --- collections ---
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS acollections (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        name TEXT NOT NULL,
                        sort_order INTEGER NOT NULL DEFAULT 0,
                        created_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS idx_acollections_sort ON acollections(sort_order, id);

                    CREATE TABLE IF NOT EXISTS acollections_files (
                        collection_id INTEGER NOT NULL,
                        file_id INTEGER NOT NULL,
                        added_at INTEGER NOT NULL,
                        PRIMARY KEY (collection_id, file_id),
                        FOREIGN KEY (collection_id) REFERENCES acollections(id) ON DELETE CASCADE,
                        FOREIGN KEY (file_id) REFERENCES afiles(id) ON DELETE CASCADE
                    );
                    CREATE INDEX IF NOT EXISTS idx_acollections_files_file ON acollections_files(file_id);
                    CREATE INDEX IF NOT EXISTS idx_acollections_files_collection_added
                        ON acollections_files(collection_id, added_at DESC, file_id);",
                )
                .map_err(|e| format!("Migration 9 failed creating collections: {}", e))?;

                // --- deduplicate album files ---
                migrate_unique_album_files(conn)?;

                // --- Live Photo pairing columns ---
                if !table_has_column(conn, "afiles", "content_identifier")? {
                    conn.execute("ALTER TABLE afiles ADD COLUMN content_identifier TEXT", [])
                        .map_err(|e| format!("Migration 9 failed adding content_identifier: {}", e))?;
                }
                if !table_has_column(conn, "afiles", "media_subtype")? {
                    conn.execute("ALTER TABLE afiles ADD COLUMN media_subtype TEXT", [])
                        .map_err(|e| format!("Migration 9 failed adding media_subtype: {}", e))?;
                }
                if !table_has_column(conn, "afiles", "live_photo_video_id")? {
                    conn.execute("ALTER TABLE afiles ADD COLUMN live_photo_video_id INTEGER", [])
                        .map_err(|e| format!("Migration 9 failed adding live_photo_video_id: {}", e))?;
                }
                conn.execute(
                    "CREATE INDEX IF NOT EXISTS idx_afiles_content_identifier ON afiles(content_identifier)",
                    [],
                ).map_err(|e| format!("Migration 9 failed adding content_identifier index: {}", e))?;
                conn.execute(
                    "CREATE INDEX IF NOT EXISTS idx_afiles_live_photo_video_id ON afiles(live_photo_video_id)",
                    [],
                ).map_err(|e| format!("Migration 9 failed adding live_photo_video_id index: {}", e))?;

                // --- case-insensitive filename index ---
                conn.execute(
                    "CREATE INDEX IF NOT EXISTS idx_afiles_folder_name_nocase
                     ON afiles(folder_id, name COLLATE NOCASE)",
                    [],
                ).map_err(|e| format!("Migration 9 failed adding case-insensitive index: {}", e))?;

                // --- per-folder scan state ---
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS folder_scan_state (
                        folder_id INTEGER NOT NULL,
                        scanner TEXT NOT NULL,
                        version INTEGER NOT NULL DEFAULT 0,
                        updated_at INTEGER NOT NULL,
                        PRIMARY KEY (folder_id, scanner),
                        FOREIGN KEY (folder_id) REFERENCES afolders(id) ON DELETE CASCADE
                    );
                    CREATE INDEX IF NOT EXISTS idx_folder_scan_state_scanner_version
                        ON folder_scan_state(scanner, version);",
                ).map_err(|e| format!("Migration 9 failed creating folder scan state: {}", e))?;
            } else if migration.version == 11 {
                if !table_has_column(conn, "afiles", "culling_flag")? {
                    conn.execute(
                        "ALTER TABLE afiles ADD COLUMN culling_flag INTEGER NOT NULL DEFAULT 0",
                        [],
                    )
                    .map_err(|e| format!("Migration 11 failed adding culling_flag: {}", e))?;
                }
                conn.execute(
                    "CREATE INDEX IF NOT EXISTS idx_afiles_culling_flag ON afiles(culling_flag)",
                    [],
                )
                .map_err(|e| format!("Migration 11 failed adding culling_flag index: {}", e))?;
            } else if migration.version == 12 {
                if !table_has_column(conn, "afolders", "has_subfolders")? {
                    conn.execute("ALTER TABLE afolders ADD COLUMN has_subfolders INTEGER", [])
                        .map_err(|e| format!("Migration 12 failed adding has_subfolders: {}", e))?;
                }
            } else if migration.version == 13 {
                if !table_has_column(conn, "afolders", "inode")? {
                    conn.execute("ALTER TABLE afolders ADD COLUMN inode INTEGER", [])
                        .map_err(|e| format!("Migration 13 failed adding inode: {}", e))?;
                }
                conn.execute(
                    "CREATE INDEX IF NOT EXISTS idx_afolders_album_inode ON afolders(album_id, inode)",
                    [],
                )
                .map_err(|e| format!("Migration 13 failed creating inode index: {}", e))?;
            } else if migration.version == 14 {
                for column in ["skipped_count", "skipped_size", "failed_count", "failed_size", "merged_count", "merged_size"] {
                    if !table_has_column(conn, "albums", column)? {
                        conn.execute(
                            &format!("ALTER TABLE albums ADD COLUMN {} INTEGER NOT NULL DEFAULT 0", column),
                            [],
                        )
                        .map_err(|e| format!("Migration 14 failed adding {}: {}", column, e))?;
                    }
                }
            } else if migration.version == 15 {
                if !table_has_column(conn, "similarity_group_items", "is_keep")? {
                    conn.execute(
                        "ALTER TABLE similarity_group_items ADD COLUMN is_keep INTEGER NOT NULL DEFAULT 0",
                        [],
                    )
                    .map_err(|e| format!("Migration 15 failed adding is_keep: {}", e))?;
                    conn.execute_batch(
                        "UPDATE similarity_group_items
                         SET is_keep = 1
                         WHERE EXISTS (
                             SELECT 1
                             FROM similarity_groups g
                             WHERE g.id = similarity_group_items.group_id
                               AND g.representative_file_id = similarity_group_items.file_id
                         );",
                    )
                    .map_err(|e| format!("Migration 15 failed initializing keep state: {}", e))?;
                }
            } else if migration.version == 16 {
                if !table_has_column(conn, "afiles", "motion_photo_offset")? {
                    conn.execute("ALTER TABLE afiles ADD COLUMN motion_photo_offset INTEGER", [])
                        .map_err(|e| {
                            format!("Migration 16 failed adding motion_photo_offset: {}", e)
                        })?;
                }
            } else if migration.version == 17 {
                migrate_tag_groups(conn)?;
            } else if migration.version == 18 {
                migrate_album_scan_scope(conn)?;
            } else if migration.version == 20 {
                migrate_rim_orientation(conn, &mut invalidate)?;
            } else if !migration.sql.trim().is_empty() {
                conn.execute_batch(migration.sql)
                    .map_err(|e| format!("Migration {} failed: {}", migration.version, e))?;
            }

            new_version = migration.version;
        }
    }

    if new_version > current_version {
        let update_version_sql = format!("PRAGMA user_version = {};", new_version);
        conn.execute_batch(&update_version_sql)
            .map_err(|e| format!("Failed to update user_version: {}", e))?;

        println!("Database successfully migrated to version {}", new_version);
    } else {
        println!("Database is up to date.");
    }

    // Cache availability must not prevent opening a migrated library. A failed
    // eviction remains in SQLite and is retried when this library opens again.
    if current_version >= 20 {
        retry_rim_cache_evictions(conn, &mut invalidate);
    }

    Ok(())
}

fn migrate_album_scan_scope(conn: &Connection) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    for (column, definition) in [
        ("file_types", "INTEGER NOT NULL DEFAULT 7"),
        ("small_image_filter", "INTEGER NOT NULL DEFAULT 0"),
        ("excluded_folders", "TEXT NOT NULL DEFAULT '[]'"),
    ] {
        if !table_has_column(&tx, "albums", column)? {
            tx.execute(&format!("ALTER TABLE albums ADD COLUMN {column} {definition}"), [])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

fn migrate_rim_orientation(conn: &Connection,
    invalidate: impl FnOnce(&[(i64, String)]) -> Result<(), String>) -> Result<(), String> {
    let candidates = {
        // The stranded version 19 may already have changed stored orientations.
        // Verify the source again, including legitimate rotations it overrode.
        let mut stmt = conn.prepare("SELECT a.id, d.path, a.name, a.e_orientation, COALESCE(a.width,0), COALESCE(a.height,0) FROM afiles a
            JOIN afolders d ON d.id=a.folder_id
            WHERE (a.e_make LIKE '%Research In Motion%' OR a.e_make LIKE '%BlackBerry%')
            AND a.e_orientation IN (1,6)")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?,
            r.get::<_, Option<i32>>(3)?, r.get::<_, u32>(4)?, r.get::<_, u32>(5)?)))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?
    };
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    ensure_rim_cache_evictions(&tx)?;
    for (id, folder, name, old_orientation, old_width, old_height) in candidates {
        let path = std::path::Path::new(&folder).join(name);
        let Some(path) = path.to_str() else { continue; };
        let Ok((w, h)) = image::image_dimensions(path) else { continue; };
        let Some(exif) = crate::t_image::read_exif_permissive(path) else { continue; };
        if !crate::t_image::is_rim_camera(&exif) { continue; }
        let orientation = crate::t_image::normalized_image_orientation(&exif, w, h);
        let (width, height) = if orientation > 4 { (h, w) } else { (w, h) };
        let metadata_matches = old_orientation == Some(orientation) && (old_width, old_height) == (width, height);
        if metadata_matches && !crate::t_image::has_rim_orientation_defect(&exif, w, h) { continue; }
        // Rescanning may have normalized metadata without replacing old cache entries.
        if !metadata_matches {
            tx.execute("UPDATE afiles SET e_orientation=?1,width=?2,height=?3 WHERE id=?4", rusqlite::params![orientation,width,height,id])
                .map_err(|e| e.to_string())?;
        }
        tx.execute("DELETE FROM athumbs WHERE file_id=?1", [id]).map_err(|e| e.to_string())?;
        tx.execute("INSERT OR REPLACE INTO orientation_cache_evictions(file_id,source_path) VALUES(?1,?2)",
            rusqlite::params![id,path]).map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    retry_rim_cache_evictions(conn, invalidate);
    Ok(())
}

fn ensure_rim_cache_evictions(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS orientation_cache_evictions (
        file_id INTEGER PRIMARY KEY, source_path TEXT NOT NULL)").map_err(|e| e.to_string())
}

fn retry_rim_cache_evictions(conn: &Connection,
    invalidate: impl FnOnce(&[(i64, String)]) -> Result<(), String>) {
    let result = (|| -> Result<(), String> {
        // Existing v20 libraries may have no queue. Optional cache work must not
        // create tables or make opening a read-only/temporarily locked library fail.
        let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='orientation_cache_evictions')",
            [], |r| r.get(0)).map_err(|e| e.to_string())?;
        if !exists { return Ok(()); }
        let pending = conn.prepare("SELECT file_id,source_path FROM orientation_cache_evictions ORDER BY file_id")
            .map_err(|e| e.to_string())?.query_map([], |r| Ok((r.get(0)?,r.get(1)?)))
            .map_err(|e| e.to_string())?.collect::<Result<Vec<(i64,String)>,_>>().map_err(|e| e.to_string())?;
        if pending.is_empty() { return Ok(()); }
        invalidate(&pending)?;
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for (id,path) in &pending {
            tx.execute("DELETE FROM orientation_cache_evictions WHERE file_id=?1 AND source_path=?2",
                rusqlite::params![id,path]).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    })();
    if let Err(error) = result {
        eprintln!("RIM cache eviction deferred until the library next opens: {error}");
    }
}

#[cfg(test)]
mod rim_migration_tests {
    use super::*;
    #[test]
    fn lap106_stamped_19_or_20_repairs_missing_scan_scope_without_resetting_preferences() {
        for (version,partial) in [(19,false),(19,true),(20,false),(20,true)] {
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch("CREATE TABLE albums(id INTEGER); INSERT INTO albums VALUES(1);
                CREATE TABLE afolders(id INTEGER,path TEXT);
                CREATE TABLE afiles(id INTEGER,folder_id INTEGER,name TEXT,e_orientation INTEGER,e_make TEXT,width INTEGER,height INTEGER);
                CREATE TABLE athumbs(file_id INTEGER);").unwrap();
            conn.pragma_update(None,"user_version",version).unwrap();
            if partial {
                conn.execute_batch("ALTER TABLE albums ADD COLUMN file_types INTEGER NOT NULL DEFAULT 7;
                    UPDATE albums SET file_types=3;").unwrap();
            }
            check_and_migrate(&conn).unwrap();
            assert_eq!(conn.query_row("PRAGMA user_version", [], |r| r.get::<_,i32>(0)).unwrap(), 20);
            assert_eq!(conn.query_row("SELECT file_types,small_image_filter,excluded_folders FROM albums", [],
                |r| Ok((r.get::<_,i32>(0)?,r.get::<_,i32>(1)?,r.get::<_,String>(2)?))).unwrap(),
                (if partial {3} else {7},0,"[]".to_owned()));
            conn.execute_batch("UPDATE albums SET small_image_filter=1,excluded_folders='[\"kept\"]'; PRAGMA user_version=19;").unwrap();
            check_and_migrate(&conn).unwrap();
            assert_eq!(conn.query_row("SELECT excluded_folders FROM albums", [], |r| r.get::<_,String>(0)).unwrap(), "[\"kept\"]");
        }
    }
    fn rim_fixture() -> (Connection, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("lap106-rim-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let fields = [
            exif::Field {tag:exif::Tag::Make,ifd_num:exif::In::PRIMARY,value:exif::Value::Ascii(vec![b"Research In Motion".to_vec()])},
            exif::Field {tag:exif::Tag::Software,ifd_num:exif::In::PRIMARY,value:exif::Value::Ascii(vec![b"Rim Exif Version1.00a".to_vec()])},
            exif::Field {tag:exif::Tag::Orientation,ifd_num:exif::In::PRIMARY,value:exif::Value::Short(vec![6])},
        ];
        let mut writer=exif::experimental::Writer::new();
        for f in &fields {writer.push_field(f);}
        let mut exif=std::io::Cursor::new(Vec::new());writer.write(&mut exif,true).unwrap();
        let mut block=b"Exif\0\0".to_vec();block.extend(exif.into_inner());
        for (name,w,h) in [("bad.jpg",80,60),("portrait.jpg",60,80)] {
            let p=root.join(name);
            image::RgbImage::from_pixel(w,h,image::Rgb([40,80,120])).save(&p).unwrap();
            let jpeg=std::fs::read(&p).unwrap();
            let mut bytes=vec![0xff,0xd8,0xff,0xe1];bytes.extend(((block.len()+2) as u16).to_be_bytes());
            bytes.extend(&block);bytes.extend(&jpeg[2..]);std::fs::write(p,bytes).unwrap();
        }
        let conn=Connection::open(root.join("library.db")).unwrap();
        conn.execute_batch("CREATE TABLE albums(id INTEGER,file_types INTEGER DEFAULT 7,small_image_filter INTEGER DEFAULT 0,excluded_folders TEXT DEFAULT '[]');CREATE TABLE afolders(id INTEGER,path TEXT);CREATE TABLE afiles(id INTEGER,folder_id INTEGER,name TEXT,e_orientation INTEGER,e_make TEXT,e_software TEXT,width INTEGER,height INTEGER);CREATE TABLE athumbs(file_id INTEGER);
            INSERT INTO afiles VALUES(1,1,'bad.jpg',6,'Research In Motion','Rim Exif Version1.00a',60,80),(2,1,'portrait.jpg',6,'Research In Motion','Rim Exif Version1.00a',80,60),(3,1,'missing.jpg',6,'Research In Motion','Rim Exif Version1.00a',60,80),(4,1,'portrait.jpg',1,'Research In Motion','Rim Exif Version1.00a',60,80);
            INSERT INTO afiles VALUES(5,1,'bad.jpg',0,'Research In Motion','Rim Exif Version1.00a',80,60);
            INSERT INTO afiles VALUES(6,1,'bad.jpg',1,'Research In Motion','Rim Exif Version1.00a',80,60);
            INSERT INTO athumbs VALUES(1),(2),(3),(4),(5),(6); PRAGMA user_version=19;").unwrap();
        conn.execute("INSERT INTO afolders VALUES(1,?1)",[root.to_str().unwrap()]).unwrap();
        assert!(get_migrations().iter().any(|m| m.version == 20));
        (conn, root)
    }
    #[test]
    fn lap106_cache_failure_commits_metadata_and_retries_after_reopen() {
        let (conn,root)=rim_fixture();
        check_and_migrate_using(&conn, |_| Err("cache unavailable".into())).unwrap();
        assert_eq!(conn.query_row("PRAGMA user_version",[],|r|r.get::<_,i32>(0)).unwrap(),20);
        assert_eq!(conn.query_row("SELECT e_orientation FROM afiles WHERE id=1",[],|r|r.get::<_,i32>(0)).unwrap(),1);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM athumbs",[],|r|r.get::<_,i32>(0)).unwrap(),3);
        drop(conn);
        let conn=Connection::open(root.join("library.db")).unwrap();
        check_and_migrate_using(&conn, |files| {
            assert_eq!(files.iter().map(|(id,_)|*id).collect::<Vec<_>>(),vec![1,4,6]);
            Ok(())
        }).unwrap();
        let mut calls=0;
        check_and_migrate_using(&conn, |_| {calls+=1;Ok(())}).unwrap();
        assert_eq!(calls,0,"completed deferred evictions must not run again");
        drop(conn);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn lap106_v20_open_does_not_create_optional_cache_tables_or_fail_on_retry_writes() {
        for pending in [false,true] {
            let conn=Connection::open_in_memory().unwrap();
            conn.execute_batch("CREATE TABLE albums(id INTEGER,file_types INTEGER,small_image_filter INTEGER,excluded_folders TEXT);PRAGMA user_version=20;").unwrap();
            if pending {
                ensure_rim_cache_evictions(&conn).unwrap();
                conn.execute("INSERT INTO orientation_cache_evictions VALUES(1,'/test/photo.jpg')",[]).unwrap();
            }
            conn.execute_batch("PRAGMA query_only=ON").unwrap();
            let mut calls=0;
            check_and_migrate_using(&conn, |_| {calls+=1;Ok(())}).unwrap();
            assert_eq!(calls,if pending {1}else{0});
            assert_eq!(conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='orientation_cache_evictions'",[],|r|r.get::<_,i32>(0)).unwrap(),if pending {1}else{0});
            if pending {
                assert_eq!(conn.query_row("SELECT COUNT(*) FROM orientation_cache_evictions",[],|r|r.get::<_,i32>(0)).unwrap(),1,"failed cache bookkeeping stays pending");
                conn.execute_batch("PRAGMA query_only=OFF").unwrap();
                check_and_migrate_using(&conn, |_| Ok(())).unwrap();
                assert_eq!(conn.query_row("SELECT COUNT(*) FROM orientation_cache_evictions",[],|r|r.get::<_,i32>(0)).unwrap(),0);
            }
        }
    }
    #[test]
    fn lap106_versioned_repair_verifies_pixels_and_does_not_repeat_completed_evictions() {
        let (conn,root)=rim_fixture();
        check_and_migrate_using(&conn, |files| {
            assert_eq!(files.iter().map(|(id,_)|*id).collect::<Vec<_>>(),vec![1,4,6],"pre-normalized metadata may still have stale cache rows");
            Ok(())
        }).unwrap();
        conn.execute("INSERT INTO athumbs VALUES(1)",[]).unwrap();
        let mut calls=0;
        check_and_migrate_using(&conn, |_| {calls+=1;Ok(())}).unwrap();
        assert_eq!(calls,0,"a completed v20 open must not repeat cache eviction");
        let rows:Vec<(i32,i32,i32,i32)>=conn.prepare("SELECT id,e_orientation,width,height FROM afiles ORDER BY id").unwrap()
            .query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().map(Result::unwrap).collect();
        assert_eq!(rows,vec![(1,1,80,60),(2,6,80,60),(3,6,60,80),(4,6,80,60),(5,0,80,60),(6,1,80,60)]);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM athumbs",[],|r|r.get::<_,i32>(0)).unwrap(),4);
        drop(conn);
        std::fs::remove_dir_all(root).unwrap();
    }
}

// Idempotent and atomic, including when a previous migration run was interrupted.
pub(crate) fn migrate_tag_groups(conn: &Connection) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS atag_groups (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL COLLATE NOCASE UNIQUE,
        sort_order INTEGER NOT NULL DEFAULT 0,
        is_default INTEGER NOT NULL DEFAULT 0 CHECK(is_default IN (0, 1))
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_atag_groups_default
        ON atag_groups(is_default) WHERE is_default = 1;
    INSERT INTO atag_groups(name, is_default)
        SELECT 'Default', 1 WHERE NOT EXISTS (SELECT 1 FROM atag_groups WHERE is_default = 1);")
        .map_err(|e| e.to_string())?;
    if !table_has_column(&tx, "atags", "group_id")? {
        tx.execute_batch("ALTER TABLE atags ADD COLUMN group_id INTEGER REFERENCES atag_groups(id);")
            .map_err(|e| e.to_string())?;
    }
    tx.execute_batch("UPDATE atags SET group_id = (SELECT id FROM atag_groups WHERE is_default = 1)
        WHERE group_id IS NULL;
        CREATE INDEX IF NOT EXISTS idx_atags_group_id ON atags(group_id);
        CREATE TRIGGER IF NOT EXISTS protect_default_tag_group_delete BEFORE DELETE ON atag_groups
        WHEN OLD.is_default = 1 BEGIN SELECT RAISE(ABORT, 'Cannot delete Default'); END;
        CREATE TRIGGER IF NOT EXISTS protect_default_tag_group_update BEFORE UPDATE ON atag_groups
        WHEN NEW.is_default != OLD.is_default OR (OLD.is_default = 1 AND NEW.name != OLD.name)
        BEGIN SELECT RAISE(ABORT, 'Cannot rename or replace Default'); END;
        CREATE TRIGGER IF NOT EXISTS assign_default_tag_group AFTER INSERT ON atags
        WHEN NEW.group_id IS NULL BEGIN UPDATE atags
        SET group_id = (SELECT id FROM atag_groups WHERE is_default = 1) WHERE id = NEW.id; END;
        CREATE TRIGGER IF NOT EXISTS require_tag_group BEFORE UPDATE OF group_id ON atags
        WHEN NEW.group_id IS NULL BEGIN SELECT RAISE(ABORT, 'Tag group is required'); END;")
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
mod album_filter_migration_tests {
    use super::*;

    #[test]
    fn album_filter_migration_is_restartable_and_defaults_to_off() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE albums(id INTEGER PRIMARY KEY); INSERT INTO albums VALUES(1);
            CREATE TABLE afolders(id INTEGER,path TEXT);
            CREATE TABLE afiles(id INTEGER,folder_id INTEGER,name TEXT,e_orientation INTEGER,e_make TEXT,width INTEGER,height INTEGER);
            CREATE TABLE athumbs(file_id INTEGER); PRAGMA user_version=17;").unwrap();
        check_and_migrate(&conn).unwrap();
        let settings: (i64, Option<i64>, String) = conn.query_row(
            "SELECT file_types,small_image_filter,excluded_folders FROM albums", [],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(settings, (7,Some(0),"[]".into()));
        // Simulate migration completing before the version was persisted.
        conn.execute_batch("PRAGMA user_version=17;").unwrap();
        check_and_migrate(&conn).unwrap();
        assert_eq!(conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0)).unwrap(),
            get_migrations().last().unwrap().version as i64);
    }
}
