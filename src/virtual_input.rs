//! Tracking of synthetic input (virtual pointers, virtual keyboards, RemoteDesktop).

use std::collections::HashSet;
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use calloop::timer::{TimeoutAction, Timer};
use calloop::RegistrationToken;
use smithay::backend::input::ButtonState;
#[cfg(feature = "xdp-gnome-screencast")]
use smithay::backend::input::{KeyState, Keycode};
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::Color32F;
use smithay::output::Output;
use smithay::reexports::wayland_protocols_misc::zwp_virtual_keyboard_v1::server::zwp_virtual_keyboard_manager_v1::{
    self, ZwpVirtualKeyboardManagerV1,
};
use smithay::reexports::wayland_server::Resource;
use smithay::input::pointer::ButtonEvent;
use smithay::utils::{Rectangle, SERIAL_COUNTER};

use crate::niri::{Niri, State};
use crate::render_helpers::solid_color::SolidColorRenderElement;
use crate::utils::{get_monotonic_time, output_size};

pub struct VirtualInputTracker {
    blocked: Arc<AtomicBool>,
    active: bool,
    last_activity: Option<Duration>,
    idle_timer: Option<RegistrationToken>,
    /// One virtual keyboard manager per client that created a virtual keyboard.
    keyboard_managers: Vec<ZwpVirtualKeyboardManagerV1>,
    /// Pointer buttons held by virtual pointers and RemoteDesktop.
    held_buttons: HashSet<u32>,
    /// Keys held by RemoteDesktop.
    #[cfg(feature = "xdp-gnome-screencast")]
    held_keys: HashSet<Keycode>,
    /// Touch points held by RemoteDesktop.
    #[cfg(feature = "xdp-gnome-screencast")]
    held_touch_slots: HashSet<u32>,
    indicator_ids: [Id; 4],
    indicator_commit: CommitCounter,
}

impl VirtualInputTracker {
    pub fn new() -> Self {
        Self {
            blocked: Arc::new(AtomicBool::new(false)),
            active: false,
            last_activity: None,
            idle_timer: None,
            keyboard_managers: Vec::new(),
            held_buttons: HashSet::new(),
            #[cfg(feature = "xdp-gnome-screencast")]
            held_keys: HashSet::new(),
            #[cfg(feature = "xdp-gnome-screencast")]
            held_touch_slots: HashSet::new(),
            indicator_ids: [Id::new(), Id::new(), Id::new(), Id::new()],
            indicator_commit: CommitCounter::default(),
        }
    }

    /// Flag shared with protocol global filters and the RemoteDesktop D-Bus interface.
    pub fn blocked_flag(&self) -> Arc<AtomicBool> {
        self.blocked.clone()
    }

    pub fn is_blocked(&self) -> bool {
        self.blocked.load(Ordering::Relaxed)
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn keyboard_created(&mut self, manager: &ZwpVirtualKeyboardManagerV1) {
        let client = manager.client().map(|c| c.id());
        if !self
            .keyboard_managers
            .iter()
            .any(|m| m.client().map(|c| c.id()) == client)
        {
            self.keyboard_managers.push(manager.clone());
        }
    }

    pub fn track_button(&mut self, button: u32, state: ButtonState) {
        match state {
            ButtonState::Pressed => self.held_buttons.insert(button),
            ButtonState::Released => self.held_buttons.remove(&button),
        };
    }

    #[cfg(feature = "xdp-gnome-screencast")]
    pub fn track_key(&mut self, keycode: Keycode, state: KeyState) {
        match state {
            KeyState::Pressed => self.held_keys.insert(keycode),
            KeyState::Released => self.held_keys.remove(&keycode),
        };
    }

    #[cfg(feature = "xdp-gnome-screencast")]
    pub fn track_touch(&mut self, slot: u32, down: bool) {
        if down {
            self.held_touch_slots.insert(slot);
        } else {
            self.held_touch_slots.remove(&slot);
        }
    }

    /// Forgets clients that disconnected.
    pub fn refresh(&mut self) {
        self.keyboard_managers.retain(|m| m.is_alive());
    }

    pub fn indicator_config_changed(&mut self) {
        self.indicator_commit.increment();
    }
}

impl Niri {
    pub fn virtual_input_activity(&mut self) {
        self.virtual_input.last_activity = Some(get_monotonic_time());

        if self.virtual_input.active {
            return;
        }
        self.virtual_input.active = true;
        self.queue_redraw_all();

        let timer = Timer::from_duration(self.virtual_input_idle_timeout());
        let token = self
            .event_loop
            .insert_source(timer, |_, _, state| state.niri.virtual_input_idle_check())
            .unwrap();
        self.virtual_input.idle_timer = Some(token);
    }

    fn virtual_input_idle_timeout(&self) -> Duration {
        let ms = self.config.borrow().virtual_input_indicator.idle_timeout_ms;
        Duration::from_millis(u64::from(ms))
    }

    fn virtual_input_idle_check(&mut self) -> TimeoutAction {
        let timeout = self.virtual_input_idle_timeout();
        let last = self.virtual_input.last_activity.unwrap_or_default();
        let elapsed = get_monotonic_time().saturating_sub(last);
        if elapsed < timeout {
            return TimeoutAction::ToDuration(timeout - elapsed);
        }

        self.virtual_input.idle_timer = None;
        self.virtual_input.active = false;
        self.queue_redraw_all();
        TimeoutAction::Drop
    }

    pub fn virtual_keyboard_created(&mut self, manager: &ZwpVirtualKeyboardManagerV1) {
        self.virtual_input.keyboard_created(manager);
        self.virtual_input_activity();
    }

    pub fn virtual_input_state(&self) -> niri_ipc::VirtualInput {
        let tracker = &self.virtual_input;

        #[cfg(feature = "xdp-gnome-screencast")]
        let remote_desktop_sessions = self
            .dbus
            .as_ref()
            .and_then(|dbus| dbus.remote_desktop_sessions.as_ref())
            .map_or(0, |sessions| sessions.session_count());
        #[cfg(not(feature = "xdp-gnome-screencast"))]
        let remote_desktop_sessions = 0;

        niri_ipc::VirtualInput {
            active: tracker.active,
            blocked: tracker.is_blocked(),
            virtual_pointers: self.virtual_pointer_state.count() as u32,
            virtual_keyboard_clients: tracker.keyboard_managers.len() as u32,
            remote_desktop_sessions: remote_desktop_sessions as u32,
            last_activity: tracker.last_activity.map(niri_ipc::Timestamp::from),
        }
    }

    /// Frame around the output while synthetic input is active.
    pub fn virtual_input_indicator_elements(
        &self,
        output: &Output,
    ) -> impl Iterator<Item = SolidColorRenderElement> {
        let config = self.config.borrow().virtual_input_indicator;
        let tracker = &self.virtual_input;
        let show = tracker.active && !config.off;

        let size = output_size(output);
        let w = config.width.min(size.w / 2.).min(size.h / 2.);
        let rects = [
            Rectangle::new((0., 0.).into(), (size.w, w).into()),
            Rectangle::new((0., size.h - w).into(), (size.w, w).into()),
            Rectangle::new((0., w).into(), (w, size.h - w * 2.).into()),
            Rectangle::new((size.w - w, w).into(), (w, size.h - w * 2.).into()),
        ];

        let commit = tracker.indicator_commit;
        let color = Color32F::from(config.color);
        tracker
            .indicator_ids
            .clone()
            .into_iter()
            .zip(rects)
            .filter(move |_| show && w > 0.)
            .map(move |(id, rect)| {
                SolidColorRenderElement::new(id, rect, commit, color, Kind::Unspecified)
            })
    }
}

impl State {
    /// Stops all current synthetic input.
    ///
    /// Releases buttons, keys and touch points that synthetic input still holds. Virtual
    /// pointers become inert. Virtual keyboards cannot be made inert, so their clients are
    /// disconnected. RemoteDesktop sessions are closed.
    pub fn revoke_virtual_input(&mut self) {
        self.release_held_virtual_input();

        let niri = &mut self.niri;
        niri.virtual_pointer_state.revoke_all();

        for manager in niri.virtual_input.keyboard_managers.drain(..) {
            if manager.is_alive() {
                manager.post_error(
                    zwp_virtual_keyboard_manager_v1::Error::Unauthorized,
                    "virtual input was revoked",
                );
            }
        }

        #[cfg(feature = "xdp-gnome-screencast")]
        if let Some(dbus) = &niri.dbus {
            if let (Some(conn), Some(sessions)) =
                (&dbus.conn_remote_desktop, &dbus.remote_desktop_sessions)
            {
                crate::dbus::mutter_remote_desktop::close_sessions(conn, sessions);
            }
        }

        if let Some(token) = niri.virtual_input.idle_timer.take() {
            niri.event_loop.remove(token);
        }
        if niri.virtual_input.active {
            niri.virtual_input.active = false;
            niri.queue_redraw_all();
        }
    }

    pub fn toggle_virtual_input_blocked(&mut self) {
        let blocked = !self.niri.virtual_input.is_blocked();
        if blocked {
            self.revoke_virtual_input();
        }
        self.niri
            .virtual_input
            .blocked
            .store(blocked, Ordering::Relaxed);
    }

    fn release_held_virtual_input(&mut self) {
        let buttons = mem::take(&mut self.niri.virtual_input.held_buttons);
        if !buttons.is_empty() {
            let pointer = self.niri.seat.get_pointer().unwrap();
            let time = get_monotonic_time().as_millis() as u32;
            for button in buttons {
                // The press triggered a bind and never reached the pointer.
                if self.niri.suppressed_buttons.remove(&button) {
                    continue;
                }

                let event = ButtonEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                    button,
                    state: ButtonState::Released,
                };
                pointer.button(self, &event);
            }
            pointer.frame(self);
        }

        #[cfg(feature = "xdp-gnome-screencast")]
        {
            let tracker = &mut self.niri.virtual_input;
            let keys = mem::take(&mut tracker.held_keys);
            let touch_slots = mem::take(&mut tracker.held_touch_slots);
            crate::dbus::mutter_remote_desktop::release_held_input(self, keys, touch_slots);
        }
    }
}
