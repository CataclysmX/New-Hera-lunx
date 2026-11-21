//! Icon in the system tray (StatusNotifierItem: KDE and most desktops): open the window, choose
//! the page, quit. With it, closing the window keeps the app running.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use ksni::blocking::TrayMethods;
use ksni::menu::{RadioGroup, RadioItem, StandardItem, SubMenu};
use ksni::{Icon, MenuItem, ToolTip};

use crate::i18n::tr;
use crate::engine::Shared;

struct Tray {
    shared: Arc<Shared>,
    icons: Vec<Icon>,
}

/// The app icon at the usual tray sizes, ARGB32 in network byte order (StatusNotifierItem).
fn tray_icons() -> Vec<Icon> {
    let color = crate::icon::tray_color();
    [16, 22, 24, 32, 48, 64]
        .into_iter()
        .filter_map(|size| {
            let rgba = crate::icon::rgba(crate::icon::TRAY_SVG, size, &color)?;
            let data = rgba
                .chunks(4)
                .flat_map(|p| [p[3], p[0], p[1], p[2]])
                .collect();
            Some(Icon {
                width: size as i32,
                height: size as i32,
                data,
            })
        })
        .collect()
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "hsm01-app".into()
    }

    fn title(&self) -> String {
        crate::APP_NAME.into()
    }

    fn icon_name(&self) -> String {
        if self.icons.is_empty() {
            "audio-card".into() // theme icon if public/tray.svg could not be drawn
        } else {
            String::new()
        }
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icons.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        let s = self.shared.status.lock().unwrap().clone();
        let state = if !s.connected {
            tr("disconnected").to_owned()
        } else if s.asleep {
            tr("in standby").to_owned()
        } else {
            format!("firmware {}.{:02}", s.firmware.0, s.firmware.1)
        };
        ToolTip {
            title: format!("{} – Hercules Stream 100", crate::APP_NAME),
            description: state,
            ..Default::default()
        }
    }

    /// Click on the icon.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.shared.show_window.store(true, Ordering::Relaxed);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let pages: Vec<String> = self
            .shared
            .config
            .read()
            .unwrap()
            .pages
            .iter()
            .enumerate()
            .map(|(i, p)| format!("{}. {}", i + 1, p.name))
            .collect();
        vec![
            StandardItem {
                label: tr("Open the window").into(),
                icon_name: "window-new".into(),
                activate: Box::new(|t: &mut Self| {
                    t.shared.show_window.store(true, Ordering::Relaxed)
                }),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: tr("Page").into(),
                submenu: vec![
                    RadioGroup {
                        selected: self.shared.page.load(Ordering::Relaxed),
                        select: Box::new(|t: &mut Self, i| {
                            t.shared.page.store(i, Ordering::Relaxed)
                        }),
                        options: pages
                            .into_iter()
                            .map(|label| RadioItem {
                                label,
                                ..Default::default()
                            })
                            .collect(),
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: tr("Quit").into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|t: &mut Self| t.shared.quit.store(true, Ordering::Relaxed)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Show the tray icon; `false` when the desktop has no tray (closing the window then quits).
pub fn spawn(shared: Arc<Shared>) -> bool {
    match (Tray {
        shared: shared.clone(),
        icons: tray_icons(),
    })
    .spawn()
    {
        Ok(handle) => {
            // keep the menu (current page, page names) and the tooltip up to date
            let watch = move || {
                let mut last = None;
                while !shared.quit.load(Ordering::Relaxed) {
                    let s = shared.status.lock().unwrap().clone();
                    let now = (
                        shared.page.load(Ordering::Relaxed),
                        shared.config_version.load(Ordering::Relaxed),
                        shared
                            .config
                            .read()
                            .unwrap()
                            .pages
                            .iter()
                            .map(|p| p.name.clone())
                            .collect::<Vec<_>>(),
                        (s.connected, s.asleep),
                    );
                    if last.as_ref() != Some(&now) {
                        handle.update(|_| {});
                        last = Some(now);
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                handle.shutdown().wait();
            };
            std::thread::Builder::new()
                .name("hsm01-tray".into())
                .spawn(watch)
                .is_ok()
        }
        Err(e) => {
            eprintln!("hsm01-app: {}", tf!("no system tray icon ({e})", e = e));
            false
        }
    }
}
