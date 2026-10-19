//! The tray icon (ADR-0056): "Open" and "Quit", a click to bring the window back. While it exists, closing the
//! window only hides it, so the deadline notifications keep coming.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::show;

pub struct TrayLabels {
    open: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

pub fn labels(language: &str) -> (&'static str, &'static str) {
    match language {
        "uk" => ("Відкрити", "Вийти"),
        _ => ("Open", "Quit"),
    }
}

impl TrayLabels {
    pub fn speak(&self, language: &str) {
        let (open, quit) = labels(language);
        let _ = self.open.set_text(open);
        let _ = self.quit.set_text(quit);
    }
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let (open_text, quit_text) = labels("en");
    let open = MenuItem::with_id(app, "open", open_text, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", quit_text, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    app.manage(TrayLabels { open, quit });
    Ok(())
}
