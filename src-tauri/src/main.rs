#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    if std::env::args().any(|argument| argument == "--mcp-bridge") {
        if let Err(error) = android_bug_desk_lib::agent_mcp::run() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    android_bug_desk_lib::run();
}
