use std::fmt::Write as _;

use insta::assert_snapshot;
use niri_config::{Action, Config};
use smithay::backend::input::{InputEvent, InputTime, KeyState, Keycode};
use smithay::input::keyboard::xkb::Keymap;
use wayland_client::protocol::wl_surface::WlSurface;

use crate::tests::client::ClientId;
use crate::tests::fixture::Fixture;
use crate::tests::test_input_backend::{TestInputBackend, TestKeyboardKeyEvent};

enum Op {
    Press(Keycode),
    Release(Keycode),
}

fn parse(keymap: &Keymap, input: &str) -> Vec<Op> {
    let mut ops = Vec::new();
    for part in input.split_ascii_whitespace() {
        let name = &part[1..];
        let Some(key) = keymap.key_by_name(name) else {
            panic!("unknown key {name}");
        };

        let c = part.bytes().next().unwrap();
        let op = match c {
            b'+' => Op::Press(key),
            b'-' => Op::Release(key),
            _ => panic!("keys must begin with + or -, got {c}"),
        };

        ops.push(op);
    }
    ops
}

fn set_up(config: &str) -> (Fixture, ClientId, WlSurface) {
    let mut config = Config::parse_mem(config).unwrap();
    // knuffel doesn't understand #[cfg(test)]...
    for bind in &mut config.binds.0 {
        bind.action = Action::TestAction;
    }

    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));

    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.roundtrip(id);

    let _ = f.client(id).state.recent_keyboard_events(&surface);

    (f, id, surface)
}

fn run_f(f: &mut Fixture, id: ClientId, surface: &WlSurface, input: &str) -> String {
    let state = f.niri_state();
    let keyboard = state.niri.seat.get_keyboard().unwrap();
    let ops = keyboard.with_xkb_state(state, |xkb| {
        let xkb = xkb.xkb().lock().unwrap();
        let keymap = unsafe { xkb.keymap() };
        parse(keymap, input)
    });

    let mut rv = String::new();

    for op in ops {
        let (code, key_state) = match op {
            Op::Press(code) => (code, KeyState::Pressed),
            Op::Release(code) => (code, KeyState::Released),
        };

        let state = f.niri_state();
        let keyboard = state.niri.seat.get_keyboard().unwrap();
        keyboard.with_xkb_state(state, |xkb| {
            let xkb = xkb.xkb().lock().unwrap();
            let xkb_state = unsafe { xkb.state() };
            let keymap = xkb_state.get_keymap();

            let c = match key_state {
                KeyState::Pressed => "+",
                KeyState::Released => "-",
            };

            let name = keymap.key_get_name(code).unwrap_or("None");
            let keysym = xkb_state.key_get_one_sym(code);

            let _ = writeln!(&mut rv, "{c}{name} {:>3} {keysym:?}", code.raw());
        });

        let prev = state.niri.test_action_count;

        state.process_input_event(InputEvent::<TestInputBackend>::Keyboard {
            event: TestKeyboardKeyEvent {
                time: InputTime::from_micros(0),
                code,
                state: key_state,
                count: 1, // niri doesn't use this
            },
        });

        let diff = f.niri().test_action_count - prev;
        for _ in 0..diff {
            let _ = writeln!(&mut rv, "    niri test-action");
        }

        f.roundtrip(id);
        for event in f.client(id).state.recent_keyboard_events(surface) {
            let _ = writeln!(&mut rv, "    surface {event}");
        }
    }

    rv
}

fn run(config: &str, input: &str) -> String {
    let (mut f, id, surface) = set_up(config);
    run_f(&mut f, id, &surface, input)
}

#[test]
fn combos() {
    let c = "
    binds {
        Mod+Ctrl+Q { close-window; }
    }
    ";

    // Action press/release.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LatQ -LatQ -LCTL -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    +AD01  24 XK_q
        niri test-action
    -AD01  24 XK_q
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 29
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    "
    );

    // Extra Alt = no action.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LALT +LatQ -LatQ -LALT -LCTL -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    +LALT  64 XK_Alt_L
        surface modifiers: depressed=76, latched=0, locked=0, group=0
        surface key pressed: 56
    +AD01  24 XK_q
        surface key pressed: 16
    -AD01  24 XK_q
        surface key released: 16
    -LALT  64 XK_Alt_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key released: 56
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 29
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    "
    );

    // Key that doesn't correspond to any bind.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LatA -LatA -LCTL -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    +AC01  38 XK_a
        surface key pressed: 30
    -AC01  38 XK_a
        surface key released: 30
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 29
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    "
    );

    // Press action, press arbitrary, release action, release arbitrary.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LatQ +LatA -LatQ -LatA -LCTL -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    +AD01  24 XK_q
        niri test-action
    +AC01  38 XK_a
        surface key pressed: 30
    -AD01  24 XK_q
    -AC01  38 XK_a
        surface key released: 30
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 29
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    "
    );

    // Press arbitrary, press action, release arbitrary, release action.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LatA +LatQ -LatA -LatQ -LCTL -LWIN"),
        @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    +AC01  38 XK_a
        surface key pressed: 30
    +AD01  24 XK_q
        niri test-action
    -AC01  38 XK_a
        surface key released: 30
    -AD01  24 XK_q
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 29
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    "
    );

    // Trigger action then release mods.
    assert_snapshot!(
        run(c, "+LWIN +LCTL +LatQ -LCTL -LWIN -LatQ"),
        @"
    +LWIN 133 XK_Super_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key pressed: 125
    +LCTL  37 XK_Control_L
        surface modifiers: depressed=68, latched=0, locked=0, group=0
        surface key pressed: 29
    +AD01  24 XK_q
        niri test-action
    -LCTL  37 XK_Control_L
        surface modifiers: depressed=64, latched=0, locked=0, group=0
        surface key released: 29
    -LWIN 133 XK_Super_L
        surface modifiers: depressed=0, latched=0, locked=0, group=0
        surface key released: 125
    -AD01  24 XK_q
    "
    );
}
