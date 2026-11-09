use lap_casa::slabs::SlabCache;
use redb::{Database, TableDefinition};
use rusqlite::Connection;
use std::path::PathBuf;
use std::time::Instant;

const THUMBNAILS_TABLE: TableDefinition<i64, &[u8]> = TableDefinition::new("thumbnails");
const ORIENTATION_VERSIONS: TableDefinition<i64, u32> = TableDefinition::new("thumbnail_orientation_versions");

#[test]
fn benchmark_folder_scroll_redb_vs_slab() {
    println!("\n========================================================");
    println!("LAP-038 BENCHMARK: THUMBNAIL CACHE SCROLL PERFORMANCE");
    println!("========================================================");

    // =========================================================================
    // PART 1: Controlled Apple-to-Apple Benchmark (5,000 thumbnails)
    // =========================================================================
    println!("\n--- PART 1: Controlled A/B Benchmark (5,000 photo thumbnails, ~40 KB each) ---");
    let temp_dir = std::env::temp_dir().join(format!("lap_bench_scroll_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&temp_dir);

    let count = 5_000usize;
    // Generate realistic JPEG-like thumbnail payload (~40KB)
    let mut sample_payload = vec![0u8; 40 * 1024];
    let len = sample_payload.len();
    sample_payload[0..4].copy_from_slice(b"\xFF\xD8\xFF\xE0");
    sample_payload[len - 2..].copy_from_slice(b"\xFF\xD9");

    let redb_path = temp_dir.join("bench_thumbs.redb");
    let slabs_dir = temp_dir.join("slabs");

    // Populate REDB
    {
        let db = Database::create(&redb_path).expect("create bench redb");
        let write_tx = db.begin_write().expect("begin write");
        {
            let mut thumbs = write_tx.open_table(THUMBNAILS_TABLE).expect("open thumbs");
            let mut vers = write_tx.open_table(ORIENTATION_VERSIONS).expect("open vers");
            for id in 1..=count as i64 {
                thumbs.insert(id, sample_payload.as_slice()).expect("insert thumb");
                vers.insert(id, 1u32).expect("insert version");
            }
        }
        write_tx.commit().expect("commit");
    }

    // Populate SLAB CACHE
    {
        let slab_cache = SlabCache::open(&slabs_dir).expect("open bench slab");
        for id in 1..=count as i64 {
            slab_cache.put(id, &sample_payload).expect("slab put");
        }
        slab_cache.flush().expect("slab flush");
    }

    // --- Benchmark REDB ---
    let (cold_redb_dur, warm_redb_dur, redb_bytes) = {
        let db = Database::open(&redb_path).expect("open bench redb");
        let read_tx = db.begin_read().expect("begin read");
        let table = read_tx.open_table(THUMBNAILS_TABLE).expect("open table");
        let vers = read_tx.open_table(ORIENTATION_VERSIONS).expect("open vers");

        // Cold pass
        let t0 = Instant::now();
        let mut bytes = 0usize;
        for id in 1..=count as i64 {
            if vers.get(id).ok().flatten().map(|v| v.value()) == Some(1) {
                if let Some(val) = table.get(id).ok().flatten() {
                    bytes += val.value().len();
                }
            }
        }
        let cold = t0.elapsed();

        // Warm pass
        let t1 = Instant::now();
        for id in 1..=count as i64 {
            if vers.get(id).ok().flatten().map(|v| v.value()) == Some(1) {
                if let Some(val) = table.get(id).ok().flatten() {
                    let _ = val.value().len();
                }
            }
        }
        let warm = t1.elapsed();

        (cold, warm, bytes)
    };

    // --- Benchmark SLAB CACHE ---
    let (cold_slab_dur, warm_slab_dur, _slab_bytes) = {
        let cache = SlabCache::open(&slabs_dir).expect("reopen slab cache");

        // Cold pass
        let t0 = Instant::now();
        let mut bytes = 0usize;
        for id in 1..=count as i64 {
            if let Some(data) = cache.get(id) {
                bytes += data.len();
            }
        }
        let cold = t0.elapsed();

        // Warm pass
        let t1 = Instant::now();
        for id in 1..=count as i64 {
            if let Some(data) = cache.get(id) {
                let _ = data.len();
            }
        }
        let warm = t1.elapsed();

        (cold, warm, bytes)
    };

    let mb = (redb_bytes as f64) / 1_048_576.0;
    println!("REDB B-Tree (Before):");
    println!("  Cold scroll (5,000 photos, {:.1} MB): {:.2?} ({:.1} us/thumb, {:.1} MB/s)",
        mb, cold_redb_dur, cold_redb_dur.as_micros() as f64 / count as f64, mb / cold_redb_dur.as_secs_f64());
    println!("  Warm scroll (5,000 photos, {:.1} MB): {:.2?} ({:.1} us/thumb, {:.1} MB/s)",
        mb, warm_redb_dur, warm_redb_dur.as_micros() as f64 / count as f64, mb / warm_redb_dur.as_secs_f64());

    println!("SLAB Container Cache (After):");
    println!("  Cold scroll (5,000 photos, {:.1} MB): {:.2?} ({:.1} us/thumb, {:.1} MB/s)",
        mb, cold_slab_dur, cold_slab_dur.as_micros() as f64 / count as f64, mb / cold_slab_dur.as_secs_f64());
    println!("  Warm scroll (5,000 photos, {:.1} MB): {:.2?} ({:.1} us/thumb, {:.1} MB/s)",
        mb, warm_slab_dur, warm_slab_dur.as_micros() as f64 / count as f64, mb / warm_slab_dur.as_secs_f64());

    let cold_speedup = cold_redb_dur.as_secs_f64() / cold_slab_dur.as_secs_f64();
    let warm_speedup = warm_redb_dur.as_secs_f64() / warm_slab_dur.as_secs_f64();
    println!("Results:");
    println!("  Cold scroll speedup: {:.2}x faster", cold_speedup);
    println!("  Warm scroll speedup: {:.2}x faster", warm_speedup);

    let _ = std::fs::remove_dir_all(&temp_dir);

    // =========================================================================
    // PART 2: Real Library Slab Cache Benchmark on Folder 1273 (10,878 photos)
    // =========================================================================
    let app_dir = std::env::var_os("LAP_BENCH_PROFILE").map(PathBuf::from)
        .unwrap_or_else(|| temp_dir.join("no-real-library"));
    let real_db = app_dir.join("libraries").join("default.db");
    let real_slabs = app_dir.join("slabs");

    if real_db.exists() && real_slabs.exists() {
        println!("\n--- PART 2: Real Library Folder 1273 Benchmark (10,878 items) ---");
        if let Ok(conn) = Connection::open(&real_db) {
            let mut stmt = conn
                .prepare("SELECT id FROM afiles WHERE folder_id = 1273 ORDER BY id ASC")
                .expect("prepare select");
            let file_ids: Vec<i64> = stmt
                .query_map([], |row| row.get(0))
                .expect("query afiles")
                .filter_map(|r| r.ok())
                .collect();

            if let Ok(slab) = SlabCache::open(&real_slabs) {
                // Cold pass across real library slab
                let t0 = Instant::now();
                let mut hits = 0;
                let mut bytes = 0usize;
                let mut stale_versions = 0;
                for &id in &file_ids {
                    if let Some(data) = slab.get(id) {
                        hits += 1;
                        bytes += data.len();
                    } else if let Some(data) = slab.get_raw(id) {
                        stale_versions += 1;
                        bytes += data.len();
                    }
                }
                let cold_dur = t0.elapsed();
                let total_hits = hits + stale_versions;

                // Warm pass across real library slab
                let t1 = Instant::now();
                for &id in &file_ids {
                    if let Some(data) = slab.get(id).or_else(|| slab.get_raw(id)) {
                        let _ = data.len();
                    }
                }
                let warm_dur = t1.elapsed();

                let real_mb = (bytes as f64) / 1_048_576.0;
                println!("Real Library Slab Cache (Concurrent Lock-Free):");
                println!("  Photos requested:    {} (hits: {} / {}, stale/legacy version: {})",
                    file_ids.len(), total_hits, file_ids.len(), stale_versions);
                println!("  Total data read:     {:.2} MB", real_mb);
                if total_hits > 0 {
                    println!("  Cold scroll total:   {:.2?} ({:.1} us/thumb, {:.1} MB/s)",
                        cold_dur, cold_dur.as_micros() as f64 / total_hits as f64, real_mb / cold_dur.as_secs_f64());
                    println!("  Warm scroll total:   {:.2?} ({:.1} us/thumb, {:.1} MB/s)",
                        warm_dur, warm_dur.as_micros() as f64 / total_hits as f64, real_mb / warm_dur.as_secs_f64());
                }
            }
        }
    }
    println!("========================================================\n");
}
