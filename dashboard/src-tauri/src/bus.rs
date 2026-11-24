//! The one D-Bus user: is a tray host registered? (DESIGN 8.8, 9.11). The only module that
//! may use `zbus`. T15 writes the body.

/// Whether something shows tray icons right now. Injected, so window tests need no bus.
pub trait TrayHost: Send + Sync {
    /// True when a StatusNotifier host is registered at this moment.
    fn present(&self) -> bool;
}

/// Asks the session bus each time; never cached.
#[derive(Copy, Clone, Debug, Default)]
pub struct SessionBusTrayHost;

impl TrayHost for SessionBusTrayHost {
    fn present(&self) -> bool {
        false
    }
}
