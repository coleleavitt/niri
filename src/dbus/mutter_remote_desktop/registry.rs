use std::collections::HashMap;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use calloop::ping::Ping;
use zbus::fdo;

use super::clipboard::{RemoteDesktopClipboardOwner, RemoteDesktopClipboardSession};
use super::input::{RemoteDesktopAbsolutePosition, RemoteDesktopPosition};
use crate::utils::{CastSessionId, CastStreamId};

#[derive(Clone, Debug)]
pub struct RemoteDesktopSessionRegistry {
    pub(super) sessions: Arc<Mutex<HashMap<String, RemoteDesktopSession>>>,
    pub(super) clipboard_owner: Arc<Mutex<RemoteDesktopClipboardOwner>>,
    pub(super) next_clipboard_serial: Arc<AtomicU32>,
}

#[derive(Clone, Debug)]
pub struct RemoteDesktopStream {
    pub stream_id: CastStreamId,
    target: RemoteDesktopStreamTarget,
}

#[derive(Clone, Debug)]
pub struct RemoteDesktopOutputStream {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug)]
pub enum RemoteDesktopStreamTarget {
    Output {
        name: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
    Window {
        id: u64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteDesktopOutputRegion {
    pub mapping_id: u64,
    pub output_name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteDesktopWindowRegion {
    pub mapping_id: u64,
    pub window_id: u64,
}

#[derive(Debug, Default)]
pub(super) struct RemoteDesktopSession {
    pub(super) path: String,
    pub(super) started: bool,
    pub(super) screen_cast_session: Option<CastSessionId>,
    pub(super) streams: HashMap<String, RemoteDesktopStream>,
    pub(super) clipboard: RemoteDesktopClipboardSession,
    /// Wakes the session's EIS connections to re-advertise their window regions.
    pub(super) eis_window_region_pings: Vec<Ping>,
}

impl Default for RemoteDesktopSessionRegistry {
    fn default() -> Self {
        Self {
            sessions: Arc::default(),
            clipboard_owner: Arc::default(),
            next_clipboard_serial: Arc::new(AtomicU32::new(1)),
        }
    }
}

impl RemoteDesktopSessionRegistry {
    pub fn insert_session(&self, session_id: String, path: String) -> fdo::Result<()> {
        self.with_sessions(|sessions| {
            if sessions.contains_key(&session_id) {
                return Err(fdo::Error::Failed(format!(
                    "remote desktop session already exists: {session_id}"
                )));
            }

            let session = RemoteDesktopSession {
                path,
                ..Default::default()
            };
            sessions.insert(session_id, session);
            Ok(())
        })
    }

    pub fn contains_session(&self, session_id: &str) -> bool {
        self.with_sessions(|sessions| Ok(sessions.contains_key(session_id)))
            .unwrap_or(false)
    }

    pub fn session_count(&self) -> usize {
        self.with_sessions(|sessions| Ok(sessions.len()))
            .unwrap_or(0)
    }

    pub fn session_paths(&self) -> Vec<String> {
        self.with_sessions(|sessions| Ok(sessions.values().map(|s| s.path.clone()).collect()))
            .unwrap_or_default()
    }

    pub fn remove_session(&self, session_id: &str) -> fdo::Result<()> {
        self.with_sessions(|sessions| {
            sessions.remove(session_id);
            Ok(())
        })
    }

    pub fn mark_started(&self, session_id: &str) -> fdo::Result<()> {
        self.with_session(session_id, |session| {
            session.started = true;
            Ok(())
        })
    }

    pub fn associate_screen_cast(
        &self,
        session_id: &str,
        screen_cast_session: CastSessionId,
    ) -> fdo::Result<()> {
        self.with_session(session_id, |session| {
            if session.started {
                return Err(fdo::Error::Failed(format!(
                    "remote desktop session is already started: {session_id}"
                )));
            }

            if session.screen_cast_session.is_some() {
                return Err(fdo::Error::Failed(format!(
                    "remote desktop session already has a screen cast: {session_id}"
                )));
            }

            session.screen_cast_session = Some(screen_cast_session);
            Ok(())
        })
    }

    pub fn clear_screen_cast(&self, screen_cast_session: CastSessionId) -> fdo::Result<()> {
        self.with_sessions(|sessions| {
            for session in sessions.values_mut() {
                if session.screen_cast_session == Some(screen_cast_session) {
                    session.screen_cast_session = None;
                    session.streams.clear();
                }
            }

            Ok(())
        })
    }

    pub fn register_stream(
        &self,
        screen_cast_session: CastSessionId,
        path: String,
        stream: RemoteDesktopStream,
    ) -> fdo::Result<()> {
        self.with_sessions(|sessions| {
            let Some(session) = sessions
                .values_mut()
                .find(|session| session.screen_cast_session == Some(screen_cast_session))
            else {
                return Err(fdo::Error::Failed(
                    "screen cast session is not attached to a remote desktop session".to_owned(),
                ));
            };

            session.streams.insert(path, stream);
            Ok(())
        })
    }

    pub fn stream_position(
        &self,
        stream_path: &str,
        x: f64,
        y: f64,
    ) -> fdo::Result<RemoteDesktopAbsolutePosition> {
        if !x.is_finite() || !y.is_finite() {
            return Err(fdo::Error::InvalidArgs(
                "remote desktop coordinates must be finite".to_owned(),
            ));
        }

        self.with_sessions(|sessions| {
            let Some(stream) = sessions
                .values()
                .find_map(|session| session.streams.get(stream_path))
            else {
                return Err(fdo::Error::Failed(format!(
                    "remote desktop stream not found: {stream_path}"
                )));
            };

            Ok(stream.position(x, y))
        })
    }

    pub fn output_regions(&self, session_id: &str) -> fdo::Result<Vec<RemoteDesktopOutputRegion>> {
        self.with_session(session_id, |session| {
            let mut regions = session
                .streams
                .values()
                .filter_map(RemoteDesktopStream::output_region)
                .collect::<Vec<_>>();
            regions.sort_by_key(|region| region.mapping_id);
            Ok(regions)
        })
    }

    pub fn window_regions(&self, session_id: &str) -> fdo::Result<Vec<RemoteDesktopWindowRegion>> {
        self.with_session(session_id, |session| {
            let mut regions = session
                .streams
                .values()
                .filter_map(RemoteDesktopStream::window_region)
                .collect::<Vec<_>>();
            regions.sort_by_key(|region| region.mapping_id);
            Ok(regions)
        })
    }

    pub(super) fn add_eis_window_region_ping(
        &self,
        session_id: &str,
        ping: Ping,
    ) -> fdo::Result<()> {
        self.with_session(session_id, |session| {
            session.eis_window_region_pings.push(ping);
            Ok(())
        })
    }

    /// Wakes the EIS connections of the session that owns the window stream.
    pub fn window_stream_resized(&self, stream_id: CastStreamId) {
        let _ = self.with_sessions(|sessions| {
            let session = sessions.values().find(|session| {
                session.streams.values().any(|stream| {
                    stream.stream_id == stream_id
                        && matches!(stream.target, RemoteDesktopStreamTarget::Window { .. })
                })
            });
            for ping in session.iter().flat_map(|s| &s.eis_window_region_pings) {
                ping.ping();
            }
            Ok(())
        });
    }

    pub(super) fn with_sessions<T>(
        &self,
        f: impl FnOnce(&mut HashMap<String, RemoteDesktopSession>) -> fdo::Result<T>,
    ) -> fdo::Result<T> {
        let mut sessions = self.sessions.lock().map_err(|_| {
            fdo::Error::Failed("remote desktop session registry is poisoned".to_owned())
        })?;
        f(&mut sessions)
    }

    pub(super) fn with_session<T>(
        &self,
        session_id: &str,
        f: impl FnOnce(&mut RemoteDesktopSession) -> fdo::Result<T>,
    ) -> fdo::Result<T> {
        self.with_sessions(|sessions| {
            let Some(session) = sessions.get_mut(session_id) else {
                return Err(fdo::Error::Failed(format!(
                    "remote desktop session not found: {session_id}"
                )));
            };

            f(session)
        })
    }
}

impl RemoteDesktopStream {
    pub fn output(stream_id: CastStreamId, output: RemoteDesktopOutputStream) -> Self {
        Self {
            stream_id,
            target: RemoteDesktopStreamTarget::Output {
                name: output.name,
                x: output.x,
                y: output.y,
                width: output.width,
                height: output.height,
            },
        }
    }

    pub fn window(stream_id: CastStreamId, window_id: u64) -> Self {
        Self {
            stream_id,
            target: RemoteDesktopStreamTarget::Window { id: window_id },
        }
    }

    fn position(&self, x: f64, y: f64) -> RemoteDesktopAbsolutePosition {
        match &self.target {
            RemoteDesktopStreamTarget::Output {
                name,
                x: _,
                y: _,
                width,
                height,
            } => RemoteDesktopAbsolutePosition::Output(RemoteDesktopPosition {
                output_name: name.clone(),
                x,
                y,
                width: *width,
                height: *height,
            }),
            RemoteDesktopStreamTarget::Window { id } => {
                RemoteDesktopAbsolutePosition::Window { id: *id, x, y }
            }
        }
    }

    fn output_region(&self) -> Option<RemoteDesktopOutputRegion> {
        match &self.target {
            RemoteDesktopStreamTarget::Output {
                name,
                x,
                y,
                width,
                height,
            } => Some(RemoteDesktopOutputRegion {
                mapping_id: self.stream_id.get(),
                output_name: name.clone(),
                x: *x,
                y: *y,
                width: *width,
                height: *height,
            }),
            RemoteDesktopStreamTarget::Window { .. } => None,
        }
    }

    fn window_region(&self) -> Option<RemoteDesktopWindowRegion> {
        match &self.target {
            RemoteDesktopStreamTarget::Output { .. } => None,
            RemoteDesktopStreamTarget::Window { id } => Some(RemoteDesktopWindowRegion {
                mapping_id: self.stream_id.get(),
                window_id: *id,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn registry_rejects_screen_cast_association_after_session_start() {
        let registry = RemoteDesktopSessionRegistry::default();
        let session_id = "started-session".to_owned();

        registry
            .insert_session(session_id.clone(), String::new())
            .unwrap();
        registry.mark_started(&session_id).unwrap();
        let err = registry
            .associate_screen_cast(&session_id, CastSessionId::next())
            .unwrap_err();

        assert!(format!("{err:?}").contains("already started"));
    }

    #[test]
    fn registry_resolves_output_stream_positions() {
        let registry = RemoteDesktopSessionRegistry::default();
        let session_id = "stream-session".to_owned();
        let screen_cast_session = CastSessionId::next();
        let stream_id = CastStreamId::next();

        registry
            .insert_session(session_id.clone(), String::new())
            .unwrap();
        registry
            .associate_screen_cast(&session_id, screen_cast_session)
            .unwrap();
        registry
            .register_stream(
                screen_cast_session,
                "/org/gnome/Mutter/ScreenCast/Stream/u999".to_owned(),
                RemoteDesktopStream::output(
                    stream_id,
                    RemoteDesktopOutputStream {
                        name: "HDMI-A-1".to_owned(),
                        x: 0,
                        y: 0,
                        width: 1920,
                        height: 1080,
                    },
                ),
            )
            .unwrap();

        let position = registry
            .stream_position("/org/gnome/Mutter/ScreenCast/Stream/u999", 320.0, 240.0)
            .unwrap();
        let RemoteDesktopAbsolutePosition::Output(position) = position else {
            panic!("expected an output position: {position:?}");
        };

        assert_eq!(position.output_name, "HDMI-A-1");
        assert_eq!(position.x, 320.0);
        assert_eq!(position.y, 240.0);
        assert_eq!(position.width, 1920);
        assert_eq!(position.height, 1080);
    }

    #[test]
    fn registry_lists_remote_desktop_output_regions() {
        let registry = RemoteDesktopSessionRegistry::default();
        let session_id = "stream-session".to_owned();
        let screen_cast_session = CastSessionId::next();
        let stream_id = CastStreamId::next();

        registry
            .insert_session(session_id.clone(), String::new())
            .unwrap();
        registry
            .associate_screen_cast(&session_id, screen_cast_session)
            .unwrap();
        registry
            .register_stream(
                screen_cast_session,
                "/org/gnome/Mutter/ScreenCast/Stream/u999".to_owned(),
                RemoteDesktopStream::output(
                    stream_id,
                    RemoteDesktopOutputStream {
                        name: "HDMI-A-1".to_owned(),
                        x: -1920,
                        y: 120,
                        width: 1920,
                        height: 1080,
                    },
                ),
            )
            .unwrap();

        let regions = registry.output_regions(&session_id).unwrap();

        assert_eq!(
            regions,
            vec![RemoteDesktopOutputRegion {
                mapping_id: stream_id.get(),
                output_name: "HDMI-A-1".to_owned(),
                x: -1920,
                y: 120,
                width: 1920,
                height: 1080,
            }]
        );
    }

    #[test]
    fn registry_resolves_window_stream_positions_to_the_window() {
        let registry = RemoteDesktopSessionRegistry::default();
        let session_id = "window-session".to_owned();
        let screen_cast_session = CastSessionId::next();
        let output_stream_id = CastStreamId::next();
        let window_stream_id = CastStreamId::next();
        let window_path = "/org/gnome/Mutter/ScreenCast/Stream/u1000";

        registry
            .insert_session(session_id.clone(), String::new())
            .unwrap();
        registry
            .associate_screen_cast(&session_id, screen_cast_session)
            .unwrap();
        registry
            .register_stream(
                screen_cast_session,
                "/org/gnome/Mutter/ScreenCast/Stream/u999".to_owned(),
                RemoteDesktopStream::output(
                    output_stream_id,
                    RemoteDesktopOutputStream {
                        name: "HDMI-A-1".to_owned(),
                        x: 0,
                        y: 0,
                        width: 1920,
                        height: 1080,
                    },
                ),
            )
            .unwrap();
        registry
            .register_stream(
                screen_cast_session,
                window_path.to_owned(),
                RemoteDesktopStream::window(window_stream_id, 42),
            )
            .unwrap();

        let position = registry.stream_position(window_path, 12.5, 7.0).unwrap();
        assert!(matches!(
            position,
            RemoteDesktopAbsolutePosition::Window {
                id: 42,
                x: 12.5,
                y: 7.0
            }
        ));

        assert_eq!(
            registry.window_regions(&session_id).unwrap(),
            vec![RemoteDesktopWindowRegion {
                mapping_id: window_stream_id.get(),
                window_id: 42,
            }]
        );
        let output_regions = registry.output_regions(&session_id).unwrap();
        assert_eq!(output_regions.len(), 1);
        assert_eq!(output_regions[0].mapping_id, output_stream_id.get());
    }

    #[test]
    fn registry_rejects_non_finite_window_stream_positions() {
        let registry = RemoteDesktopSessionRegistry::default();
        let session_id = "window-session".to_owned();
        let screen_cast_session = CastSessionId::next();
        let window_path = "/org/gnome/Mutter/ScreenCast/Stream/u1001";

        registry
            .insert_session(session_id.clone(), String::new())
            .unwrap();
        registry
            .associate_screen_cast(&session_id, screen_cast_session)
            .unwrap();
        registry
            .register_stream(
                screen_cast_session,
                window_path.to_owned(),
                RemoteDesktopStream::window(CastStreamId::next(), 42),
            )
            .unwrap();

        assert!(registry.stream_position(window_path, f64::NAN, 0.).is_err());
        assert!(registry
            .stream_position(window_path, 0., f64::INFINITY)
            .is_err());
    }

    #[test]
    fn registry_wakes_eis_connections_of_resized_window_streams() {
        let registry = RemoteDesktopSessionRegistry::default();
        let mut event_loop = calloop::EventLoop::<Vec<&str>>::try_new().unwrap();
        let add_session = |name: &'static str| {
            let screen_cast_session = CastSessionId::next();
            registry
                .insert_session(name.to_owned(), String::new())
                .unwrap();
            registry
                .associate_screen_cast(name, screen_cast_session)
                .unwrap();
            let (ping, source) = calloop::ping::make_ping().unwrap();
            event_loop
                .handle()
                .insert_source(source, move |(), &mut (), woken| woken.push(name))
                .unwrap();
            registry.add_eis_window_region_ping(name, ping).unwrap();
            screen_cast_session
        };
        let first = add_session("first");
        add_session("second");

        let output_stream_id = CastStreamId::next();
        let window_stream_id = CastStreamId::next();
        registry
            .register_stream(
                first,
                "/org/gnome/Mutter/ScreenCast/Stream/u1002".to_owned(),
                RemoteDesktopStream::output(
                    output_stream_id,
                    RemoteDesktopOutputStream {
                        name: "HDMI-A-1".to_owned(),
                        x: 0,
                        y: 0,
                        width: 1920,
                        height: 1080,
                    },
                ),
            )
            .unwrap();
        registry
            .register_stream(
                first,
                "/org/gnome/Mutter/ScreenCast/Stream/u1003".to_owned(),
                RemoteDesktopStream::window(window_stream_id, 42),
            )
            .unwrap();

        let mut woken = Vec::new();
        registry.window_stream_resized(output_stream_id);
        event_loop
            .dispatch(Some(Duration::ZERO), &mut woken)
            .unwrap();
        assert!(woken.is_empty());

        registry.window_stream_resized(window_stream_id);
        event_loop
            .dispatch(Some(Duration::ZERO), &mut woken)
            .unwrap();
        assert_eq!(woken, ["first"]);
    }
}
