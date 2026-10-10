use niri_config::{Action, Config};
use smithay::utils::Point;

use super::*;
use crate::dbus::mutter_remote_desktop::{
    dispatch_to_niri, RemoteDesktopAbsolutePosition, RemoteDesktopToNiri,
};

fn set_up() -> (Fixture, u64) {
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
    (f, window_id)
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
