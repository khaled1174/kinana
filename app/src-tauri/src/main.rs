#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod bridge;
fn main() {
    let home = bridge::home().expect("HOME unavailable");
    let app = tauri::Builder::default()
        .manage(bridge::Desktop::new(&home))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                .on_navigation(|url| {
                    (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
                        || (matches!(url.scheme(), "http" | "https")
                            && url.host_str() == Some("tauri.localhost"))
                })
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bridge::bootstrap,
            bridge::answer_audit_rules_prompt,
            bridge::desktop_settings,
            bridge::vault_location_warnings,
            bridge::classification_guide,
            bridge::agent_skill_locations,
            bridge::preview_settings,
            bridge::start_settings,
            bridge::settings_status,
            bridge::cancel_settings,
            bridge::desktop_doctor,
            bridge::start_scan,
            bridge::commit_scan,
            bridge::scan_status,
            bridge::cancel_scan,
            bridge::projects,
            bridge::preview_project,
            bridge::start_project,
            bridge::project_status,
            bridge::cancel_project,
            bridge::catalog,
            bridge::connect_vault,
            bridge::skill_detail,
            bridge::pin_skill,
            bridge::category_skill,
            bridge::review_skill,
            bridge::pending_descriptions,
            bridge::preview_accept_descriptions,
            bridge::accept_descriptions,
            bridge::preview_classify,
            bridge::start_classify,
            bridge::classify_status,
            bridge::cancel_classify,
            bridge::undo_vault,
            bridge::undo_auto_approval,
            bridge::pi_models,
            bridge::reveal_skill_folder,
            bridge::vault_bundles,
            bridge::vault_groups,
            bridge::save_bundle,
            bridge::preview_import,
            bridge::prepare_import,
            bridge::import_status,
            bridge::cancel_import,
            bridge::commit_import,
            bridge::start_sources,
            bridge::sources_status,
            bridge::cancel_sources
        ]);
    #[cfg(feature = "e2e")]
    let app = app
        .plugin(tauri_plugin_wdio::init())
        .plugin(tauri_plugin_wdio_webdriver::init());
    app.run(tauri::generate_context!())
        .expect("could not run Kinana");
}
