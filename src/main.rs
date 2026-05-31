#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod griddb;
mod scanner;
mod steam;
mod ui;

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let config = config::load()?;
    let app_log = ui::log::AppLog::new();
    init_logging(app_log.clone());

    // Multi-threaded tokio runtime for async HTTP/API operations.
    // The GUI stays on the main thread.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let _guard = rt.enter();

    ui::run(app_log, config)?;

    Ok(())
}

fn init_logging(app_log: ui::log::AppLog) {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
    use ui::log::AppLogLayer;

    let filter = EnvFilter::new("info");

    let stderr_layer = cfg!(debug_assertions).then(|| {
        tracing_subscriber::fmt::layer().with_writer(std::io::stderr)
    });

    tracing_subscriber::registry()
        .with(filter)
        .with(AppLogLayer::new(app_log))
        .with(stderr_layer)
        .init();
}
