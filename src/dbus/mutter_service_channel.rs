use std::os::unix::net::UnixStream;

use zbus::{fdo, interface, zvariant};

use super::Start;
use crate::niri::NewClient;

const PORTAL_BACKEND: u32 = 1;
const FILE_CHOOSER_PORTAL_BACKEND: u32 = 2;
const GLOBAL_SHORTCUTS_PORTAL_BACKEND: u32 = 3;

fn is_supported_service_client_type(service_client_type: u32) -> bool {
    matches!(
        service_client_type,
        PORTAL_BACKEND | FILE_CHOOSER_PORTAL_BACKEND | GLOBAL_SHORTCUTS_PORTAL_BACKEND
    )
}

pub struct ServiceChannel {
    to_niri: calloop::channel::Sender<NewClient>,
}

#[interface(name = "org.gnome.Mutter.ServiceChannel")]
impl ServiceChannel {
    async fn open_wayland_service_connection(
        &mut self,
        service_client_type: u32,
    ) -> fdo::Result<zvariant::OwnedFd> {
        if !is_supported_service_client_type(service_client_type) {
            return Err(fdo::Error::InvalidArgs(
                "Invalid service client type".to_owned(),
            ));
        }

        let (sock1, sock2) = UnixStream::pair().unwrap();
        let client = NewClient {
            client: sock2,
            restricted: false,
            // FIXME: maybe you can get the PID from D-Bus somehow?
            credentials_unknown: true,
        };
        if let Err(err) = self.to_niri.send(client) {
            warn!("error sending message to niri: {err:?}");
            return Err(fdo::Error::Failed("internal error".to_owned()));
        }

        Ok(zvariant::OwnedFd::from(std::os::fd::OwnedFd::from(sock1)))
    }
}

impl ServiceChannel {
    pub fn new(to_niri: calloop::channel::Sender<NewClient>) -> Self {
        Self { to_niri }
    }
}

impl Start for ServiceChannel {
    fn start(self) -> anyhow::Result<zbus::blocking::Connection> {
        let conn = zbus::blocking::connection::Builder::session()?
            .name("org.gnome.Mutter.ServiceChannel")?
            .serve_at("/org/gnome/Mutter/ServiceChannel", self)?
            .build()?;
        Ok(conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_mutter_service_client_types() {
        assert!(is_supported_service_client_type(PORTAL_BACKEND));
        assert!(is_supported_service_client_type(
            FILE_CHOOSER_PORTAL_BACKEND
        ));
        assert!(is_supported_service_client_type(
            GLOBAL_SHORTCUTS_PORTAL_BACKEND
        ));
    }

    #[test]
    fn rejects_unknown_service_client_types() {
        assert!(!is_supported_service_client_type(0));
        assert!(!is_supported_service_client_type(4));
        assert!(!is_supported_service_client_type(u32::MAX));
    }
}
