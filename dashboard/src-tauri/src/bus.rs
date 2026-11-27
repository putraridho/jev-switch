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
    /// Runs the bus call on its own thread: it needs its own runtime, and building one on a
    /// thread that already drives async tasks (an async command) panics.
    fn present(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            std::thread::spawn(ask_bus).join().unwrap_or(false)
        }
        #[cfg(not(target_os = "linux"))]
        {
            true
        }
    }
}

#[cfg(target_os = "linux")]
fn ask_bus() -> bool {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return false;
    };
    rt.block_on(async {
        let builder = zbus::connection::Builder::session();
        let Ok(conn) = builder.build().await else {
            return false;
        };
        let Ok(dest) = zbus::names::BusName::try_from("org.kde.StatusNotifierWatcher") else {
            return false;
        };
        let Ok(path) = zbus::ObjectPath::try_from("/StatusNotifierWatcher") else {
            return false;
        };
        let Ok(iface) = zbus::names::InterfaceName::try_from("org.kde.StatusNotifierWatcher")
        else {
            return false;
        };
        let reply = conn
            .call_method(
                Some(dest),
                path,
                Some(iface),
                "IsStatusNotifierHostRegistered",
                &(),
            )
            .await;
        match reply {
            Ok(msg) => msg.body().deserialize::<bool>().unwrap_or(false),
            Err(_) => false,
        }
    })
}
