//! Face suggestions for named people.
//!
//! Named (anchored) people imported from Picasa have face boxes but no embeddings.
//! `seed_named_embeddings` gives them one: it copies the embedding of a LapCasa-detected
//! face whose centre lies inside the imported box (and marks that detected face as a
//! duplicate of the named one), or computes one from the box with the recognition model.
//! `compute_suggestions` then compares every unconfirmed detected face with each named
//! person's sample faces (mean of the top-k cosine similarities) and marks the best
//! person as `suggested_person_id` when the score clears the threshold. Confirming
//! assigns the face; rejecting records the pair so it is never suggested again.
use crate::t_face::{FaceBox, FaceEngine};
use ndarray::{Array2, Axis};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// Tuned by eye on a test library: at 0.60
/// sampled precision was ~98% across three people; 0.55-0.60 drops to ~80%.
pub const DEFAULT_THRESHOLD: f32 = 0.60;
/// Score = mean of this many best similarities to the person's sample faces.
pub const TOP_K: usize = 3;
/// Best person must beat the runner-up by this much, or the face is ambiguous.
pub const MIN_MARGIN: f32 = 0.05;

// Detected (SCRFD) boxes are tighter than imported face rectangles. Measured on the
// 1,133 overlapping pairs: width x0.615, height x0.664, centre 4.6% of height lower.
const IMPORTED_TO_DET_W: f32 = 0.615;
const IMPORTED_TO_DET_H: f32 = 0.664;
const IMPORTED_TO_DET_DY: f32 = 0.046;

/// Additive schema for suggestions. Safe to run on every start.
pub fn migrate(conn: &Connection) -> Result<(), String> {
    for column in [
        "suggested_person_id INTEGER",
        "suggestion_score REAL",
        "duplicate_of INTEGER",
        // When the face was first suggested. Kept after a tick or an x, so the person
        // view's order (confirmed first, then suggestions by this) never shifts mid-review.
        "suggested_at INTEGER",
        // When a suggestion was last ticked or ignored: the suggestions-only view keeps
        // faces resolved since it was opened, so tiles stay put there too.
        "resolved_at INTEGER",
    ] {
        let name = column.split(' ').next().unwrap();
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('faces') WHERE name = ?1)",
                [name],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !exists {
            conn.execute(&format!("ALTER TABLE faces ADD COLUMN {column}"), [])
                .map_err(|e| e.to_string())?;
        }
    }
    conn.execute(
        "UPDATE faces SET suggested_at = 1 WHERE suggested_person_id IS NOT NULL AND suggested_at IS NULL",
        [],
    )
    .map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_faces_suggested_person_id ON faces(suggested_person_id);
         CREATE TABLE IF NOT EXISTS face_rejections (
             face_id INTEGER NOT NULL,
             person_id INTEGER NOT NULL,
             created_at INTEGER,
             PRIMARY KEY (face_id, person_id)
         );
         CREATE TABLE IF NOT EXISTS face_ignores (
             face_id INTEGER PRIMARY KEY REFERENCES faces(id) ON DELETE CASCADE,
             ignored_at INTEGER NOT NULL
         );",
    )
    .map_err(|e| e.to_string())
}

#[derive(Debug, Default, Serialize, Clone)]
pub struct SeedStats {
    pub from_overlap: usize,
    pub computed: usize,
    pub failed: usize,
}

#[derive(Debug, Default, Serialize, Clone)]
pub struct SuggestStats {
    pub named_people: usize,
    pub sample_faces: usize,
    pub candidates: usize,
    pub suggested: usize,
    pub threshold: f32,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
struct BoxJson {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

fn parse_box(json: &str) -> Option<BoxJson> {
    serde_json::from_str::<BoxJson>(json)
        .ok()
        .filter(|b| b.width > 0.0 && b.height > 0.0)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn parse_embedding(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.is_empty() || bytes.len() % 4 != 0 {
        return None;
    }
    let v: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    (norm > f32::EPSILON).then(|| v.iter().map(|x| x / norm).collect())
}

const NAMED: &str = "COALESCE(p.is_anchored, 0) = 1 AND TRIM(COALESCE(p.name, '')) <> ''";
const VISIBLE: &str = "NOT EXISTS (SELECT 1 FROM face_ignores i WHERE i.face_id = f.id)";

/// Give named people's faces an embedding. `engine` = None only copies from overlaps.
pub fn seed_named_embeddings(
    conn: &Connection,
    mut engine: Option<&mut FaceEngine>,
) -> Result<SeedStats, String> {
    let mut stats = SeedStats::default();
    let named: Vec<(i64, i64, String)> = conn
        .prepare(&format!(
            "SELECT f.id, f.file_id, f.bbox FROM faces f JOIN persons p ON p.id = f.person_id
             WHERE f.embedding IS NULL AND {NAMED} AND {VISIBLE} ORDER BY f.file_id"
        ))
        .map_err(|e| e.to_string())?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;

    let mut detected_stmt = conn
        .prepare(
            "SELECT f.id, f.bbox, f.embedding FROM faces f LEFT JOIN persons p ON p.id = f.person_id
             WHERE f.file_id = ?1 AND f.embedding IS NOT NULL AND f.duplicate_of IS NULL
               AND f.id NOT IN (SELECT face_id FROM face_ignores)
               AND (f.person_id IS NULL OR COALESCE(p.is_anchored, 0) = 0)",
        )
        .map_err(|e| e.to_string())?;
    let mut path_stmt = conn
        .prepare("SELECT b.path, a.name FROM afiles a JOIN afolders b ON b.id = a.folder_id WHERE a.id = ?1")
        .map_err(|e| e.to_string())?;

    let mut image_cache: Option<(i64, Option<image::DynamicImage>)> = None;
    for (face_id, file_id, bbox_json) in named {
        let Some(b) = parse_box(&bbox_json) else {
            stats.failed += 1;
            continue;
        };
        // 1. A detected face centred inside the imported box is the same physical face.
        let detected: Vec<(i64, String, Vec<u8>)> = detected_stmt
            .query_map([file_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        let (pcx, pcy) = (b.x + b.width / 2.0, b.y + b.height / 2.0);
        let best = detected
            .into_iter()
            .filter_map(|(id, json, emb)| {
                let d = parse_box(&json)?;
                let (cx, cy) = (d.x + d.width / 2.0, d.y + d.height / 2.0);
                let inside = cx >= b.x && cx <= b.x + b.width && cy >= b.y && cy <= b.y + b.height;
                inside.then(|| (id, (cx - pcx).powi(2) + (cy - pcy).powi(2), emb))
            })
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        if let Some((det_id, _, emb)) = best {
            conn.execute("UPDATE faces SET embedding = ?1 WHERE id = ?2", params![emb, face_id])
                .map_err(|e| e.to_string())?;
            conn.execute(
                "UPDATE faces SET duplicate_of = ?1, person_id = NULL, suggested_person_id = NULL,
                        suggestion_score = NULL WHERE id = ?2",
                params![face_id, det_id],
            )
            .map_err(|e| e.to_string())?;
            stats.from_overlap += 1;
            continue;
        }
        // 2. Otherwise embed the imported box (mapped to a detector-style box).
        let Some(engine) = engine.as_deref_mut() else { continue };
        if image_cache.as_ref().map(|c| c.0) != Some(file_id) {
            let img = path_stmt
                .query_row([file_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .optional()
                .map_err(|e| e.to_string())?
                .and_then(|(dir, name)| image::open(std::path::Path::new(&dir).join(name)).ok());
            image_cache = Some((file_id, img));
        }
        let Some(img) = image_cache.as_ref().and_then(|c| c.1.as_ref()) else {
            stats.failed += 1;
            continue;
        };
        let (iw, ih) = (img.width() as f32, img.height() as f32);
        let w = b.width * IMPORTED_TO_DET_W;
        let h = b.height * IMPORTED_TO_DET_H;
        let cy = pcy + b.height * IMPORTED_TO_DET_DY;
        let x = (pcx - w / 2.0).clamp(0.0, (iw - 2.0).max(0.0));
        let y = (cy - h / 2.0).clamp(0.0, (ih - 2.0).max(0.0));
        let fb = FaceBox {
            x,
            y,
            width: w.min(iw - x),
            height: h.min(ih - y),
            confidence: 1.0,
            landmarks: None,
        };
        if fb.width < 8.0 || fb.height < 8.0 {
            stats.failed += 1;
            continue;
        }
        match engine.embed_face_box(img, &fb) {
            Ok(emb) => {
                conn.execute(
                    "UPDATE faces SET embedding = ?1 WHERE id = ?2",
                    params![to_bytes(&emb), face_id],
                )
                .map_err(|e| e.to_string())?;
                stats.computed += 1;
            }
            Err(_) => stats.failed += 1,
        }
    }
    Ok(stats)
}

/// Recompute all pending suggestions. Rejected (face, person) pairs are never suggested,
/// nor is a person suggested in a photo where they are already confirmed, nor twice in one photo.
/// Every candidate's best score is stored in suggestion_score (for tuning); the person is
/// only set when the score clears `threshold` and beats the runner-up by MIN_MARGIN.
pub fn compute_suggestions(conn: &Connection, threshold: f32) -> Result<SuggestStats, String> {
    let mut stats = SuggestStats { threshold, ..Default::default() };

    // Samples: confirmed faces of named people.
    let samples: Vec<(i64, i64, Vec<u8>)> = conn
        .prepare(&format!(
            "SELECT f.person_id, f.file_id, f.embedding FROM faces f JOIN persons p ON p.id = f.person_id
             WHERE f.embedding IS NOT NULL AND {NAMED} AND {VISIBLE}"
        ))
        .map_err(|e| e.to_string())?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let mut people: Vec<i64> = Vec::new();
    let mut person_index: HashMap<i64, usize> = HashMap::new();
    let mut sample_person: Vec<usize> = Vec::new();
    let mut sample_rows: Vec<f32> = Vec::new();
    let mut confirmed_in_file: HashSet<(i64, usize)> = HashSet::new();
    for (pid, file_id, bytes) in &samples {
        let Some(emb) = parse_embedding(bytes) else { continue };
        if emb.len() != 512 {
            continue;
        }
        let idx = *person_index.entry(*pid).or_insert_with(|| {
            people.push(*pid);
            people.len() - 1
        });
        sample_person.push(idx);
        sample_rows.extend_from_slice(&emb);
        confirmed_in_file.insert((*file_id, idx));
    }
    stats.named_people = people.len();
    stats.sample_faces = sample_person.len();

    conn.execute("UPDATE faces SET suggested_person_id = NULL, suggestion_score = NULL", [])
        .map_err(|e| e.to_string())?;
    if people.is_empty() {
        return Ok(stats);
    }
    let sample_matrix = Array2::from_shape_vec((sample_person.len(), 512), sample_rows)
        .map_err(|e| e.to_string())?;

    let rejected: HashSet<(i64, i64)> = conn
        .prepare("SELECT face_id, person_id FROM face_rejections")
        .map_err(|e| e.to_string())?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;

    // Candidates: detected faces not confirmed for a named person and not duplicates.
    let candidates: Vec<(i64, i64, Vec<u8>)> = conn
        .prepare(
            "SELECT f.id, f.file_id, f.embedding FROM faces f LEFT JOIN persons p ON p.id = f.person_id
             WHERE f.embedding IS NOT NULL AND f.duplicate_of IS NULL
               AND f.id NOT IN (SELECT face_id FROM face_ignores)
               AND (f.person_id IS NULL OR COALESCE(p.is_anchored, 0) = 0)",
        )
        .map_err(|e| e.to_string())?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    stats.candidates = candidates.len();

    // (face_id, file_id, best person idx, best score, passes)
    let mut results: Vec<(i64, i64, usize, f32, bool)> = Vec::with_capacity(candidates.len());
    let n_people = people.len();
    for chunk in candidates.chunks(1024) {
        let mut rows = Vec::with_capacity(chunk.len() * 512);
        let mut kept = Vec::with_capacity(chunk.len());
        for (id, file_id, bytes) in chunk {
            if let Some(emb) = parse_embedding(bytes).filter(|e| e.len() == 512) {
                rows.extend_from_slice(&emb);
                kept.push((*id, *file_id));
            }
        }
        if kept.is_empty() {
            continue;
        }
        let m = Array2::from_shape_vec((kept.len(), 512), rows).map_err(|e| e.to_string())?;
        let sims = m.dot(&sample_matrix.t());
        for (row, (face_id, file_id)) in sims.axis_iter(Axis(0)).zip(kept) {
            // Top-k similarities per person.
            let mut top: Vec<[f32; TOP_K]> = vec![[f32::NEG_INFINITY; TOP_K]; n_people];
            for (s, &pi) in row.iter().zip(sample_person.iter()) {
                let t = &mut top[pi];
                if *s > t[TOP_K - 1] {
                    let mut i = TOP_K - 1;
                    while i > 0 && *s > t[i - 1] {
                        t[i] = t[i - 1];
                        i -= 1;
                    }
                    t[i] = *s;
                }
            }
            let mut best = (usize::MAX, f32::NEG_INFINITY);
            let mut second = f32::NEG_INFINITY;
            for (pi, t) in top.iter().enumerate() {
                if confirmed_in_file.contains(&(file_id, pi)) || rejected.contains(&(face_id, people[pi])) {
                    continue;
                }
                let vals: Vec<f32> = t.iter().copied().filter(|v| v.is_finite()).collect();
                if vals.is_empty() {
                    continue;
                }
                let score = vals.iter().sum::<f32>() / vals.len() as f32;
                if score > best.1 {
                    second = best.1;
                    best = (pi, score);
                } else if score > second {
                    second = score;
                }
            }
            if best.0 == usize::MAX {
                continue;
            }
            let passes = best.1 >= threshold && best.1 - second >= MIN_MARGIN;
            results.push((face_id, file_id, best.0, best.1, passes));
        }
    }

    // One suggestion per person per photo: keep the highest score.
    let mut best_in_file: HashMap<(i64, usize), (i64, f32)> = HashMap::new();
    for &(face_id, file_id, pi, score, passes) in &results {
        if passes {
            let e = best_in_file.entry((file_id, pi)).or_insert((face_id, score));
            if score > e.1 {
                *e = (face_id, score);
            }
        }
    }
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    {
        let mut update = tx
            .prepare(
                "UPDATE faces SET suggested_person_id = ?1, suggestion_score = ?2,
                        suggested_at = CASE WHEN ?1 IS NULL THEN suggested_at ELSE COALESCE(suggested_at, ?4) END
                 WHERE id = ?3",
            )
            .map_err(|e| e.to_string())?;
        for &(face_id, file_id, pi, score, passes) in &results {
            let suggested = passes && best_in_file.get(&(file_id, pi)).map(|e| e.0) == Some(face_id);
            let person: Option<i64> = suggested.then(|| people[pi]);
            stats.suggested += suggested as usize;
            update
                .execute(params![person, score as f64, face_id, now()])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(stats)
}

/// Confirm suggestions for a person (all of them when `face_ids` is None).
pub fn confirm(conn: &Connection, person_id: i64, face_ids: Option<&[i64]>) -> Result<usize, String> {
    let ids = id_filter(face_ids);
    conn.execute(
        &format!(
            "UPDATE faces SET person_id = ?1, suggested_person_id = NULL, suggestion_score = NULL,
                    resolved_at = ?2
             WHERE suggested_person_id = ?1{ids} AND id NOT IN (SELECT face_id FROM face_ignores)"
        ),
        params![person_id, now()],
    )
    .map_err(|e| e.to_string())
}

/// Learning step after confirms / undos for one person (suggestions keep coming as you
/// confirm): re-score the unconfirmed detected faces against everyone's samples, now that
/// this person has more confirmed faces, and add the faces where this person comes out on
/// top under the same rules as `compute_suggestions`. Existing suggestions are left alone.
/// Returns the file ids of the photos that gained a suggestion.
pub fn refresh_person(conn: &Connection, person_id: i64, threshold: f32) -> Result<Vec<i64>, String> {
    let samples: Vec<(i64, i64, Vec<u8>)> = conn
        .prepare(&format!(
            "SELECT f.person_id, f.file_id, f.embedding FROM faces f JOIN persons p ON p.id = f.person_id
             WHERE f.embedding IS NOT NULL AND {NAMED} AND {VISIBLE}"
        ))
        .map_err(|e| e.to_string())?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let mut people: Vec<i64> = Vec::new();
    let mut person_index: HashMap<i64, usize> = HashMap::new();
    let mut sample_person: Vec<usize> = Vec::new();
    let mut rows: Vec<f32> = Vec::new();
    let mut target_files: HashSet<i64> = HashSet::new();
    for (pid, file_id, bytes) in &samples {
        let Some(emb) = parse_embedding(bytes).filter(|e| e.len() == 512) else { continue };
        let idx = *person_index.entry(*pid).or_insert_with(|| {
            people.push(*pid);
            people.len() - 1
        });
        sample_person.push(idx);
        rows.extend_from_slice(&emb);
        if *pid == person_id {
            target_files.insert(*file_id);
        }
    }
    let Some(&target) = person_index.get(&person_id) else { return Ok(Vec::new()) };
    let sample_matrix = Array2::from_shape_vec((sample_person.len(), 512), rows).map_err(|e| e.to_string())?;
    // Cheap pre-filter: only faces whose best match among this person's own samples clears
    // the threshold can win (the top-k mean never exceeds the best), so only those get the
    // full comparison against everyone. Keeps a tick's learning step fast.
    let target_rows: Vec<usize> = (0..sample_person.len()).filter(|&i| sample_person[i] == target).collect();
    let target_matrix = sample_matrix.select(Axis(0), &target_rows);

    // Photos that already hold a suggestion for this person: at most one per photo.
    let suggested_files: HashSet<i64> = conn
        .prepare("SELECT file_id FROM faces WHERE suggested_person_id = ?1")
        .map_err(|e| e.to_string())?
        .query_map([person_id], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let candidates: Vec<(i64, i64, Vec<u8>)> = conn
        .prepare(
            "SELECT f.id, f.file_id, f.embedding FROM faces f LEFT JOIN persons p ON p.id = f.person_id
             WHERE f.embedding IS NOT NULL AND f.duplicate_of IS NULL AND f.suggested_person_id IS NULL
               AND f.id NOT IN (SELECT face_id FROM face_ignores)
               AND (f.person_id IS NULL OR COALESCE(p.is_anchored, 0) = 0)
               AND NOT EXISTS (SELECT 1 FROM face_rejections r WHERE r.face_id = f.id AND r.person_id = ?1)",
        )
        .map_err(|e| e.to_string())?
        .query_map([person_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;

    // (face_id, file_id, score) for faces where this person wins.
    let mut winners: Vec<(i64, i64, f32)> = Vec::new();
    for chunk in candidates.chunks(1024) {
        let mut buf = Vec::with_capacity(chunk.len() * 512);
        let mut kept = Vec::with_capacity(chunk.len());
        for (id, file_id, bytes) in chunk {
            if target_files.contains(file_id) || suggested_files.contains(file_id) {
                continue;
            }
            if let Some(emb) = parse_embedding(bytes).filter(|e| e.len() == 512) {
                buf.extend_from_slice(&emb);
                kept.push((*id, *file_id));
            }
        }
        if kept.is_empty() {
            continue;
        }
        let m = Array2::from_shape_vec((kept.len(), 512), buf).map_err(|e| e.to_string())?;
        let own = m.dot(&target_matrix.t());
        let plausible: Vec<usize> = own
            .axis_iter(Axis(0))
            .enumerate()
            .filter(|(_, r)| r.iter().any(|s| *s >= threshold))
            .map(|(i, _)| i)
            .collect();
        if plausible.is_empty() {
            continue;
        }
        let m = m.select(Axis(0), &plausible);
        let kept: Vec<(i64, i64)> = plausible.iter().map(|&i| kept[i]).collect();
        let sims = m.dot(&sample_matrix.t());
        for (row, (face_id, file_id)) in sims.axis_iter(Axis(0)).zip(kept) {
            let mut top: Vec<[f32; TOP_K]> = vec![[f32::NEG_INFINITY; TOP_K]; people.len()];
            for (s, &pi) in row.iter().zip(sample_person.iter()) {
                let t = &mut top[pi];
                if *s > t[TOP_K - 1] {
                    let mut i = TOP_K - 1;
                    while i > 0 && *s > t[i - 1] {
                        t[i] = t[i - 1];
                        i -= 1;
                    }
                    t[i] = *s;
                }
            }
            let mean = |t: &[f32; TOP_K]| {
                let v: Vec<f32> = t.iter().copied().filter(|x| x.is_finite()).collect();
                (!v.is_empty()).then(|| v.iter().sum::<f32>() / v.len() as f32)
            };
            let Some(score) = mean(&top[target]) else { continue };
            if score < threshold {
                continue;
            }
            let runner_up = top
                .iter()
                .enumerate()
                .filter(|(pi, _)| *pi != target)
                .filter_map(|(_, t)| mean(t))
                .fold(f32::NEG_INFINITY, f32::max);
            if score - runner_up >= MIN_MARGIN {
                winners.push((face_id, file_id, score));
            }
        }
    }
    // One per photo: keep the best-scoring face.
    let mut best: HashMap<i64, (i64, f32)> = HashMap::new();
    for (face_id, file_id, score) in winners {
        let e = best.entry(file_id).or_insert((face_id, score));
        if score > e.1 {
            *e = (face_id, score);
        }
    }
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let stamp = now();
    for (face_id, score) in best.values() {
        tx.execute(
            "UPDATE faces SET suggested_person_id = ?1, suggestion_score = ?2,
                    suggested_at = COALESCE(suggested_at, ?4) WHERE id = ?3",
            params![person_id, *score as f64, face_id, stamp],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    let mut files: Vec<i64> = best.keys().copied().collect();
    files.sort();
    Ok(files)
}

/// Undo an "x": forget the rejections, then let the person's matching pick them up again.
pub fn unreject(conn: &Connection, person_id: i64, face_ids: &[i64]) -> Result<usize, String> {
    if face_ids.is_empty() {
        return Ok(0);
    }
    conn.execute(
        &format!("DELETE FROM face_rejections WHERE person_id = ?1{}", id_filter_col("face_id", face_ids)),
        [person_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(refresh_person(conn, person_id, DEFAULT_THRESHOLD)?.len())
}

/// "Not this person" for faces already confirmed for them: unassign, record the rejection
/// (so learning never suggests them back), and refresh the person's cover thumbnail if it
/// was one of these faces. Files on disk are untouched. Undo = `confirm_ignored`.
pub fn exclude_confirmed(conn: &Connection, person_id: i64, face_ids: &[i64]) -> Result<usize, String> {
    if face_ids.is_empty() {
        return Ok(0);
    }
    let ids = id_filter(Some(face_ids));
    conn.execute(
        &format!(
            "INSERT OR IGNORE INTO face_rejections (face_id, person_id, created_at)
             SELECT id, ?1, ?2 FROM faces WHERE person_id = ?1{ids}"
        ),
        params![person_id, now()],
    )
    .map_err(|e| e.to_string())?;
    let n = conn
        .execute(
            &format!("UPDATE faces SET person_id = NULL, resolved_at = ?2 WHERE person_id = ?1{ids}"),
            params![person_id, now()],
        )
        .map_err(|e| e.to_string())?;
    conn.execute(
        &format!(
            "UPDATE persons SET cover_face_id = NULL, thumbnail = NULL
             WHERE id = ?1 AND cover_face_id IN ({})",
            face_ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
        ),
        [person_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(n)
}

/// From the ignored list: "this is them after all" - forget the rejection and confirm.
pub fn confirm_ignored(conn: &Connection, person_id: i64, face_ids: &[i64]) -> Result<usize, String> {
    if face_ids.is_empty() {
        return Ok(0);
    }
    conn.execute(
        &format!("DELETE FROM face_rejections WHERE person_id = ?1{}", id_filter_col("face_id", face_ids)),
        [person_id],
    )
    .map_err(|e| e.to_string())?;
    let n = conn
        .execute(
            &format!(
                "UPDATE faces SET person_id = ?1, suggested_person_id = NULL, suggestion_score = NULL
                 WHERE 1 = 1{}",
                id_filter(Some(face_ids))
            ),
            [person_id],
        )
        .map_err(|e| e.to_string())?;
    refresh_person(conn, person_id, DEFAULT_THRESHOLD)?;
    Ok(n)
}

/// A person's ignored faces: (file_id, face_id, bbox json).
pub fn ignored_for_person(conn: &Connection, person_id: i64) -> Result<Vec<(i64, i64, String)>, String> {
    conn.prepare(
        "SELECT f.file_id, f.id, COALESCE(f.bbox, '') FROM face_rejections r JOIN faces f ON f.id = r.face_id
         WHERE r.person_id = ?1 ORDER BY f.file_id",
    )
    .map_err(|e| e.to_string())?
    .query_map([person_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
    .map_err(|e| e.to_string())?
    .collect::<Result<_, _>>()
    .map_err(|e| e.to_string())
}

fn id_filter_col(col: &str, ids: &[i64]) -> String {
    format!(" AND {col} IN ({})", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","))
}

/// Similarity at which confirming one face also confirms a pending suggestion of the same
/// person (near-identical faces drop out of the list together).
/// Checked by eye on one person's suggestion pairs at 0.80-0.85: all the same person.
pub const CASCADE_THRESHOLD: f32 = 0.80;

/// Confirm the given suggestions, then cascade: every other pending suggestion of the person
/// whose embedding is within CASCADE_THRESHOLD of a newly confirmed face is confirmed too,
/// repeatedly until nothing more joins. Returns the file ids of every face confirmed.
pub fn confirm_with_cascade(conn: &Connection, person_id: i64, face_ids: &[i64], cascade_threshold: f32) -> Result<Vec<i64>, String> {
    let pending: Vec<(i64, i64, Vec<u8>)> = conn
        .prepare("SELECT id, file_id, embedding FROM faces WHERE suggested_person_id = ?1 AND id NOT IN (SELECT face_id FROM face_ignores)")
        .map_err(|e| e.to_string())?
        .query_map([person_id], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<Vec<u8>>>(2)?.unwrap_or_default())))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let embs: Vec<Option<Vec<f32>>> = pending
        .iter()
        .map(|(_, _, b)| parse_embedding(b).filter(|e| e.len() == 512).map(|mut e| {
            let n = e.iter().map(|v| v * v).sum::<f32>().sqrt();
            if n > 0.0 { e.iter_mut().for_each(|v| *v /= n); }
            e
        }))
        .collect();
    let requested: HashSet<i64> = face_ids.iter().copied().collect();
    let mut chosen: Vec<bool> = pending.iter().map(|(id, _, _)| requested.contains(id)).collect();
    let mut frontier: Vec<usize> = (0..pending.len()).filter(|&i| chosen[i]).collect();
    while let Some(i) = frontier.pop() {
        let Some(a) = &embs[i] else { continue };
        for j in 0..pending.len() {
            if chosen[j] {
                continue;
            }
            if let Some(b) = &embs[j] {
                let sim: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
                if sim >= cascade_threshold {
                    chosen[j] = true;
                    frontier.push(j);
                }
            }
        }
    }
    let ids: Vec<i64> = pending.iter().zip(&chosen).filter(|(_, c)| **c).map(|(p, _)| p.0).collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    confirm(conn, person_id, Some(&ids))?;
    Ok(pending.iter().zip(&chosen).filter(|(_, c)| **c).map(|(p, _)| p.1).collect())
}

/// Reject suggestions for a person (all of them when `face_ids` is None); never re-suggested.
pub fn reject(conn: &Connection, person_id: i64, face_ids: Option<&[i64]>) -> Result<usize, String> {
    let ids = id_filter(face_ids);
    conn.execute(
        &format!(
            "INSERT OR IGNORE INTO face_rejections (face_id, person_id, created_at)
             SELECT id, ?1, ?2 FROM faces WHERE suggested_person_id = ?1{ids}"
        ),
        params![person_id, now()],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        &format!(
            "UPDATE faces SET suggested_person_id = NULL, suggestion_score = NULL, resolved_at = ?2
             WHERE suggested_person_id = ?1{ids}"
        ),
        params![person_id, now()],
    )
    .map_err(|e| e.to_string())
}

fn id_filter(face_ids: Option<&[i64]>) -> String {
    match face_ids {
        None => String::new(),
        Some(ids) => format!(
            " AND id IN ({})",
            ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emb(v: &[(usize, f32)]) -> Vec<u8> {
        let mut e = vec![0f32; 512];
        for &(i, x) in v {
            e[i] = x;
        }
        to_bytes(&e)
    }

    fn fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE persons (id INTEGER PRIMARY KEY, name TEXT, is_anchored INTEGER);
             CREATE TABLE faces (id INTEGER PRIMARY KEY, file_id INTEGER, bbox TEXT, embedding BLOB, person_id INTEGER);
             CREATE TABLE afolders (id INTEGER PRIMARY KEY, path TEXT);
             CREATE TABLE afiles (id INTEGER PRIMARY KEY, folder_id INTEGER, name TEXT);
             INSERT INTO persons VALUES (1, 'Ann', 1), (2, 'Bea', 1), (3, 'Person 1', 0);",
        )
        .unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap(); // idempotent
        let b = r#"{"x":0,"y":0,"width":10,"height":10}"#;
        let rows: Vec<(i64, i64, Vec<u8>, Option<i64>)> = vec![
            (1, 100, emb(&[(0, 1.0)]), Some(1)),             // Ann sample
            (2, 101, emb(&[(1, 1.0)]), Some(2)),             // Bea sample
            (3, 200, emb(&[(0, 0.9), (2, 0.1)]), Some(3)),   // looks like Ann
            (4, 200, emb(&[(0, 0.8), (2, 0.6)]), None),      // also Ann-ish, same photo, weaker
            (5, 100, emb(&[(0, 1.0)]), None),                // Ann-like but Ann already in photo
            (6, 300, emb(&[(2, 1.0)]), None),                // nobody
            (7, 301, emb(&[(1, 0.95), (3, 0.05)]), None),    // Bea
        ];
        for (id, file, e, p) in rows {
            conn.execute("INSERT INTO faces (id, file_id, bbox, embedding, person_id) VALUES (?1,?2,?3,?4,?5)",
                params![id, file, b, e, p]).unwrap();
        }
        conn
    }

    fn suggested(conn: &Connection) -> Vec<(i64, i64)> {
        conn.prepare("SELECT id, suggested_person_id FROM faces WHERE suggested_person_id IS NOT NULL ORDER BY id")
            .unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
            .collect::<Result<_, _>>().unwrap()
    }

    #[test]
    fn global_ignores_are_not_suggested_or_confirmed() {
        let conn = fixture();
        conn.execute("INSERT INTO face_ignores VALUES (3, 1), (7, 1)", []).unwrap();
        compute_suggestions(&conn, 0.5).unwrap();
        assert_eq!(suggested(&conn), vec![(4, 1)]);
        conn.execute("UPDATE faces SET suggested_person_id = 2 WHERE id = 7", []).unwrap();
        assert_eq!(confirm(&conn, 2, None).unwrap(), 0);
        assert!(confirm_with_cascade(&conn, 2, &[7], CASCADE_THRESHOLD).unwrap().is_empty());
        conn.execute("DELETE FROM face_ignores WHERE face_id=7", []).unwrap();
        assert_eq!(confirm(&conn, 2, Some(&[7])).unwrap(), 1);
    }

    #[test]
    fn suggests_confirms_and_rejects() {
        let conn = fixture();
        let stats = compute_suggestions(&conn, 0.5).unwrap();
        assert_eq!(stats.named_people, 2);
        // 4 loses to 3 in the same photo; 5 is excluded because Ann is confirmed in file 100.
        assert_eq!(suggested(&conn), vec![(3, 1), (7, 2)]);

        assert_eq!(reject(&conn, 1, Some(&[3])).unwrap(), 1);
        compute_suggestions(&conn, 0.5).unwrap();
        // 3 rejected for Ann, so the weaker face 4 in the same photo may now be suggested.
        assert_eq!(suggested(&conn), vec![(4, 1), (7, 2)]);

        assert_eq!(confirm(&conn, 2, None).unwrap(), 1);
        let p: i64 = conn.query_row("SELECT person_id FROM faces WHERE id = 7", [], |r| r.get(0)).unwrap();
        assert_eq!(p, 2);
        assert_eq!(suggested(&conn), vec![(4, 1)]);
    }

    #[test]
    fn confirm_cascades_to_near_identical_suggestions() {
        let conn = fixture();
        // Three pending Ann suggestions: 20 and 21 near-identical (chain via 21 to 22), 23 unlike.
        let rows: Vec<(i64, i64, Vec<u8>)> = vec![
            (20, 500, emb(&[(0, 1.0)])),
            (21, 501, emb(&[(0, 0.95), (4, 0.31)])),   // sim to 20 ~0.95
            (22, 502, emb(&[(0, 0.75), (4, 0.66)])),   // sim to 20 ~0.75, to 21 ~0.92 (chained)
            (23, 503, emb(&[(0, 0.6), (5, 0.8)])),     // sim to all <= 0.6
        ];
        for (id, file, e) in rows {
            conn.execute("INSERT INTO faces (id, file_id, embedding, suggested_person_id) VALUES (?1,?2,?3,1)",
                params![id, file, e]).unwrap();
        }
        let mut files = confirm_with_cascade(&conn, 1, &[20], CASCADE_THRESHOLD).unwrap();
        files.sort();
        assert_eq!(files, vec![500, 501, 502]);
        let still: Vec<(i64, i64)> = suggested(&conn).into_iter().filter(|(id, _)| *id >= 20).collect();
        assert_eq!(still, vec![(23, 1)]);
    }

    #[test]
    fn confirming_teaches_new_suggestions_and_undo_restores() {
        let conn = fixture();
        // 30: pending Ann suggestion. 31: too unlike Ann's only sample (sim 0.5) to be suggested,
        // but very like face 30 - so once 30 is confirmed, Ann should learn and suggest 31.
        conn.execute("INSERT INTO faces (id, file_id, embedding, suggested_person_id) VALUES (30, 600, ?1, 1)",
            [emb(&[(0, 0.7), (6, 0.71)])]).unwrap();
        conn.execute("INSERT INTO faces (id, file_id, embedding) VALUES (31, 601, ?1)",
            [emb(&[(0, 0.5), (6, 0.866)])]).unwrap();
        let sugg = |conn: &Connection| -> Option<i64> {
            conn.query_row("SELECT suggested_person_id FROM faces WHERE id = 31", [], |r| r.get(0)).unwrap()
        };
        refresh_person(&conn, 1, DEFAULT_THRESHOLD).unwrap();
        assert_eq!(sugg(&conn), None);
        confirm(&conn, 1, Some(&[30])).unwrap();
        refresh_person(&conn, 1, DEFAULT_THRESHOLD).unwrap();
        assert_eq!(sugg(&conn), Some(1));

        // Accidental x on 31, then undo: it comes back as a suggestion.
        reject(&conn, 1, Some(&[31])).unwrap();
        assert_eq!(ignored_for_person(&conn, 1).unwrap().len(), 1);
        refresh_person(&conn, 1, DEFAULT_THRESHOLD).unwrap();
        assert_eq!(sugg(&conn), None);
        unreject(&conn, 1, &[31]).unwrap();
        assert_eq!(sugg(&conn), Some(1));
        assert!(ignored_for_person(&conn, 1).unwrap().is_empty());

        // From the ignored list straight to confirmed.
        reject(&conn, 1, Some(&[31])).unwrap();
        assert_eq!(confirm_ignored(&conn, 1, &[31]).unwrap(), 1);
        let p: Option<i64> = conn.query_row("SELECT person_id FROM faces WHERE id = 31", [], |r| r.get(0)).unwrap();
        assert_eq!(p, Some(1));
    }

    #[test]
    fn not_this_person_excludes_and_undo_restores() {
        let conn = fixture();
        // Real persons table has these (t_sqlite create_db); the shared fixture does not.
        conn.execute_batch("ALTER TABLE persons ADD COLUMN cover_face_id INTEGER;
                            ALTER TABLE persons ADD COLUMN thumbnail BLOB;").unwrap();
        conn.execute("UPDATE persons SET cover_face_id = 1 WHERE id = 1", []).unwrap();
        // Face 1 is a confirmed Ann sample; "Not Ann" takes it off her and blocks suggestions.
        assert_eq!(exclude_confirmed(&conn, 1, &[1]).unwrap(), 1);
        let (p, cover): (Option<i64>, Option<i64>) = conn
            .query_row("SELECT f.person_id, (SELECT cover_face_id FROM persons WHERE id = 1) FROM faces f WHERE f.id = 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((p, cover), (None, None));
        assert_eq!(ignored_for_person(&conn, 1).unwrap().len(), 1);
        // Undo: back to Ann, rejection gone.
        assert_eq!(confirm_ignored(&conn, 1, &[1]).unwrap(), 1);
        let p: Option<i64> = conn.query_row("SELECT person_id FROM faces WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(p, Some(1));
        assert!(ignored_for_person(&conn, 1).unwrap().is_empty());
    }

    #[test]
    fn overlap_seeding_marks_duplicate() {
        let conn = fixture();
        conn.execute("INSERT INTO faces (id, file_id, bbox, person_id) VALUES (10, 400, '{\"x\":0,\"y\":0,\"width\":100,\"height\":100}', 1)", []).unwrap();
        conn.execute("INSERT INTO faces (id, file_id, bbox, embedding, person_id) VALUES (11, 400, '{\"x\":20,\"y\":20,\"width\":60,\"height\":60}', ?1, 3)", [emb(&[(0, 1.0)])]).unwrap();
        let stats = seed_named_embeddings(&conn, None).unwrap();
        assert_eq!(stats.from_overlap, 1);
        let (dup, pid): (Option<i64>, Option<i64>) = conn.query_row("SELECT duplicate_of, person_id FROM faces WHERE id = 11", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((dup, pid), (Some(10), None));
        let has: bool = conn.query_row("SELECT embedding IS NOT NULL FROM faces WHERE id = 10", [], |r| r.get(0)).unwrap();
        assert!(has);
    }
}
