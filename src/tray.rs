use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

#[derive(Clone, Debug)]
pub enum TrayEvent {
    MenuItem(tray_icon::menu::MenuId),
}

pub struct TrayHandle {
    _icon: TrayIcon,
    _watcher: std::thread::JoinHandle<()>,
    pub events: Arc<Mutex<VecDeque<TrayEvent>>>,
    pub run_now_id: tray_icon::menu::MenuId,
    pub show_hide_id: tray_icon::menu::MenuId,
    pub open_config_id: tray_icon::menu::MenuId,
    pub open_logs_id: tray_icon::menu::MenuId,
    pub quit_id: tray_icon::menu::MenuId,
}

fn make_icon() -> eyre::Result<tray_icon::Icon> {
    const SIZE: u32 = 32;
    // 32×32 solid steam-blue square
    let rgba: Vec<u8> = (0..SIZE * SIZE)
        .flat_map(|_| [0x1b_u8, 0x2e, 0x4a, 0xff])
        .collect();
    tray_icon::Icon::from_rgba(rgba, SIZE, SIZE)
        .map_err(|e| eyre::eyre!("tray icon: {e}"))
}

pub fn install(ctx: &egui::Context) -> eyre::Result<TrayHandle> {
    let icon = make_icon()?;

    let run_now = MenuItem::new("Run Now", true, None);
    let show_hide = MenuItem::new("Show / Hide", true, None);
    let open_config = MenuItem::new("Open Config", true, None);
    let open_logs = MenuItem::new("Open Logs Folder", true, None);
    let quit = MenuItem::new("Quit", true, None);

    let run_now_id = run_now.id().clone();
    let show_hide_id = show_hide.id().clone();
    let open_config_id = open_config.id().clone();
    let open_logs_id = open_logs.id().clone();
    let quit_id = quit.id().clone();

    let sep = PredefinedMenuItem::separator();
    let sep2 = PredefinedMenuItem::separator();

    let menu = Menu::new();
    menu.append(&run_now).map_err(|e| eyre::eyre!("{e}"))?;
    menu.append(&show_hide).map_err(|e| eyre::eyre!("{e}"))?;
    menu.append(&sep).map_err(|e| eyre::eyre!("{e}"))?;
    menu.append(&open_config).map_err(|e| eyre::eyre!("{e}"))?;
    menu.append(&open_logs).map_err(|e| eyre::eyre!("{e}"))?;
    menu.append(&sep2).map_err(|e| eyre::eyre!("{e}"))?;
    menu.append(&quit).map_err(|e| eyre::eyre!("{e}"))?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon(icon)
        .with_tooltip("VaporForge")
        .build()
        .map_err(|e| eyre::eyre!("tray build: {e}"))?;

    let events: Arc<Mutex<VecDeque<TrayEvent>>> = Arc::new(Mutex::new(VecDeque::new()));
    let events_clone = Arc::clone(&events);
    let ctx_clone = ctx.clone();

    // Background thread: drains the muda MenuEvent channel and wakes the GUI.
    // Needed so tray clicks are handled even when the window is hidden.
    let watcher = std::thread::spawn(move || {
        let rx = MenuEvent::receiver().clone();
        loop {
            while let Ok(event) = rx.try_recv() {
                events_clone.lock().unwrap().push_back(TrayEvent::MenuItem(event.id));
                ctx_clone.request_repaint();
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });

    Ok(TrayHandle {
        _icon: tray,
        _watcher: watcher,
        events,
        run_now_id,
        show_hide_id,
        open_config_id,
        open_logs_id,
        quit_id,
    })
}
