//! The item in the system tray, which is how the application is normally reached.
//!
//! `SPEC.md` §4 wants a tray-and-window application that starts with the session, so that the
//! parents only ever think about the phone. The window spends most of its life closed; this
//! is what stays.
//!
//! `STACK.md` §3.2 chose `ksni`, which speaks `StatusNotifierItem` over D-Bus directly. That
//! is what Plasma actually consumes, so the tray needs neither libdbus nor the XEmbed bridge.

use std::sync::mpsc::Sender;

use ksni::blocking::TrayMethods;
use ksni::menu::{MenuItem, StandardItem};

/// What a person asked the tray to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    /// Bring the window up.
    Open,

    /// Leave. The transfers stop with it, which is why it is a menu item rather than a
    /// click: closing the window should not do this by accident.
    Quit,
}

/// The tray item, holding the channel it reports through.
pub struct Tray {
    asked: Sender<Asked>,
    status: String,
}

impl Tray {
    #[must_use]
    pub fn new(asked: Sender<Asked>) -> Self {
        Self {
            asked,
            status: "Waiting for a phone".to_string(),
        }
    }

    /// What the tray says when somebody hovers over it.
    pub fn set_status(&mut self, status: String) {
        self.status = status;
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").to_string()
    }

    fn title(&self) -> String {
        "Photo Sync".to_string()
    }

    fn icon_name(&self) -> String {
        // A stock icon rather than one shipped here: it follows the desktop's own theme, so
        // it looks like it belongs on whichever one this is.
        "camera-photo".to_string()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Photo Sync".to_string(),
            description: self.status.clone(),
            ..ksni::ToolTip::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.asked.send(Asked::Open);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Open Photo Sync".to_string(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.asked.send(Asked::Open);
                }),
                ..StandardItem::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".to_string(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.asked.send(Asked::Quit);
                }),
                ..StandardItem::default()
            }
            .into(),
        ]
    }
}

/// Puts the item in the tray, if there is one to put it in.
///
/// A desktop with no `StatusNotifierItem` host is not a reason to refuse to run: the window
/// and every transfer work without it, and saying so once is better than failing to start.
///
/// # Errors
/// When the tray service cannot be reached.
pub fn show(tray: Tray) -> Result<ksni::blocking::Handle<Tray>, ksni::Error> {
    tray.spawn()
}
