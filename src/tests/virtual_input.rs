use std::collections::HashSet;

use niri_config::Action;
use niri_ipc::VirtualInput;
use smithay::input::keyboard::{xkb, KeymapFile};
use wayland_client::protocol::wl_keyboard::KeymapFormat;
use wayland_client::protocol::wl_pointer::ButtonState;

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
fn revoke_releases_virtual_pointer_buttons() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    let pointer = f.client(id).create_virtual_pointer();
    pointer.button(0, 0x110, ButtonState::Pressed);
    pointer.frame();
    f.roundtrip(id);
    assert!(f.niri().seat.get_pointer().unwrap().is_grabbed());

    f.niri_state().do_action(Action::RevokeVirtualInput, false);
    assert!(!f.niri().seat.get_pointer().unwrap().is_grabbed());
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
fn revoke_releases_virtual_keyboard_keys() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));

    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.client(id).create_keyboard();
    f.double_roundtrip(id);

    let typist = f.add_client();
    let keyboard = f.client(typist).create_virtual_keyboard();
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap =
        xkb::Keymap::new_from_names(&context, "", "", "", "", None, xkb::KEYMAP_COMPILE_NO_FLAGS)
            .unwrap();
    KeymapFile::new(&keymap)
        .with_fd(false, |fd, size| {
            keyboard.keymap(KeymapFormat::XkbV1 as u32, fd, size as u32);
        })
        .unwrap();
    // Shift and A.
    keyboard.modifiers(1, 0, 0, 0);
    keyboard.key(0, 30, 1);
    f.roundtrip(typist);
    f.roundtrip(id);
    let state = &f.client(id).state;
    assert_eq!(state.pressed_keys, HashSet::from([30]));
    assert_eq!(state.mods_depressed, 1);

    // Revoking disconnects the virtual keyboard client.
    f.client(typist).expect_protocol_error = true;
    f.niri_state().do_action(Action::RevokeVirtualInput, false);
    f.roundtrip(id);
    let state = &f.client(id).state;
    assert!(state.pressed_keys.is_empty());
    assert_eq!(state.mods_depressed, 0);
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
