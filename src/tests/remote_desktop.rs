use std::io;
use std::os::unix::net::UnixStream;

use niri_config::{Action, Config};
use reis::event::{DeviceCapability, EiEvent, EiEventConverter};
use reis::handshake::EiHandshaker;
use reis::{ei, PendingRequestResult};
use smithay::utils::{Point, Size};
use wayland_client::protocol::wl_surface::WlSurface;

use super::client::ClientId;
use super::*;
use crate::dbus::mutter_remote_desktop::{
    dispatch_to_niri, RemoteDesktopAbsolutePosition, RemoteDesktopDeviceTypes,
    RemoteDesktopEisConnection, RemoteDesktopSessionRegistry, RemoteDesktopStream,
    RemoteDesktopToNiri,
};
use crate::dbus::mutter_screen_cast::ScreenCastToNiri;
use crate::utils::{CastSessionId, CastStreamId};

fn set_up() -> (Fixture, u64) {
    let (f, window_id, _, _) = set_up_with_client();
    (f, window_id)
}

fn set_up_with_client() -> (Fixture, u64, ClientId, WlSurface) {
    let config = Config::parse_mem(
        r#"
output "headless-2" {
    scale 2
}
"#,
    )
    .unwrap();
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));
    f.add_output(2, (2560, 1440));
    f.niri_focus_output(2);

    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(100, 100);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.niri_complete_animations();

    let window_id = f.niri().layout.focus().unwrap().id().get();
    (f, window_id, id, surface)
}

fn move_pointer(f: &mut Fixture, id: u64, x: f64, y: f64) {
    dispatch_to_niri(
        f.niri_state(),
        RemoteDesktopToNiri::PointerMotionAbsolute {
            position: RemoteDesktopAbsolutePosition::Window { id, x, y },
        },
    );
}

fn pointer_location(f: &mut Fixture) -> Point<f64, smithay::utils::Logical> {
    f.niri().seat.get_pointer().unwrap().current_location()
}

#[test]
fn window_stream_pointer_motion_lands_on_the_window() {
    let (mut f, id) = set_up();
    let geo = f.niri().window_geometry(id).unwrap();
    assert_eq!(geo.scale, Some(2.));
    let rect = geo.window_rect.unwrap();
    assert_ne!(rect.x, 0.);

    // Window stream coordinates are buffer pixels at the output scale.
    move_pointer(&mut f, id, 40., 60.);
    assert_eq!(
        pointer_location(&mut f),
        Point::from((rect.x + 20., rect.y + 30.))
    );
    let surface = f.niri().pointer_contents.window.clone().map(|(w, _)| w);
    let focused = f.niri().layout.focus().map(|m| m.window.clone());
    assert_eq!(surface, focused);

    // Positions outside the window are clamped to its last pixel.
    move_pointer(&mut f, id, 1000., -5.);
    assert_eq!(
        pointer_location(&mut f),
        Point::from((rect.x + rect.width - 0.5, rect.y))
    );
}

#[test]
fn window_stream_pointer_motion_is_dropped_when_the_window_cannot_be_targeted() {
    let (mut f, id) = set_up();

    move_pointer(&mut f, id, 10., 10.);
    let before = pointer_location(&mut f);

    move_pointer(&mut f, u64::MAX, 50., 50.);
    assert_eq!(pointer_location(&mut f), before);

    f.niri().layout.open_overview();
    move_pointer(&mut f, id, 50., 50.);
    assert_eq!(pointer_location(&mut f), before);
}

#[test]
fn window_stream_pointer_motion_is_dropped_while_virtual_input_is_blocked() {
    let (mut f, id) = set_up();

    move_pointer(&mut f, id, 10., 10.);
    let before = pointer_location(&mut f);

    f.niri_state()
        .do_action(Action::ToggleVirtualInputBlocked, false);
    move_pointer(&mut f, id, 50., 50.);
    assert_eq!(pointer_location(&mut f), before);

    f.niri_state()
        .do_action(Action::ToggleVirtualInputBlocked, false);
    move_pointer(&mut f, id, 50., 50.);
    assert_ne!(pointer_location(&mut f), before);
}

#[test]
fn window_stream_size_is_the_window_size_in_buffer_pixels() {
    let (mut f, id) = set_up();

    let (reply, rx) = async_channel::bounded(1);
    f.niri_state()
        .on_screen_cast_msg(ScreenCastToNiri::WindowStreamSize {
            window_id: id,
            reply,
        });

    // The window is 100×100 on an output with scale 2.
    assert_eq!(rx.try_recv().unwrap(), Some(Size::from((200, 200))));
}

/// Non-blocking libei sender client.
struct EiClient {
    context: ei::Context,
    handshaker: EiHandshaker<'static>,
    converter: Option<EiEventConverter>,
}

impl EiClient {
    fn new(socket: UnixStream) -> Self {
        Self {
            context: ei::Context::new(socket).unwrap(),
            handshaker: EiHandshaker::new("niri-test", ei::handshake::ContextType::Sender),
            converter: None,
        }
    }

    /// Dispatches niri and the client until the client receives an event.
    fn next_event(&mut self, f: &mut Fixture) -> EiEvent {
        for _ in 0..100 {
            if let Some(event) = self.converter.as_mut().and_then(|c| c.next_event()) {
                return event;
            }

            f.dispatch();
            match self.context.read() {
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => (),
                res => {
                    res.unwrap();
                }
            }
            while let Some(event) = self.context.pending_event() {
                let PendingRequestResult::Request(event) = event else {
                    panic!("invalid EIS event");
                };
                match &mut self.converter {
                    Some(converter) => converter.handle_event(event).unwrap(),
                    None => {
                        if let Some(resp) = self.handshaker.handle_event(event).unwrap() {
                            self.converter = Some(EiEventConverter::new(&self.context, resp));
                        }
                    }
                }
            }
        }
        panic!("no EIS event arrived");
    }
}

#[test]
fn eis_window_regions_are_readvertised_when_the_window_resizes() {
    let (mut f, id, client_id, surface) = set_up_with_client();

    let registry = RemoteDesktopSessionRegistry::default();
    let session_id = "eis-session";
    let screen_cast_session = CastSessionId::next();
    let stream_id = CastStreamId::next();
    registry
        .insert_session(session_id.to_owned(), String::new())
        .unwrap();
    registry
        .associate_screen_cast(session_id, screen_cast_session)
        .unwrap();
    registry
        .register_stream(
            screen_cast_session,
            "/org/gnome/Mutter/ScreenCast/Stream/u1".to_owned(),
            RemoteDesktopStream::window(stream_id, id),
        )
        .unwrap();

    let (server, client) = UnixStream::pair().unwrap();
    dispatch_to_niri(
        f.niri_state(),
        RemoteDesktopToNiri::ConnectEis(RemoteDesktopEisConnection::new(
            server,
            session_id.to_owned(),
            registry.clone(),
            RemoteDesktopDeviceTypes::default(),
        )),
    );
    let mut client = EiClient::new(client);

    let EiEvent::SeatAdded(seat) = client.next_event(&mut f) else {
        panic!("expected a seat");
    };
    seat.seat
        .bind_capabilities(DeviceCapability::PointerAbsolute | DeviceCapability::Button);
    client.context.flush().unwrap();

    let mapping_id = stream_id.get().to_string();
    let window_device = |event| match event {
        EiEvent::DeviceAdded(added) => {
            let regions = added.device.regions();
            let region = regions
                .iter()
                .find(|region| region.mapping_id.as_ref() == Some(&mapping_id))?;
            Some((added.device.clone(), (region.width, region.height)))
        }
        _ => None,
    };
    let (old_device, size) = std::iter::repeat_with(|| client.next_event(&mut f))
        .find_map(window_device)
        .unwrap();
    assert_eq!(size, (100, 100));

    let window = f.client(client_id).window(&surface);
    window.set_size(150, 120);
    window.commit();
    f.double_roundtrip(client_id);
    registry.window_stream_resized(stream_id);

    let mut removed = false;
    let (new_device, size) = std::iter::repeat_with(|| client.next_event(&mut f))
        .find_map(|event| {
            if let EiEvent::DeviceRemoved(event) = &event {
                removed |= event.device == old_device;
            }
            window_device(event)
        })
        .unwrap();
    assert!(removed);
    assert_ne!(new_device, old_device);
    assert_eq!(size, (150, 120));
}
