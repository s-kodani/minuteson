//! The native predefined Quit action calls NSApplication.terminate directly.
//! Route the menu and Cmd+Q through Tauri's ExitRequested save/stop barrier.
use tauri::menu::{AboutMetadata, Menu, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu};

const GUARDED_QUIT: &str = "mon-guarded-quit";

pub(crate) fn install(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // Preserve Tauri's Edit, File, View, Window and Help menus and native roles.
    let menu = Menu::default(app.handle())?;
    let Some(MenuItemKind::Submenu(original)) = menu.items()?.into_iter().next() else {
        return Err("macOS application menu is missing".into());
    };
    let package = app.package_info();
    let config = app.config();
    let about = AboutMetadata {
        name: Some(package.name.clone()),
        version: Some(package.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config.bundle.publisher.clone().map(|p| vec![p]),
        ..Default::default()
    };
    let application = Submenu::with_items(
        app,
        original.text()?,
        true,
        &[
            &PredefinedMenuItem::about(app, None, Some(about))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, GUARDED_QUIT, "終了", true, Some("Cmd+Q"))?,
        ],
    )?;
    menu.remove(&original)?;
    menu.insert(&application, 0)?;
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        if event.id().as_ref() == GUARDED_QUIT {
            // This dispatches ExitRequested. Only finish_main_close sets the
            // ExitGuard after confirmed draft saving and safe worker shutdown.
            app.exit(0);
        }
    });
    Ok(())
}
