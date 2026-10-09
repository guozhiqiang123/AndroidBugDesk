pub mod adb;
pub mod commands;
pub mod evidence;
pub mod model;
pub mod store;
pub mod submission;
#[cfg(test)]
mod tests;
pub mod validation;
pub mod yunxiao;
pub fn run() {
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
            commands::set_auth,
            commands::clear_auth,
            commands::load_metadata,
            commands::import_profile,
            commands::new_draft,
            commands::save_draft,
            commands::validate_draft,
            commands::choose_adb,
            commands::list_devices,
            commands::start_session,
            commands::stop_session,
            commands::mark_problem,
            commands::pin_logs,
            commands::import_attachment,
            commands::confirm_attachment,
            commands::remove_attachment,
            commands::read_log,
            commands::export_evidence,
            commands::submit_bug,
            commands::reconcile_submission,
            commands::copy_receipt,
            commands::open_receipt
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
                if let Some(t) = state.token.lock().unwrap().as_mut() {
                    use zeroize::Zeroize;
                    t.zeroize();
                };
            }
        });
}
