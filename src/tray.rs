//! System tray icon (Windows and macOS). On other platforms this is a no-op stub.

use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// State shared with the tray event handlers.
#[derive(Default)]
pub struct Shared {
    /// Main window currently shown (false when hidden to the tray).
    pub visible: AtomicBool,
    /// The user chose Quit from the tray menu.
    pub quit: AtomicBool,
}

#[cfg(any(windows, target_os = "macos"))]
mod imp {
    use super::*;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    pub struct Tray {
        icon: TrayIcon,
        pub shared: Arc<Shared>,
        last_tip: String,
    }

    pub const SUPPORTED: bool = true;

    fn toggle(ctx: &egui::Context, shared: &Shared) {
        let show = !shared.visible.load(Ordering::Relaxed);
        set_visible(ctx, shared, show);
    }

    pub fn set_visible(ctx: &egui::Context, shared: &Shared, show: bool) {
        shared.visible.store(show, Ordering::Relaxed);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(show));
        if show {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        ctx.request_repaint();
    }

    impl Tray {
        pub fn new(ctx: &egui::Context) -> Option<Self> {
            let img = crate::icon::window_icon();
            let icon = Icon::from_rgba(img.rgba, img.width, img.height).ok()?;
            let menu = Menu::new();
            let show = MenuItem::new("Show / hide", true, None);
            let quit = MenuItem::new("Quit", true, None);
            menu.append(&show).ok()?;
            menu.append(&PredefinedMenuItem::separator()).ok()?;
            menu.append(&quit).ok()?;

            let shared = Arc::new(Shared::default());
            shared.visible.store(true, Ordering::Relaxed);

            let (show_id, quit_id) = (show.id().clone(), quit.id().clone());
            let (c, s) = (ctx.clone(), shared.clone());
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                if e.id == show_id {
                    toggle(&c, &s);
                } else if e.id == quit_id {
                    s.quit.store(true, Ordering::Relaxed);
                    c.send_viewport_cmd(egui::ViewportCommand::Close);
                    c.request_repaint();
                }
            }));
            let (c, s) = (ctx.clone(), shared.clone());
            TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                    toggle(&c, &s);
                }
            }));

            let icon = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .with_tooltip("ACOM Controller")
                .with_icon(icon)
                .build()
                .ok()?;
            Some(Self { icon, shared, last_tip: String::new() })
        }

        pub fn set_tooltip(&mut self, tip: &str) {
            if tip != self.last_tip {
                let _ = self.icon.set_tooltip(Some(tip));
                self.last_tip = tip.to_owned();
            }
        }
    }

    impl Drop for Tray {
        fn drop(&mut self) {
            MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
            TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::*;

    pub struct Tray {
        pub shared: Arc<Shared>,
    }

    pub const SUPPORTED: bool = false;

    pub fn set_visible(_: &egui::Context, _: &Shared, _: bool) {}

    impl Tray {
        pub fn new(_: &egui::Context) -> Option<Self> {
            None
        }
        pub fn set_tooltip(&mut self, _: &str) {}
    }
}

pub use imp::{SUPPORTED, Tray, set_visible};
