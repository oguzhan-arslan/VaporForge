#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod griddb;
mod scanner;
mod steam;
mod ui;

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let mut config = config::load()?;
    let app_log = ui::log::AppLog::new();
    init_logging(app_log.clone());

    validate_steam_user(&mut config);

    // Multi-threaded tokio runtime for async HTTP/API operations.
    // The GUI stays on the main thread.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let _guard = rt.enter();

    ui::run(app_log, config)?;

    Ok(())
}

fn validate_steam_user(config: &mut config::AppConfig) {
    if config.steam.user_id.is_empty() {
        return;
    }
    let valid = steam::paths::find_steam_dir()
        .zip(config.steam.user_id.parse::<u64>().ok())
        .map(|(dir, uid)| steam::paths::user_dir_exists(&dir, uid))
        .unwrap_or(false);
    if !valid {
        tracing::warn!(
            "Configured Steam user ID '{}' not found on disk; clearing override.",
            config.steam.user_id
        );
        config.steam.user_id.clear();
        let _ = config::save(config);
    }
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
