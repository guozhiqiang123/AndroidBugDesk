pub mod adb;
pub mod capture;
pub mod commands;
pub mod diagnostics;
pub mod drafts;
pub mod evidence;
pub mod history;
#[cfg(test)]
mod inspection_tests;
pub mod login_memory;
pub mod model;
pub mod oauth;
pub mod post_capture;
pub mod project_settings;
pub mod store;
pub mod submission;
pub mod submission_fields;
#[cfg(test)]
mod tests;
pub mod updates;
pub mod validation;
pub mod workitem_links;
pub mod workspace;
pub mod yunxiao;
pub fn run() {
    diagnostics::install_panic_hook();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            use tauri::Manager;
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .setup(commands::setup)
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::check_auth,
            commands::login,
            oauth::oauth_login,
            oauth::cancel_oauth_login,
            commands::list_organizations,
            commands::list_projects,
            commands::select_project,
            project_settings::load_project_settings,
            project_settings::save_project_settings,
            commands::load_selection_memory,
            commands::remember_selections,
            commands::list_remote_history,
            commands::set_auth,
            commands::clear_auth,
            commands::load_metadata,
            commands::import_profile,
            commands::new_draft,
            drafts::delete_draft,
            drafts::delete_drafts,
            commands::save_draft,
            commands::validate_draft,
            commands::choose_adb,
            commands::list_devices,
            commands::inspect_device,
            commands::start_session,
            commands::stop_session,
            commands::mark_problem,
            commands::pin_logs,
            commands::capture_bugreport,
            post_capture::start_post_capture,
            post_capture::post_capture_status,
            commands::redact_log,
            commands::import_attachment,
            commands::preview_attachment,
            commands::save_attachment_coverage,
            commands::remove_attachment,
            commands::remove_all_attachments,
            commands::export_evidence,
            commands::submit_bug,
            commands::reconcile_submission,
            commands::copy_receipt,
            commands::copy_history_link,
            commands::open_history_item,
            commands::open_receipt,
            updates::check_for_updates,
            updates::install_update,
            updates::open_release_page
        ])
        .build(tauri::generate_context!())
        .expect("desktop initialization failed")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                use tauri::Manager;
                let state = app.state::<commands::AppState>();
                if let Some(c) = state.collector.lock().unwrap().as_ref() {
                    let _ = c.stop.send(true);
                }
                state.stop_page_observer();
                diagnostics::event("app", "exit requested");
                if let Some(t) = state.token.lock().unwrap().as_mut() {
                    use zeroize::Zeroize;
                    t.zeroize();
                };
            }
        });
}
