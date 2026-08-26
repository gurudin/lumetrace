mod database;
mod file_space;
mod pdf_preview;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .register_uri_scheme_protocol("lumetrace-file-preview", |context, request| {
            let database = context.app_handle().state::<database::Database>();
            file_space::file_preview_response(database.inner(), request.uri().path())
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let database = database::initialize(app.handle()).map_err(
                |error| -> Box<dyn std::error::Error> { std::io::Error::other(error).into() },
            )?;
            app.manage(database);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            file_space::get_file_space_snapshot,
            file_space::search_file_space_files,
            file_space::set_file_space_file_tags,
            file_space::reorder_file_space_files,
            file_space::configure_file_space_root,
            file_space::create_file_space_folder,
            file_space::rename_file_space_folder,
            file_space::move_file_space_folder,
            file_space::delete_file_space_folder,
            file_space::rename_file_space_file,
            file_space::move_file_space_file,
            file_space::delete_file_space_file,
            file_space::reveal_file_space_file,
            file_space::copy_file_space_file,
            file_space::copy_file_space_file_path,
            file_space::start_file_space_drag_out,
            file_space::open_file_space_file,
            file_space::choose_application_for_file_space_file,
            file_space::get_task_file_timeline,
            file_space::set_current_task_file_version,
            file_space::read_task_file_version,
            file_space::read_file_space_markdown,
            file_space::read_file_space_text,
            file_space::save_file_space_markdown,
            pdf_preview::read_file_space_pdf,
            file_space::import_file_space_files,
            file_space::import_file_space_dropped_files,
            file_space::cancel_file_space_import
        ])
        .build(tauri::generate_context!())
        .expect("error while building LumeTrace");
    app.run(|_, _| {});
}
