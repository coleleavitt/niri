use client::ClientId;
use niri_config::Config;
use smithay::backend::input::ButtonState;
use smithay::input::pointer::ButtonEvent;
use smithay::utils::{Point, SERIAL_COUNTER};
use wayland_client::protocol::wl_surface::WlSurface;

use super::*;

// Sets up a fixture with one output and a 100×100 tiled window at (16, 16), with the pointer over
// the window at surface-local (50, 50).
fn set_up() -> (Fixture, ClientId, WlSurface, u32) {
    let mut f = Fixture::with_config(Config::default());
    f.add_output(1, (1920, 1080));

    let id = f.add_client();
    f.client(id).create_pointer();
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

    f.niri_state().move_cursor(Point::from((66., 66.)));
    f.double_roundtrip(id);

    let (focus, serial) = f.client(id).state.pointer_focus.clone().unwrap();
    assert_eq!(focus, surface);
    assert_eq!(f.client(id).state.pointer_location, (50., 50.));

    (f, id, surface, serial)
}

fn pointer_location(f: &mut Fixture) -> (f64, f64) {
    let loc = f.niri().seat.get_pointer().unwrap().current_location();
    (loc.x, loc.y)
}

#[test]
fn warp_within_focused_surface() {
    let (mut f, id, surface, serial) = set_up();

    f.client(id).warp_pointer(&surface, 10., 20.5, serial);
    f.double_roundtrip(id);

    assert_eq!(pointer_location(&mut f), (26., 36.5));
    assert_eq!(f.client(id).state.pointer_location, (10., 20.5));
}

#[test]
fn reject_wrong_serial() {
    let (mut f, id, surface, serial) = set_up();

    f.client(id).warp_pointer(&surface, 10., 20., serial + 1);
    f.double_roundtrip(id);

    assert_eq!(pointer_location(&mut f), (66., 66.));
}

#[test]
fn reject_outside_surface() {
    let (mut f, id, surface, serial) = set_up();

    f.client(id).warp_pointer(&surface, 100., 20., serial);
    f.client(id).warp_pointer(&surface, -1., 20., serial);
    f.double_roundtrip(id);

    assert_eq!(pointer_location(&mut f), (66., 66.));
}

#[test]
fn reject_without_pointer_focus() {
    let (mut f, id, surface, serial) = set_up();

    f.niri_state().move_cursor(Point::from((500., 500.)));
    f.double_roundtrip(id);
    assert_eq!(f.client(id).state.pointer_focus, None);

    f.client(id).warp_pointer(&surface, 10., 20., serial);
    f.double_roundtrip(id);

    assert_eq!(pointer_location(&mut f), (500., 500.));
}

#[test]
fn warp_during_implicit_grab() {
    let (mut f, id, surface, serial) = set_up();

    let pointer = f.niri().seat.get_pointer().unwrap();
    pointer.button(
        f.niri_state(),
        &ButtonEvent {
            serial: SERIAL_COUNTER.next_serial(),
            time: 0,
            button: 0x110,
            state: ButtonState::Pressed,
        },
    );
    f.niri_state().move_cursor(Point::from((500., 500.)));
    f.double_roundtrip(id);
    assert_eq!(f.client(id).state.pointer_focus.clone().unwrap().0, surface);

    f.client(id).warp_pointer(&surface, 10., 20., serial);
    f.double_roundtrip(id);

    assert_eq!(pointer_location(&mut f), (26., 36.));
}
