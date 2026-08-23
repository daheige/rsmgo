use tauri::Manager;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! Welcome to rsmgo desktop.", name)
}

/// Open a native directory picker and return the selected directory's absolute
/// path. Uses the async dialog so the panel is presented on the main thread
/// (required by AppKit on macOS) without blocking the WebView.
#[tauri::command]
async fn pick_directory() -> Option<String> {
    rfd::AsyncFileDialog::new()
        .set_title("选择工作目录")
        .pick_folder()
        .await
        .map(|handle| handle.path().to_string_lossy().to_string())
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![greet, pick_directory])
        .setup(|app| {
            #[cfg(debug_assertions)]
            {
                let window = app.get_webview_window("main").unwrap();
                window.open_devtools();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running rsmgo desktop");
}
