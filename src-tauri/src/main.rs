#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    #[cfg(windows)]
    {
        let mut args =
            std::env::var_os("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap_or_default();
        if !args
            .to_string_lossy()
            .split_whitespace()
            .any(|arg| arg == "--enable-smooth-scrolling")
        {
            args.push(" --enable-smooth-scrolling");
            // Set this before Tokio or WebView2 starts any threads. All windows
            // must inherit the same browser arguments and profile environment.
            unsafe { std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", args) };
        }
    }
    lap_casa::run();
}
