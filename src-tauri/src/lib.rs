mod agent_cli;
mod ai_qa;
mod ai_service;
mod content_extractor;
mod database;
mod feedback;
mod file_query;
mod file_space;
mod pdf_preview;
mod semantic_search;
mod version_comparison;
mod workspace;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{webview::PageLoadEvent, Manager, RunEvent, WindowEvent};

fn show_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let startup_ready = Arc::new(AtomicBool::new(false));
    let ready_for_page = Arc::clone(&startup_ready);
    let ready_for_instance = Arc::clone(&startup_ready);
    #[cfg(target_os = "macos")]
    let ready_for_reopen = Arc::clone(&startup_ready);

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(move |app, _, _| {
            if ready_for_instance.load(Ordering::Acquire) {
                show_main_window(app);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .register_uri_scheme_protocol("lumetrace-file-preview", |context, request| {
            let database = context.app_handle().state::<database::Database>();
            file_space::file_preview_response(database.inner(), request.uri().path())
        })
        .on_page_load(move |webview, payload| {
            if webview.label() != "main" || !matches!(payload.event(), PageLoadEvent::Finished) {
                return;
            }
            if !ready_for_page.swap(true, Ordering::AcqRel) {
                show_main_window(webview.app_handle());
            }
        })
        .setup(|app| {
            let (database, workspace_registry) = workspace::initialize(app.handle()).map_err(
                |error| -> Box<dyn std::error::Error> { std::io::Error::other(error).into() },
            )?;
            app.manage(database);
            app.manage(workspace_registry);
            app.manage(file_space::FileSpaceVersionNotificationQueue::default());
            app.manage(file_space::FileSpaceBackgroundRuntime::default());
            app.manage(ai_qa::FileSpaceAiRuntime::default());
            let app_data_directory =
                app.path()
                    .app_data_dir()
                    .map_err(|error| -> Box<dyn std::error::Error> {
                        std::io::Error::other(error).into()
                    })?;
            app.manage(semantic_search::SemanticSearchRuntime::new(
                &app_data_directory,
            ));
            file_space::start_file_space_watcher(app.handle().clone()).map_err(
                |error| -> Box<dyn std::error::Error> { std::io::Error::other(error).into() },
            )?;
            file_space::start_file_content_extractor(app.handle().clone()).map_err(
                |error| -> Box<dyn std::error::Error> { std::io::Error::other(error).into() },
            )?;
            semantic_search::start_semantic_indexer(app.handle().clone()).map_err(
                |error| -> Box<dyn std::error::Error> { std::io::Error::other(error).into() },
            )?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            feedback::open_feedback_channel,
            ai_qa::ask_file_space_ai,
            ai_qa::cancel_file_space_ai,
            ai_qa::get_file_space_ai_history,
            file_query::lookup_file_space_file_names,
            file_query::get_file_space_file_version_summary,
            ai_service::get_ai_service_settings,
            ai_service::check_cloud_ai_connection,
            ai_service::check_local_llm_connection,
            ai_service::save_cloud_ai_settings,
            ai_service::save_local_llm_settings,
            agent_cli::check_agent_clis,
            agent_cli::check_agent_cli_status,
            agent_cli::check_agent_cli,
            agent_cli::get_agent_cli_settings,
            agent_cli::save_agent_cli_settings,
            workspace::get_file_space_workspaces,
            workspace::switch_file_space_workspace,
            workspace::create_file_space_workspace,
            workspace::rename_file_space_workspace,
            workspace::remove_file_space_workspace,
            file_space::get_file_space_snapshot,
            file_space::list_file_space_files,
            file_space::scan_file_space_external_changes,
            file_space::drain_file_space_version_notifications,
            file_space::get_file_space_background_status,
            file_space::set_file_space_background_paused,
            file_space::retry_file_space_background_failures,
            file_space::search_file_space_files,
            file_space::set_file_space_file_tags,
            file_space::reorder_file_space_files,
            file_space::configure_file_space_root,
            file_space::export_file_space_backup,
            file_space::inspect_file_space_backup,
            file_space::restore_file_space_backup,
            file_space::import_existing_file_space_root,
            file_space::create_file_space_folder,
            file_space::create_file_space_text_file,
            file_space::rename_file_space_folder,
            file_space::move_file_space_folder,
            file_space::delete_file_space_folder,
            file_space::rename_file_space_file,
            file_space::move_file_space_file,
            file_space::move_file_space_files,
            file_space::delete_file_space_file,
            file_space::restore_file_space_trash_entry,
            file_space::empty_file_space_trash,
            file_space::purge_expired_file_space_trash,
            file_space::reveal_file_space_file,
            file_space::copy_file_space_file,
            file_space::copy_file_space_file_path,
            file_space::start_file_space_drag_out,
            file_space::open_file_space_file,
            file_space::choose_application_for_file_space_file,
            file_space::get_task_file_timeline,
            file_space::set_current_task_file_version,
            file_space::read_task_file_version,
            version_comparison::get_task_file_text_diff,
            file_space::read_file_space_markdown,
            file_space::read_file_space_text,
            file_space::save_file_space_markdown,
            pdf_preview::read_file_space_pdf,
            semantic_search::get_semantic_search_status,
            semantic_search::install_semantic_search_model,
            semantic_search::cancel_semantic_search_model_download,
            semantic_search::remove_semantic_search_model,
            semantic_search::retry_semantic_search_index,
            file_space::inspect_file_space_import_conflicts,
            file_space::import_file_space_files,
            file_space::inspect_file_space_dropped_conflicts,
            file_space::import_file_space_dropped_files,
            file_space::cancel_file_space_import
        ])
        .build(tauri::generate_context!())
        .expect("error while building Lume Trace");
    app.run(move |app, event| {
        #[cfg(target_os = "macos")]
        if let RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } = &event
        {
            if label == "main" {
                api.prevent_close();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
                return;
            }
        }

        #[cfg(target_os = "macos")]
        if matches!(event, RunEvent::Reopen { .. }) && ready_for_reopen.load(Ordering::Acquire) {
            show_main_window(app);
        }
    });
}
