// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

/**
 * Main entry point for the Tauri application.
 * project: Lap
 * author:  julyx10
 * date:    2024-08-08
 */
use tauri::Manager;

#[cfg(all(feature = "vm-rig", feature = "gpu-providers"))]
compile_error!("vm-rig excludes GPU providers; use --no-default-features --features vm-rig");

pub mod t_advanced_ai;
pub mod t_ai;
pub mod t_ai_png;
pub mod t_apple_sidecar;
pub mod t_cluster;
pub mod t_cmds;
pub mod t_common;
pub mod t_config;
pub mod t_dedup;
pub mod t_embedded_jpeg;
pub mod t_enhance;
pub mod t_face;
pub mod t_face_suggest;
pub mod t_folder_rules;
pub mod t_heif;
pub mod t_http;
pub mod t_image;
pub mod t_edit_recipe;
pub mod t_curves;
mod t_edit_tools;
pub mod t_jpeg;
pub mod t_jxl;
pub mod t_lens;
pub mod t_libraw;
pub mod t_menu;
pub mod t_migration;
pub mod t_tag_groups;
pub mod t_motion_photo;
pub mod t_pasteboard;
pub mod t_picasa;
pub mod t_protocol;
pub mod t_similar;
pub mod t_sqlite;
pub mod t_storage;
pub mod t_thumb_cache;
pub mod t_face_thumb;
pub mod t_utils;
pub mod t_video;
pub mod t_xmp;

pub use t_cmds::merge_person;

/// The main function is the entry point for the Tauri application.
#[tokio::main]
pub async fn run() {
    std::panic::set_hook(Box::new(|panic_info| {
        eprintln!("Unhandled panic: {}", panic_info);
    }));

    let builder = tauri::Builder::default();
    let builder = t_protocol::register_protocols(builder);

    let run_result = builder
        .plugin(tauri_plugin_window_state::Builder::default().build()) // macOS: ~/Library/Application Support/{APP_NAME}/window-state.json
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .manage(t_video::VideoManager::default())
        .manage(t_ai::AiState(std::sync::Mutex::new(t_ai::AiEngine::new())))
        .manage(t_face::FaceState(std::sync::Arc::new(
            std::sync::Mutex::new(t_face::FaceEngine::new()),
        )))
        .manage(t_cmds::IndexCancellation(std::sync::Arc::new(
            std::sync::Mutex::new(std::collections::HashMap::new()),
        )))
        .manage(t_cmds::ImportCancellation(std::sync::Arc::new(std::sync::Mutex::new(
            t_cmds::ImportState::default(),
        ))))
        .manage(t_face::FaceIndexCancellation(std::sync::Arc::new(
            std::sync::Mutex::new(false),
        )))
        .manage(t_face::FaceIndexingStatus(std::sync::Arc::new(
            std::sync::Mutex::new(false),
        )))
        .manage(t_face::FaceIndexProgressState(std::sync::Arc::new(
            std::sync::Mutex::new(t_face::FaceIndexProgress {
                current: 0,
                total: 0,
                faces_found: 0,
                phase: "indexing".to_string(),
            }),
        )))
        .manage(t_dedup::DedupState::default())
        .manage(t_similar::SimilarState::default())
        .setup(|_app| {
            t_video::init_ffmpeg_path(&_app.handle());
            t_config::set_app_identifier(&_app.config().identifier);
            let background_enabled = !cfg!(feature = "vm-rig") && t_config::load_app_config()
                .map(|config| config.process_library_in_background)
                .unwrap_or(false);
            t_advanced_ai::set_background_enabled(background_enabled);
            if background_enabled {
                t_advanced_ai::start_ai_worker(_app.handle().clone());
            }
            t_menu::install_app_menu(&_app.handle())?;

            #[cfg(windows)]
            if let Some(window) = _app.get_webview_window("main") {
                if let Err(error) = t_utils::set_windows_taskbar_icon(&window) {
                    eprintln!("Failed to set taskbar icon: {error}");
                }
            }

            #[cfg(not(target_os = "macos"))]
            if let Some(window) = _app.get_webview_window("main") {
                if let Err(e) = window.set_decorations(false) {
                    eprintln!("Failed to disable main window decorations: {}", e);
                }
            }

            // tauri.windows.conf.json sets zoomHotkeysEnabled=true so wry sets
            // both IsZoomControlEnabled and IsPinchZoomEnabled to true at WebView
            // creation. That combination is what allows Chromium to synthesize
            // wheel+ctrlKey events for touchpad pinch. Touchscreen pinch is still
            // handled by our pointer-event logic (with `touch-action: none`).

            // Create the database on startup
            if let Err(e) = t_sqlite::create_db() {
                eprintln!("Failed to initialize database: {}", e);
            }

            // Initialize video HTTP server for Linux
            #[cfg(target_os = "linux")]
            t_http::init_video_http_server();

            // Cleanup video cache
            t_video::init_video_cache(&_app.handle());

            if !t_sqlite::is_database_corrupted() {
                if let Err(e) = t_utils::restore_album_scopes(&_app.handle()) {
                    eprintln!("Failed to restore asset scopes: {}", e);
                }
            }

            // Initialize AI Engine only when background processing is enabled.
            if background_enabled {
                let app_handle_bg = _app.handle().clone();
                std::thread::spawn(move || {
                if !t_advanced_ai::background_enabled() {
                    return;
                }
                let ai_state = app_handle_bg.state::<t_ai::AiState>();
                let mut ai_engine = ai_state.0.lock().unwrap();
                match ai_engine.load_models(&app_handle_bg) {
                    Ok(_) => println!("AI Engine started successfully"),
                    Err(e) => {
                        eprintln!("Failed to start AI Engine: {}", e);
                        #[cfg(target_os = "windows")]
                        {
                            let arch_key = if cfg!(target_arch = "aarch64") {
                                r"HKLM\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\ARM64"
                            } else {
                                r"HKLM\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\X64"
                            };
                            let result = std::process::Command::new("reg")
                                .args(["query", arch_key, "/v", "Installed"])
                                .stdout(std::process::Stdio::null())
                                .status();
                            let installed = result.is_ok() && result.unwrap().success();
                            if !installed {
                                let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
                                let url = format!("https://aka.ms/vs/17/release/vc_redist.{}.exe", arch);
                                let _ = std::process::Command::new("powershell")
                                    .args(["-NoProfile", "-Command", &format!(
                                        r#"$wsh = New-Object -ComObject Wscript.Shell; $wsh.Popup('LapCasa requires the Microsoft Visual C++ Redistributable.`n`nA download page will open in your browser.`nPlease install it, then restart LapCasa.', 0, 'LapCasa - Missing Dependency', 0x30); Start-Process '{}'"#,
                                        url
                                    )])
                                    .stdout(std::process::Stdio::null())
                                    .stderr(std::process::Stdio::null())
                                    .status();
                                std::process::exit(1);
                            }
                        }
                    }
                }
                if !t_advanced_ai::background_enabled() {
                    ai_engine.unload_models();
                }
            });
            }

            if !t_sqlite::is_database_corrupted() {
                t_utils::start_folder_mtime_sync(_app.handle().clone());
            }

            // Open devtools in development mode
            // #[cfg(debug_assertions)] // only include this block in debug builds
            // {
            //     let window = app.get_webview_window("main").unwrap();
            //     window.open_devtools();
            // }

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }

            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Prevent the default close so we can decide what to do per platform.
                api.prevent_close();

                // macOS convention: closing the window hides it; the app stays
                // alive and can be reopened via the Dock icon (handled by
                // RunEvent::Reopen). Real quit keeps using the native Quit menu.
                #[cfg(target_os = "macos")]
                {
                    let _ = window.hide();
                }

                #[cfg(not(target_os = "macos"))]
                {
                    let app_handle = window.app_handle();

                    // Close every other window first (image viewer, settings, ...),
                    // then exit the process.
                    let windows = app_handle.webview_windows();
                    for (_, other_window) in windows {
                        if other_window.label() != "main" {
                            if let Err(err) = other_window.close() {
                                eprintln!("Failed to close window: {}", err);
                            }
                        }
                    }

                    app_handle.exit(0);
                }
            }
        })
        .on_menu_event(|app, event| {
            t_menu::handle_menu_event(app, event);
        })
        .invoke_handler(tauri::generate_handler![
            t_picasa::picasa_import_defaults,
            t_picasa::picasa_import_preview,
            t_picasa::picasa_import_apply,
            t_picasa::picasa_import_open_report,
            // library
            t_cmds::get_app_config,
            t_cmds::set_background_processing,
            t_cmds::get_ai_progress,
            t_cmds::get_supported_format_extensions,
            t_cmds::get_ffmpeg_backed_image_extensions,
            t_cmds::check_gstreamer_available,
            t_cmds::set_last_selected_item_index,
            t_cmds::get_db_storage_dir,
            t_cmds::is_using_custom_db_storage,
            t_cmds::change_db_storage_dir,
            t_cmds::reset_db_storage_dir,
            t_cmds::add_library,
            t_cmds::edit_library,
            t_cmds::remove_library,
            t_cmds::hide_library,
            t_cmds::reorder_libraries,
            t_cmds::switch_library,
            t_cmds::is_database_corrupted,
            t_cmds::get_library_info,
            t_cmds::save_library_state,
            t_cmds::get_library_state,
            t_cmds::get_current_library_state,
            // album
            t_cmds::get_all_albums,
            t_cmds::get_all_album_folders,
            t_cmds::generate_directory_thumbnails,
            t_cmds::get_album,
            t_cmds::check_album_accessibility,
            t_cmds::recount_album,
            t_cmds::get_album_visible_counts,
            t_cmds::add_album,
            t_cmds::edit_album,
            t_cmds::remove_album,
            t_cmds::reorder_albums,
            t_cmds::set_album_cover,
            t_cmds::index_album,
            t_cmds::cancel_indexing,
            t_cmds::get_index_recovery_info,
            t_cmds::clear_index_recovery_info,
            // folder
            t_cmds::select_folder,
            t_cmds::fetch_folder,
            t_cmds::count_folder,
            t_cmds::list_album_subfolders,
            t_folder_rules::list_folder_tree,
            t_folder_rules::list_drives,
            t_folder_rules::add_album_root,
            t_folder_rules::get_folder_modes,
            t_folder_rules::get_folder_rules,
            t_folder_rules::set_folder_rule,
            t_cmds::create_folder,
            t_cmds::rename_folder,
            t_cmds::move_folder,
            t_cmds::move_folder_outside_library,
            t_cmds::copy_folder,
            t_cmds::delete_folder,
            t_cmds::delete_folder_permanently,
            t_cmds::reveal_path,
            t_cmds::open_external_url,
            t_cmds::set_desktop_wallpaper,
            t_cmds::get_external_app_display_name,
            t_cmds::open_file_with_app,
            t_cmds::open_files_with_app,
            // file query
            t_cmds::get_query_count_and_sum,
            t_cmds::get_query_time_line,
            t_cmds::get_query_files,
            t_cmds::search_files,
            t_cmds::get_grouped_query_rows,
            t_cmds::get_folder_sections,
            t_cmds::get_group_file_ids,
            t_cmds::get_grouped_file_position,
            t_cmds::get_query_file_ids,
            t_cmds::get_library_visible_counts,
            t_cmds::get_query_file_position,
            // smart album
            t_cmds::get_smart_query_count_and_sum,
            t_cmds::get_smart_query_time_line,
            t_cmds::get_smart_query_files,
            t_cmds::get_smart_grouped_query_rows,
            t_cmds::get_smart_group_file_ids,
            t_cmds::get_smart_query_file_ids,
            t_cmds::get_smart_query_file_position,
            // collection
            t_cmds::list_collections,
            t_cmds::get_collection_counts,
            t_cmds::create_collection,
            t_cmds::rename_collection,
            t_cmds::delete_collection,
            t_cmds::reorder_collections,
            t_cmds::add_files_to_collection,
            t_cmds::remove_files_from_collection,
            t_cmds::get_collection_selection_counts,
            t_cmds::clear_collection,
            t_cmds::get_collection_file_ids,
            t_cmds::get_file_collections,
            t_cmds::get_collection_count_and_sum,
            t_cmds::get_collection_files,
            t_cmds::get_collection_grouped_query_rows,
            t_cmds::get_collection_group_file_ids,
            t_cmds::get_collection_query_file_ids,
            // folder file query
            t_cmds::get_files_by_ids,
            t_cmds::get_selection_summary,
            t_cmds::get_folder_files,
            t_cmds::sync_album_folder_mtimes,
            t_cmds::refresh_album_subfolders,
            t_cmds::is_directory_accessible,
            t_cmds::is_directory_empty,
            t_cmds::get_folder_thumb_count,
            // file operations
            t_cmds::edit_image,
            t_cmds::save_image_recipe,
            t_cmds::restore_image_recipe,
            t_cmds::get_rendered_faces,
            t_cmds::copy_edited_image,
            t_cmds::copy_images,
            t_cmds::rename_file,
            t_cmds::move_file,
            t_cmds::move_file_outside_library,
            t_cmds::copy_file,
            t_cmds::import_file,
            t_cmds::import_and_organize,
            t_cmds::cancel_import_and_organize,
            t_cmds::import_url,
            t_cmds::import_from_drag,
            t_cmds::get_drag_payload,
            t_cmds::import_file_bytes,
            t_cmds::has_importable_clipboard,
            t_cmds::import_clipboard,
            t_cmds::delete_file,
            t_cmds::delete_file_permanently,
            t_cmds::delete_db_file,
            t_cmds::batch_delete_files,
            // file metadata
            t_cmds::edit_file_comment,
            t_cmds::clean_unused_thumbnail_cache,
            t_cmds::get_file_thumb,
            t_cmds::get_file_thumb_by_id,
            t_cmds::get_file_thumbs,
            t_cmds::get_file_info,
            t_cmds::update_file_info,
            t_cmds::refresh_selected_file_info,
            t_cmds::prepare_motion_photo_video,
            t_cmds::add_file_to_db,
            t_cmds::check_file_exists,
            t_cmds::set_file_rotate,
            t_cmds::get_file_has_tags,
            // favorite
            t_cmds::get_favorite_folders,
            t_cmds::get_folder_favorite,
            t_cmds::set_folder_favorite,
            t_cmds::get_folder_search_excluded,
            t_cmds::set_folder_search_excluded,
            t_cmds::get_folder_faces_excluded,
            t_cmds::set_folder_faces_excluded,
            t_cmds::get_folder_ai_excluded,
            t_cmds::set_folder_ai_excluded,
            t_cmds::set_file_favorite,
            t_cmds::set_file_rating,
            t_cmds::set_file_culling_flag,
            t_cmds::toggle_auto_enhance,
            t_cmds::batch_update_file_metadata,
            // tag
            t_cmds::get_tag_group_name,
            t_cmds::get_tag_groups,
            t_cmds::save_tag_group,
            t_cmds::reorder_tag_groups,
            t_cmds::delete_tag_group,
            t_cmds::move_tags_to_group,
            t_cmds::get_all_tags,
            t_cmds::get_tag_counts,
            t_cmds::get_tag_name,
            t_cmds::create_tag,
            t_cmds::rename_tag,
            t_cmds::delete_tag,
            t_cmds::get_tags_for_file,
            t_cmds::add_tag_to_file,
            t_cmds::remove_tag_from_file,
            t_cmds::get_tag_selection_counts,
            t_cmds::apply_tags_to_files,
            // calendar
            t_cmds::get_taken_dates,
            // camera
            t_cmds::get_camera_info,
            t_cmds::get_lens_info,
            // location
            t_cmds::get_location_info,
            t_cmds::get_gps_map_points,
            // settings
            t_cmds::get_package_info,
            t_cmds::get_build_time,
            t_cmds::get_storage_file_info,
            // ai
            t_cmds::check_ai_status,
            t_cmds::get_image_search_model_status,
            t_cmds::set_image_search_model,
            t_cmds::download_multilingual_image_search_model,
            t_cmds::cancel_multilingual_image_search_model_download,
            t_cmds::generate_embedding,
            t_cmds::search_similar_images,
            // person (face recognition)
            t_cmds::index_faces,
            t_cmds::cancel_face_index,
            t_cmds::reset_faces,
            t_cmds::is_face_indexing,
            t_cmds::get_face_stats,
            t_cmds::get_persons,
            t_cmds::get_persons_page,
            t_cmds::rename_person,
            t_cmds::delete_person,
            t_cmds::merge_person,
            t_cmds::get_faces_for_file,
            t_cmds::get_person_face_boxes,
            t_cmds::compute_face_suggestions,
            t_cmds::confirm_face_suggestions,
            t_cmds::reject_face_suggestions,
            t_cmds::undo_reject_face_suggestions,
            t_cmds::exclude_faces_from_person,
            t_cmds::refresh_person_suggestions,
            t_cmds::confirm_ignored_faces,
            t_cmds::get_person_ignored_faces,
            t_cmds::restore_globally_ignored_faces,
            t_cmds::get_person_thumbnail,
            // dedup
            t_cmds::dedup_start_scan,
            t_cmds::dedup_get_scan_status,
            t_cmds::dedup_cancel_scan,
            t_cmds::dedup_list_groups,
            t_cmds::dedup_get_overview,
            t_cmds::dedup_set_keep,
            t_cmds::dedup_delete,
            t_cmds::similar_start_scan,
            t_cmds::similar_get_scan_status,
            t_cmds::similar_cancel_scan,
            t_cmds::similar_get_eligible_count,
            t_cmds::similar_list_groups,
            t_cmds::similar_get_overview,
            t_cmds::similar_get_group,
            t_cmds::similar_set_keep,
            t_cmds::similar_has_scan,
            // video
            t_video::prepare_video,
            t_video::cancel_video_prepare,
            t_video::clear_video_cache,
            // backup / restore
            t_cmds::get_db_storage_info,
            t_cmds::backup_databases,
            t_cmds::parse_backup_file,
            t_cmds::restore_databases,
        ])
        .build(tauri::generate_context!());

    match run_result {
        Ok(app) => {
            app.run(move |_app_handle, event| match event {
                tauri::RunEvent::ExitRequested { .. } => {
                    if let Err(error) = t_thumb_cache::shutdown() { eprintln!("{error}"); }
                }
                // macOS: clicking the Dock icon of a running app reopens it.
                // When the main window is hidden (closed-to-hide), show it again.
                #[cfg(target_os = "macos")]
                tauri::RunEvent::Reopen { .. } => {
                    if let Some(window) = _app_handle.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                _ => {}
            });
        }
        Err(err) => {
            eprintln!("Error while running application: {}", err);
        }
    }
}
