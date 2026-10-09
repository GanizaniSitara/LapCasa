use tauri::{Builder, Wry};

use crate::{t_image, t_sqlite};

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
            let app_handle = _ctx.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                let has_auto_enhance = crate::t_enhance::is_file_auto_enhanced(file_id);
                if let Some(data) = crate::t_thumb_cache::get(file_id) {
                    let final_data = if has_auto_enhance {
                        crate::t_enhance::apply_auto_enhance_to_bytes_for_file(&data, Some(file_id)).unwrap_or(data)
                    } else {
                        data
                    };
                    responder.respond(image_response(final_data));
                    return;
                }

                let response = match t_sqlite::AThumb::fetch_raw_for_library(file_id, &library_id) {
                    Ok(Some(data)) => {
                        let _ = crate::t_thumb_cache::put(file_id, &data);
                        let final_data = if has_auto_enhance {
                            crate::t_enhance::apply_auto_enhance_to_bytes_for_file(&data, Some(file_id)).unwrap_or(data)
                        } else {
                            data
                        };
                        image_response(final_data)
                    }
                    _ => {
                        if let Ok(Some(file)) = t_sqlite::AFile::get_file_info(file_id) {
                            if let Some(file_path) = file.file_path.clone() {
                                let file_type = file.file_type.unwrap_or(0);
                                let orientation = file.e_orientation.unwrap_or(1) as i32;
                                let album_id = file.album_id.unwrap_or(0);
                                let thumbnail_size = 200;
                                t_sqlite::AThumb::schedule_background_generation_for_library(
                                    app_handle,
                                    file_id,
                                    file_path,
                                    file_type,
                                    orientation,
                                    thumbnail_size,
                                    false,
                                    album_id,
                                    false,
                                    None,
                                );
                            }
                        }
                        text_response(http::StatusCode::NOT_FOUND, "thumbnail not found")
                    }
                };
                responder.respond(response);
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
                // A prepared crop needs no SQL, decoder, or async executor slot.
                // Deliver it before potentially blocking library queries run.
                if let Ok(Some(data)) = crate::t_face_thumb::cached(&key) {
                    responder.respond(image_response(data));
                    return;
                }
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

            let app_handle = _ctx.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                let has_auto_enhance = crate::t_enhance::is_file_auto_enhanced(file_id);
                if let Some(data) = crate::t_thumb_cache::get(file_id) {
                    let final_data = if has_auto_enhance {
                        crate::t_enhance::apply_auto_enhance_to_bytes_for_file(&data, Some(file_id)).unwrap_or(data)
                    } else {
                        data
                    };
                    responder.respond(image_response(final_data));
                    return;
                }

                if let Ok(Some(data)) = t_sqlite::AThumb::fetch_raw_for_library(file_id, &library_id) {
                    let _ = crate::t_thumb_cache::put(file_id, &data);
                    let final_data = if has_auto_enhance {
                        crate::t_enhance::apply_auto_enhance_to_bytes_for_file(&data, Some(file_id)).unwrap_or(data)
                    } else {
                        data
                    };
                    responder.respond(image_response(final_data));
                    return;
                }

                if let Ok(Some(file)) = t_sqlite::AFile::get_file_info(file_id) {
                    if let Some(file_path) = file.file_path {
                        let file_type = file.file_type.unwrap_or(0);
                        let orientation = file.e_orientation.unwrap_or(1) as i32;
                        let album_id = file.album_id.unwrap_or(0);
                        let thumbnail_size = 200;
                        t_sqlite::AThumb::schedule_background_generation_for_library(
                            app_handle,
                            file_id,
                            file_path.clone(),
                            file_type,
                            orientation,
                            thumbnail_size,
                            false,
                            album_id,
                            false,
                            None,
                        );

                        if let Ok(data) = t_image::get_file_image_bytes_cached(&file_path, false).await {
                            let final_data = if has_auto_enhance {
                                crate::t_enhance::apply_auto_enhance_to_bytes_for_file(&data, Some(file_id)).unwrap_or(data)
                            } else {
                                data
                            };
                            responder.respond(image_response(final_data));
                            return;
                        }
                    }
                }
                responder.respond(text_response(http::StatusCode::NOT_FOUND, "thumbnail not found"));
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

            let file = match t_sqlite::AFile::get_file_info(file_id) {
                Ok(Some(file)) => file,
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

            let original=request.uri().query().unwrap_or("").split('&').any(|p|p=="original=1");
            let has_auto_enhance = !original && file
                .edits
                .as_deref()
                .map(crate::t_enhance::is_auto_enhance_active)
                .unwrap_or(false);

            tauri::async_runtime::spawn(async move {
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
