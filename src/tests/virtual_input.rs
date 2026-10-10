use niri_config::Action;
use niri_ipc::VirtualInput;

use super::*;

fn virtual_input(f: &mut Fixture) -> VirtualInput {
    f.niri().virtual_input_state()
}

#[test]
fn virtual_pointer_activity_and_revoke() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    assert_eq!(virtual_input(&mut f), VirtualInput::default());

    let pointer = f.client(id).create_virtual_pointer();
    f.roundtrip(id);
    let state = virtual_input(&mut f);
    assert_eq!(state.virtual_pointers, 1);
    assert!(!state.active);

    pointer.motion(0, 10., 10.);
    pointer.frame();
    f.roundtrip(id);
    let state = virtual_input(&mut f);
    assert!(state.active);
    assert!(state.last_activity.is_some());

    f.niri_state().do_action(Action::RevokeVirtualInput, false);
    let revoked = virtual_input(&mut f);
    assert_eq!(revoked.virtual_pointers, 0);
    assert!(!revoked.active);
    assert!(!revoked.blocked);

    // The revoked pointer is inert.
    pointer.motion(0, 10., 10.);
    pointer.frame();
    f.roundtrip(id);
    assert_eq!(virtual_input(&mut f), revoked);

    // New pointers work again.
    let pointer = f.client(id).create_virtual_pointer();
    pointer.motion(0, 10., 10.);
    f.roundtrip(id);
    let state = virtual_input(&mut f);
    assert_eq!(state.virtual_pointers, 1);
    assert!(state.active);
}

#[test]
fn virtual_keyboard_is_tracked_per_client() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    f.client(id).create_virtual_keyboard();
    f.client(id).create_virtual_keyboard();
    f.roundtrip(id);
    let state = virtual_input(&mut f);
    assert_eq!(state.virtual_keyboard_clients, 1);
    assert!(state.active);
}

#[test]
fn blocking_refuses_new_virtual_input() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    let pointer = f.client(id).create_virtual_pointer();
    f.roundtrip(id);

    f.niri_state()
        .do_action(Action::ToggleVirtualInputBlocked, false);
    let blocked = virtual_input(&mut f);
    assert!(blocked.blocked);
    assert_eq!(blocked.virtual_pointers, 0);

    // The existing pointer is revoked, and new devices from bound managers are inert.
    pointer.motion(0, 10., 10.);
    let pointer = f.client(id).create_virtual_pointer();
    pointer.motion(0, 10., 10.);
    f.client(id).create_virtual_keyboard();
    f.roundtrip(id);
    assert_eq!(virtual_input(&mut f), blocked);

    // New clients don't see the globals.
    let id = f.add_client();
    let client = f.client(id);
    assert!(client.state.virtual_pointer_manager.is_none());
    assert!(client.state.virtual_keyboard_manager.is_none());

    f.niri_state()
        .do_action(Action::ToggleVirtualInputBlocked, false);
    assert!(!virtual_input(&mut f).blocked);

    let id = f.add_client();
    let pointer = f.client(id).create_virtual_pointer();
    pointer.motion(0, 10., 10.);
    f.roundtrip(id);
    let state = virtual_input(&mut f);
    assert_eq!(state.virtual_pointers, 1);
    assert!(state.active);
}
