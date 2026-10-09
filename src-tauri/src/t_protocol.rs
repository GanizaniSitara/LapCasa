use tauri::{Builder, Wry};

use crate::{t_image, t_sqlite};

/// Opt-in stage timings for a scrolling capture; never log photo paths or bytes.
pub(crate) struct ThumbnailTrace {
    file_id: i64,
    kind: &'static str,
    clock: Option<(std::time::Instant, std::time::Instant)>,
}

impl ThumbnailTrace {
    pub(crate) fn new(file_id: i64, kind: &'static str) -> Self {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let enabled = *ENABLED.get_or_init(|| std::env::var_os("LAP_THUMB_TRACE").is_some());
        let clock = enabled.then(|| {
            let now = std::time::Instant::now();
            (now, now)
        });
        Self { file_id, kind, clock }
    }

    pub(crate) fn mark(&mut self, stage: &str, bytes: usize) {
        if let Some((start, last)) = self.clock.as_mut() {
            let now = std::time::Instant::now();
            eprintln!("{}", serde_json::json!({
                "event": "thumbnail_profile", "kind": self.kind, "file_id": self.file_id,
                "stage": stage, "ms": now.duration_since(*last).as_secs_f64() * 1000.0,
                "total_ms": now.duration_since(*start).as_secs_f64() * 1000.0, "bytes": bytes,
            }).to_string());
            *last = now;
        }
    }
}

fn text_response(status: http::StatusCode, body: &str) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "text/plain")
        .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(http::header::ACCESS_CONTROL_ALLOW_METHODS, "GET, OPTIONS")
        .header(http::header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
        .body(body.as_bytes().to_vec())
        .unwrap()
}

fn detect_image_mime(data: &[u8]) -> &'static str {
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if data.starts_with(&[0x47, 0x49, 0x46, 0x38]) {
        "image/gif"
    } else if data.starts_with(&[0x49, 0x49, 0x2A, 0x00])
        || data.starts_with(&[0x4D, 0x4D, 0x00, 0x2A])
        || data.starts_with(&[0x4D, 0x4D, 0x00, 0x2B])
        || data.starts_with(&[0x49, 0x49, 0x2B, 0x00])
    {
        "image/tiff"
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        detect_isobmff_mime(data)
    }
}

fn detect_isobmff_mime(data: &[u8]) -> &'static str {
    if data.len() < 16 || &data[4..8] != b"ftyp" {
        return "application/octet-stream";
    }

    let box_size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if box_size < 16 || box_size > data.len() {
        return "application/octet-stream";
    }

    match &data[8..12] {
        b"avif" | b"avis" => "image/avif",
        b"heic" | b"heix" | b"hevc" | b"hevx" => "image/heic",
        b"mif1" | b"heif" => "image/heif",
        _ => {
            for brand in data[16..box_size].chunks(4) {
                if brand.len() < 4 {
                    break;
                }
                match brand {
                    b"avif" | b"avis" => return "image/avif",
                    b"heic" | b"heix" | b"hevc" | b"hevx" => {
                        return "image/heic";
                    }
                    b"mif1" | b"heif" => return "image/heif",
                    _ => {}
                }
            }
            "application/octet-stream"
        }
    }
}

fn image_response(data: Vec<u8>) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(http::StatusCode::OK)
        .header(http::header::CONTENT_TYPE, detect_image_mime(&data))
        .header(http::header::CACHE_CONTROL, "max-age=31536000, immutable")
        .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(http::header::ACCESS_CONTROL_ALLOW_METHODS, "GET, OPTIONS")
        .header(http::header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
        .body(data)
        .unwrap()
}

fn has_recipe_revision(uri: &http::Uri) -> bool {
    uri.query().unwrap_or("").split('&').any(|p|p.strip_prefix("er=").and_then(|n|n.parse::<u64>().ok()).is_some_and(|n|n>0))
}

async fn recipe_response(file_id: i64, thumbnail: bool) -> http::Response<Vec<u8>> {
    match crate::t_edit_recipe::render(file_id,thumbnail).await {
        Ok(data)=>image_response(data),
        Err(error)=>text_response(http::StatusCode::NOT_FOUND,&error),
    }
}

// SQLite, image decoding and redb transactions are synchronous. Keep them off
// both the window callback and the async executor used by grid/navigation IPC.
pub(crate) async fn thumbnail_work<F, R>(work: F) -> Result<R, String>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    static WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
    let permit = WORKERS.acquire().await.map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        work()
    }).await.map_err(|e| e.to_string())
}

type DecodeJob = Box<dyn FnOnce() + Send>;

struct DecodeQueue {
    state: std::sync::Mutex<(usize, Vec<DecodeJob>)>,
}

impl DecodeQueue {
    const fn new() -> Self {
        Self { state: std::sync::Mutex::new((0, Vec::new())) }
    }

    async fn run<F, R>(&'static self, work: F) -> Result<R, String>
    where F: FnOnce() -> R + Send + 'static, R: Send + 'static {
        let (send, receive) = tokio::sync::oneshot::channel();
        {
            let mut state = self.state.lock().map_err(|e| e.to_string())?;
            state.1.push(Box::new(move || { let _ = send.send(work()); }));
            if state.0 < 8 {
                state.0 += 1;
                tauri::async_runtime::spawn_blocking(move || loop {
                    let job = {
                        let mut state = self.state.lock().unwrap();
                        // Requests from the current viewport arrive last. Do not
                        // make them wait behind photos already scrolled past.
                        match state.1.pop() {
                            Some(job) => job,
                            None => { state.0 -= 1; return; }
                        }
                    };
                    // A failed decoder must not permanently consume a worker.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                });
            }
        }
        receive.await.map_err(|e| e.to_string())
    }
}

async fn thumbnail_decode_work<F, R>(work: F) -> Result<R, String>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    static DECODERS: DecodeQueue = DecodeQueue::new();
    DECODERS.run(work).await
}

async fn thumbnail_response(file_id: i64, library_id: String, kind: &'static str) -> http::Response<Vec<u8>> {
    // Cache hits must not queue behind decodes of photos already scrolled past.
    let cached = thumbnail_work(move || {
        crate::t_thumb_cache::get(file_id).map(|data| {
            image_response(if crate::t_enhance::is_file_auto_enhanced(file_id) {
                crate::t_enhance::apply_auto_enhance_to_bytes_for_file(&data, Some(file_id)).unwrap_or(data)
            } else { data })
        })
    }).await;
    match cached {
        Ok(Some(response)) => return response,
        Err(error) => return text_response(http::StatusCode::INTERNAL_SERVER_ERROR, &error),
        _ => {}
    }
    let mut trace = ThumbnailTrace::new(file_id, kind);
    thumbnail_decode_work(move || {
        trace.mark("executor_wait", 0);
        let has_auto_enhance = crate::t_enhance::is_file_auto_enhanced(file_id);
        trace.mark("edit_lookup", 0);
        let data = if let Some(data) = crate::t_thumb_cache::get(file_id) {
            trace.mark("cache_hit", data.len());
            Some(data)
        } else {
            trace.mark("cache_miss", 0);
            let data = t_sqlite::AThumb::fetch_raw_for_library(file_id, &library_id).ok().flatten();
            trace.mark("sql_thumbnail", data.as_ref().map_or(0, Vec::len));
            data.or_else(|| {
                let file = t_sqlite::AFile::get_file_info(file_id).ok().flatten()?;
                let path = file.file_path?;
                // A grid cache miss must produce a thumbnail, not send a full
                // original while separately queuing the same decode again.
                let thumb = t_sqlite::AThumb::get_or_create_thumb(
                    file_id, &path, file.file_type.unwrap_or(0),
                    file.e_orientation.unwrap_or(1) as i32, 200, false, false,
                    file.duration.map(|d| d as u64), None,
                ).ok().flatten()?;
                let data = if thumb.error_code == 2 {
                    std::fs::read(path).ok()
                } else {
                    thumb.thumb_data
                };
                trace.mark("generated_thumbnail", data.as_ref().map_or(0, Vec::len));
                data
            })
        };
        match data {
            Some(data) => image_response(if has_auto_enhance {
                crate::t_enhance::apply_auto_enhance_to_bytes_for_file(&data, Some(file_id)).unwrap_or(data)
            } else { data }),
            None => text_response(http::StatusCode::NOT_FOUND, "thumbnail not found"),
        }
    }).await.unwrap_or_else(|error| text_response(http::StatusCode::INTERNAL_SERVER_ERROR, &error))
}

pub fn register_protocols(builder: Builder<Wry>) -> Builder<Wry> {
    builder
        .register_asynchronous_uri_scheme_protocol("thumb", |_ctx, request, responder| {
            // URL format: thumb://localhost/{library_id}/{file_id}
            // library_id is also used for cache namespace isolation.
            let path = request.uri().path();
            let mut segments = path.trim_start_matches('/').split('/');
            let library_id = segments.next().unwrap_or("default").to_string();
            let file_id_str = segments.next().unwrap_or("");
            let file_id: i64 = file_id_str.parse().unwrap_or(0);

            if library_id.is_empty() || file_id <= 0 {
                responder.respond(text_response(
                    http::StatusCode::BAD_REQUEST,
                    "invalid file_id",
                ));
                return;
            }

            if has_recipe_revision(request.uri()) {
                tauri::async_runtime::spawn(async move {responder.respond(recipe_response(file_id,true).await);});
                return;
            }
            tauri::async_runtime::spawn(async move {
                responder.respond(thumbnail_response(file_id, library_id, "thumb").await);
            });
        })
        .register_asynchronous_uri_scheme_protocol("lap", |_ctx, request, responder| {
            // URL format: lap://localhost/{library_id}/{file_id} or lap://localhost/{file_id}
            let path = request.uri().path();
            let mut segments = path.trim_start_matches('/').split('/');
            let first = segments.next().unwrap_or("default");
            let second = segments.next().unwrap_or("");
            let (library_id, file_id) = if let Ok(id) = first.parse::<i64>() {
                ("default".to_string(), id)
            } else {
                (first.to_string(), second.parse::<i64>().unwrap_or(0))
            };

            if file_id <= 0 {
                responder.respond(text_response(
                    http::StatusCode::BAD_REQUEST,
                    "invalid file_id",
                ));
                return;
            }

            if request.uri().query().unwrap_or("").split('&').any(|p| p.starts_with("fx=")) {
                let Some((rect, size, aspect, face_id)) = crate::t_face_thumb::parse_crop(request.uri().query().unwrap_or("")) else {
                    responder.respond(text_response(http::StatusCode::BAD_REQUEST, "invalid face crop"));
                    return;
                };
                let key = format!("face-v4:{}", request.uri());
                tauri::async_runtime::spawn(async move {
                    let response = match crate::t_face_thumb::get(key, file_id, rect, size, aspect, face_id).await {
                        Ok(data) => image_response(data),
                        Err(error) => text_response(http::StatusCode::NOT_FOUND, &error),
                    };
                    responder.respond(response);
                });
                return;
            }

            if has_recipe_revision(request.uri()) {
                tauri::async_runtime::spawn(async move {responder.respond(recipe_response(file_id,true).await);});
                return;
            }

            tauri::async_runtime::spawn(async move {
                responder.respond(thumbnail_response(file_id, library_id, "lap").await);
            });
        })
        .register_asynchronous_uri_scheme_protocol("preview", |_ctx, request, responder| {
            // URL format: preview://localhost/{library_id}/{file_id}?rawThumbnailSource=embedded
            // library_id is for browser cache isolation only; file_id is the last segment
            let path = request.uri().path();
            let file_id_str = path.rsplit('/').next().unwrap_or("");
            let file_id: i64 = file_id_str.parse().unwrap_or(0);
            let prefer_embedded_raw_preview = request
                .uri()
                .query()
                .is_some_and(|query| query.split('&').any(|item| item == "rawThumbnailSource=embedded"));

            if request.uri().query().unwrap_or("").split('&').any(|p|p=="baseEdit=1") {
                tauri::async_runtime::spawn(async move {
                    responder.respond(match crate::t_edit_recipe::render_base(file_id).await {
                        Ok(data)=>image_response(data),Err(error)=>text_response(http::StatusCode::NOT_FOUND,&error),
                    });
                });
                return;
            }
            if has_recipe_revision(request.uri()) {
                tauri::async_runtime::spawn(async move {responder.respond(recipe_response(file_id,false).await);});
                return;
            }

            if file_id <= 0 {
                responder.respond(text_response(
                    http::StatusCode::BAD_REQUEST,
                    "invalid file_id",
                ));
                return;
            }

            let original=request.uri().query().unwrap_or("").split('&').any(|p|p=="original=1");
            tauri::async_runtime::spawn(async move {
                let file = match thumbnail_work(move || t_sqlite::AFile::get_file_info(file_id)).await {
                    Ok(Ok(Some(file))) => file,
                    _ => {
                        responder.respond(text_response(http::StatusCode::NOT_FOUND, "file not found"));
                        return;
                    }
                };

                let file_path = match file.file_path {
                    Some(path) if !path.is_empty() => path,
                    _ => {
                        responder.respond(text_response(
                            http::StatusCode::NOT_FOUND,
                            "file path not found",
                        ));
                        return;
                    }
                };

                let has_auto_enhance = !original && file
                    .edits
                    .as_deref()
                    .map(crate::t_enhance::is_auto_enhance_active)
                    .unwrap_or(false);

                let response = match t_image::get_file_image_bytes_cached(
                    &file_path,
                    prefer_embedded_raw_preview,
                )
                .await
                {
                    Ok(data) => {
                        let final_data = if has_auto_enhance {
                            crate::t_enhance::apply_auto_enhance_to_bytes_for_file(&data, Some(file_id)).unwrap_or(data)
                        } else {
                            data
                        };
                        image_response(final_data)
                    }
                    Err(_) => text_response(http::StatusCode::NOT_FOUND, "preview not found"),
                };
                responder.respond(response);
            });
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};
    use std::time::Duration;

    #[tokio::test(flavor = "current_thread")]
    async fn newest_thumbnail_overtakes_the_scrolled_past_backlog() {
        static QUEUE: DecodeQueue = DecodeQueue::new();
        let active = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(AtomicBool::new(false));
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut jobs = Vec::new();
        for _ in 0..8 {
            let (active, release) = (active.clone(), release.clone());
            jobs.push(tokio::spawn(QUEUE.run(move || {
                active.fetch_add(1, Ordering::SeqCst);
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                while !release.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(5));
                }
            })));
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while active.load(Ordering::SeqCst) != 8 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.unwrap();
        for id in 0..32 {
            let order = order.clone();
            jobs.push(tokio::spawn(QUEUE.run(move || { order.lock().unwrap().push(id); })));
            tokio::task::yield_now().await;
        }
        release.store(true, Ordering::SeqCst);
        for job in jobs { job.await.unwrap().unwrap(); }
        // Several workers may finish concurrently, but the newest request must
        // start in the first wave, rather than after all 31 obsolete requests.
        assert!(order.lock().unwrap().iter().position(|id| *id == 31).unwrap() < 8);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn thumbnail_load_leaves_executor_responsive_and_bounds_workers() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(AtomicBool::new(false));
        let mut jobs = Vec::new();
        for _ in 0..12 {
            let (active, peak, release) = (active.clone(), peak.clone(), release.clone());
            jobs.push(tokio::spawn(thumbnail_work(move || {
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                // A deadline keeps a regression from hanging the test process.
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                while !release.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(5));
                }
                active.fetch_sub(1, Ordering::SeqCst);
            })));
        }
        let responsive = tokio::time::timeout(Duration::from_secs(1), async {
            while active.load(Ordering::SeqCst) != 4 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }).await.is_ok();
        release.store(true, Ordering::SeqCst);
        for job in jobs { job.await.unwrap().unwrap(); }
        assert!(responsive, "thumbnail work blocked the async executor");
        assert_eq!(peak.load(Ordering::SeqCst), 4);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cache_hits_do_not_wait_for_saturated_decoders() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(AtomicBool::new(false));
        let mut jobs = Vec::new();
        for _ in 0..24 {
            let (active, peak, release) = (active.clone(), peak.clone(), release.clone());
            jobs.push(tokio::spawn(thumbnail_decode_work(move || {
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                while !release.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(5));
                }
                active.fetch_sub(1, Ordering::SeqCst);
            })));
        }
        let responsive = tokio::time::timeout(Duration::from_secs(1), async {
            while active.load(Ordering::SeqCst) != 8 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            assert_eq!(thumbnail_work(|| 42).await.unwrap(), 42);
        }).await.is_ok();
        release.store(true, Ordering::SeqCst);
        for job in jobs { job.await.unwrap().unwrap(); }
        assert!(responsive, "cache lookup queued behind unrelated decodes");
        assert_eq!(peak.load(Ordering::SeqCst), 8);
    }
}
