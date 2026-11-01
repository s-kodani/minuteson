//! The native predefined Quit action calls NSApplication.terminate directly.
//! Route menu, Cmd+Q and Dock termination through the same save/stop barrier.
use objc2::{
    runtime::{AnyClass, AnyObject, ClassBuilder, Sel},
    sel, MainThreadMarker,
};
use objc2_app_kit::{NSApplication, NSApplicationTerminateReply};
use std::{ffi::CStr, sync::OnceLock};
use tauri::menu::{AboutMetadata, Menu, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu};

const GUARDED_QUIT: &str = "mon-guarded-quit";
static QUIT_APP: OnceLock<tauri::AppHandle> = OnceLock::new();

extern "C" fn application_should_terminate(
    _: &AnyObject,
    _: Sel,
    _: *const NSApplication,
) -> NSApplicationTerminateReply {
    if let Some(app) = QUIT_APP.get() {
        // exit queues a runtime event; never wait for the WebView or workers
        // inside AppKit's synchronous termination callback. Cancel the native
        // path so it cannot bypass ExitRequested/finish_main_close.
        app.exit(0);
    }
    NSApplicationTerminateReply::TerminateCancel
}

fn guarded_delegate_class(
    name: &CStr,
    parent: &'static AnyClass,
) -> Result<&'static AnyClass, &'static str> {
    // A future Tao termination delegate must be integrated deliberately, not
    // silently replaced. Inherit every existing launch/reopen/URL/dealloc hook.
    if parent
        .instance_method(sel!(applicationShouldTerminate:))
        .is_some()
    {
        return Err("macOS termination delegate already has a termination policy");
    }
    let mut class = ClassBuilder::new(name, parent)
        .ok_or("macOS guarded termination delegate could not be allocated")?;
    // SAFETY: This is AppKit's exact selector and NSUInteger reply ABI. The
    // sender is only an opaque object pointer and is never dereferenced.
    unsafe {
        class.add_method(
            sel!(applicationShouldTerminate:),
            application_should_terminate as extern "C" fn(_, _, _) -> _,
        );
    }
    let class = class.register();
    if class.instance_size() != parent.instance_size() || class.superclass() != Some(parent) {
        return Err("macOS guarded termination delegate changed the instance layout");
    }
    Ok(class)
}

fn install_native_quit_barrier(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let main_thread =
        MainThreadMarker::new().ok_or("macOS Quit must be installed on the main thread")?;
    let application = NSApplication::sharedApplication(main_thread);
    let delegate = application
        .delegate()
        .ok_or("macOS application delegate is missing")?;
    let delegate: &AnyObject = (&*delegate).as_ref();
    let parent = delegate.class();
    // Tao owns this instance and its ivars. Never substitute a different
    // instance, add ivars or alter a global NSApplication/NSObject method.
    if parent.name() != c"TaoAppDelegateParent" {
        return Err("macOS application delegate is not the supported Tao delegate".into());
    }
    let class = guarded_delegate_class(c"MonGuardedAppDelegate", parent)?;
    QUIT_APP
        .set(app.handle().clone())
        .map_err(|_| "macOS Quit was already installed")?;
    // SAFETY: Main-thread setup occurs before any UI action can terminate the
    // app. The new class directly subclasses the current class, adds no ivars,
    // and has the same checked instance size; inherited methods are unchanged.
    let previous = unsafe { AnyObject::set_class(delegate, class) };
    assert_eq!(
        previous, parent,
        "macOS delegate changed during Quit installation"
    );
    Ok(())
}

pub(crate) fn install(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    install_native_quit_barrier(app)?;
    // Preserve Tauri's Edit, File, View, Window and Help menus and native roles.
    let menu = Menu::default(app.handle())?;
    let Some(MenuItemKind::Submenu(original)) = menu.items()?.into_iter().next() else {
        return Err("macOS application menu is missing".into());
    };
    let package = app.package_info();
    let config = app.config();
    let about = AboutMetadata {
        name: config.product_name.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::{msg_send, rc::Retained, ClassType};
    use objc2_foundation::NSObject;

    extern "C" fn inherited_marker(_: &AnyObject, _: Sel) -> u64 {
        17
    }

    #[test]
    fn native_quit_subclass_preserves_instance_and_inherited_methods_and_cancels_termination() {
        let mut parent = ClassBuilder::new(c"MonQuitTestParent", NSObject::class()).unwrap();
        parent.add_ivar::<u64>(c"monTestMarker");
        unsafe {
            parent.add_method(
                sel!(monTestMarker),
                inherited_marker as extern "C" fn(_, _) -> _,
            );
        }
        let parent = parent.register();
        let child = guarded_delegate_class(c"MonQuitTestChild", parent).unwrap();
        assert_eq!(parent.instance_size(), child.instance_size());
        assert_eq!(child.superclass(), Some(parent));
        assert_eq!(
            parent.instance_variable(c"monTestMarker"),
            child.instance_variable(c"monTestMarker")
        );
        assert_eq!(
            parent.instance_method(sel!(monTestMarker)),
            child.instance_method(sel!(monTestMarker))
        );
        // Only this test-owned instance changes class. A null sender is safe:
        // the callback treats it as opaque, never as a Rust reference.
        let instance: Retained<AnyObject> = unsafe { msg_send![parent, new] };
        assert_eq!(unsafe { AnyObject::set_class(&instance, child) }, parent);
        let marker: u64 = unsafe { msg_send![&*instance, monTestMarker] };
        assert_eq!(marker, 17);
        let reply: NSApplicationTerminateReply = unsafe {
            msg_send![&*instance, applicationShouldTerminate: std::ptr::null::<NSApplication>()]
        };
        assert_eq!(reply, NSApplicationTerminateReply::TerminateCancel);
        assert_eq!(instance.class(), child);
    }

    #[test]
    fn native_quit_refuses_to_override_an_existing_termination_policy() {
        let parent = guarded_delegate_class(c"MonQuitExistingPolicy", NSObject::class()).unwrap();
        assert!(guarded_delegate_class(c"MonQuitOverrideMustNotExist", parent).is_err());
        assert!(AnyClass::get(c"MonQuitOverrideMustNotExist").is_none());
    }

    #[test]
    fn native_quit_refuses_a_duplicate_class_without_changing_the_parent() {
        let parent = NSObject::class();
        let original_size = parent.instance_size();
        guarded_delegate_class(c"MonQuitDuplicate", parent).unwrap();
        assert!(guarded_delegate_class(c"MonQuitDuplicate", parent).is_err());
        assert_eq!(parent.instance_size(), original_size);
        assert!(parent
            .instance_method(sel!(applicationShouldTerminate:))
            .is_none());
    }
}
