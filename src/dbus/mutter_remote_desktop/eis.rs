use std::os::unix::net::UnixStream;

use calloop::PostAction;
use reis::calloop::{EisRequestSource, EisRequestSourceEvent};
use reis::eis::device::DeviceType;
use reis::enumflags2::BitFlags;
use reis::request::{
    Bind, Connection, Device, DeviceCapability, DeviceClosed, EisRequest, RequestDevice,
};
use smithay::backend::input::{AxisSource, ButtonState, KeyState, Keycode};

use super::device_types::RemoteDesktopDeviceTypes;
use super::input::window::window_stream_size;
use super::input::{
    dispatch_to_niri, RemoteDesktopAbsolutePosition, RemoteDesktopAxisFrame, RemoteDesktopPosition,
};
use super::keymap::current_keymap_file;
use super::legacy_input::XKB_KEYCODE_OFFSET;
use super::registry::{
    RemoteDesktopOutputRegion, RemoteDesktopSessionRegistry, RemoteDesktopWindowRegion,
};
use crate::dbus::mutter_remote_desktop::RemoteDesktopToNiri;
use crate::niri::State;

#[derive(Debug)]
pub struct RemoteDesktopEisConnection {
    socket: UnixStream,
    session_id: String,
    registry: RemoteDesktopSessionRegistry,
    device_types: RemoteDesktopDeviceTypes,
}

#[derive(Debug, Clone, PartialEq)]
struct EisRegion {
    mapping_id: String,
    target: EisRegionTarget,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    scale: f64,
}

#[derive(Debug, Clone, PartialEq)]
enum EisRegionTarget {
    Output(String),
    Window(u64),
}

#[derive(Default)]
struct RemoteDesktopEisState {
    session_id: String,
    registry: RemoteDesktopSessionRegistry,
    device_types: RemoteDesktopDeviceTypes,
    regions: Vec<EisRegion>,
    window_regions: Vec<EisRegion>,
    /// Standalone absolute devices, one per window stream and capability, like Mutter.
    window_devices: Vec<(Device, EisRegion)>,
    seat: Option<reis::request::Seat>,
    keyboard: Option<Device>,
    pointer: Option<Device>,
    pointer_absolute: Option<Device>,
    touch: Option<Device>,
}

impl RemoteDesktopEisConnection {
    pub fn new(
        socket: UnixStream,
        session_id: String,
        registry: RemoteDesktopSessionRegistry,
        device_types: RemoteDesktopDeviceTypes,
    ) -> Self {
        Self {
            socket,
            session_id,
            registry,
            device_types,
        }
    }
}

pub fn connect_to_niri(
    state: &mut State,
    connection: RemoteDesktopEisConnection,
) -> Result<(), String> {
    let context = reis::eis::Context::new(connection.socket)
        .map_err(|err| format!("failed to create EIS context: {err:?}"))?;
    let source = EisRequestSource::new(context, 1);
    let mut eis_state = RemoteDesktopEisState {
        session_id: connection.session_id,
        registry: connection.registry,
        device_types: connection.device_types,
        regions: Vec::new(),
        window_regions: Vec::new(),
        window_devices: Vec::new(),
        seat: None,
        keyboard: None,
        pointer: None,
        pointer_absolute: None,
        touch: None,
    };

    state
        .niri
        .event_loop
        .insert_source(source, move |event, connection, state| {
            Ok(eis_state.handle_source_event(event, connection, state))
        })
        .map_err(|err| format!("failed to insert EIS source: {err:?}"))?;

    Ok(())
}

impl RemoteDesktopEisState {
    fn handle_source_event(
        &mut self,
        event: Result<EisRequestSourceEvent, reis::Error>,
        connection: &mut Connection,
        state: &mut State,
    ) -> PostAction {
        // The session was stopped, possibly by revoking virtual input.
        if !self.registry.contains_session(&self.session_id) {
            return PostAction::Remove;
        }

        let action = match event {
            Ok(EisRequestSourceEvent::Connected) => {
                self.handle_connected(connection, state);
                PostAction::Continue
            }
            Ok(EisRequestSourceEvent::Request(request)) => {
                self.handle_request(connection, state, request)
            }
            Err(err) => {
                warn!("error communicating with remote desktop EIS client: {err:?}");
                PostAction::Remove
            }
        };

        if let Err(err) = connection.flush() {
            warn!("error flushing remote desktop EIS connection: {err:?}");
            return PostAction::Remove;
        }

        action
    }

    fn handle_connected(&mut self, connection: &Connection, state: &State) {
        self.regions = match output_regions(&self.registry, &self.session_id) {
            Ok(regions) => regions,
            Err(err) => {
                warn!("error resolving remote desktop EIS output regions: {err:?}");
                Vec::new()
            }
        };
        self.window_regions = match self.registry.window_regions(&self.session_id) {
            Ok(regions) => window_regions(state, &regions),
            Err(err) => {
                warn!("error resolving remote desktop EIS window regions: {err:?}");
                Vec::new()
            }
        };
        self.seat = Some(connection.add_seat(Some("niri"), self.device_types.capabilities()));
    }

    fn handle_request(
        &mut self,
        connection: &Connection,
        state: &mut State,
        request: EisRequest,
    ) -> PostAction {
        match request {
            EisRequest::Disconnect => PostAction::Remove,
            EisRequest::Bind(request) => {
                self.handle_bind(connection, state, request);
                PostAction::Continue
            }
            EisRequest::RequestDevice(request) => {
                self.handle_request_device(connection, state, request);
                PostAction::Continue
            }
            EisRequest::DeviceClosed(request) => {
                self.handle_device_closed(request);
                PostAction::Continue
            }
            EisRequest::KeyboardKey(event) => {
                dispatch_to_niri(state, keyboard_key_event(event.key, event.state));
                PostAction::Continue
            }
            EisRequest::PointerMotion(event) => {
                dispatch_to_niri(
                    state,
                    RemoteDesktopToNiri::PointerMotionRelative {
                        dx: f64::from(event.dx),
                        dy: f64::from(event.dy),
                    },
                );
                PostAction::Continue
            }
            EisRequest::PointerMotionAbsolute(event) => {
                if let Some(position) = self.absolute_position(
                    &event.device,
                    f64::from(event.dx_absolute),
                    f64::from(event.dy_absolute),
                ) {
                    dispatch_to_niri(
                        state,
                        RemoteDesktopToNiri::PointerMotionAbsolute { position },
                    );
                }
                PostAction::Continue
            }
            EisRequest::Button(event) => {
                dispatch_to_niri(
                    state,
                    RemoteDesktopToNiri::PointerButton {
                        button: event.button,
                        state: button_state(event.state),
                    },
                );
                PostAction::Continue
            }
            EisRequest::ScrollDelta(event) => {
                dispatch_to_niri(
                    state,
                    RemoteDesktopToNiri::PointerAxis {
                        frame: smooth_scroll_frame(f64::from(event.dx), f64::from(event.dy)),
                    },
                );
                PostAction::Continue
            }
            EisRequest::ScrollStop(event) => {
                dispatch_to_niri(
                    state,
                    RemoteDesktopToNiri::PointerAxis {
                        frame: stop_scroll_frame(event.x, event.y),
                    },
                );
                PostAction::Continue
            }
            EisRequest::ScrollCancel(event) => {
                dispatch_to_niri(
                    state,
                    RemoteDesktopToNiri::PointerAxis {
                        frame: cancel_scroll_frame(event.x, event.y),
                    },
                );
                PostAction::Continue
            }
            EisRequest::ScrollDiscrete(event) => {
                dispatch_to_niri(
                    state,
                    RemoteDesktopToNiri::PointerAxis {
                        frame: discrete_scroll_frame(event.discrete_dx, event.discrete_dy),
                    },
                );
                PostAction::Continue
            }
            EisRequest::TouchDown(event) => {
                if let Some(position) =
                    self.absolute_position(&event.device, f64::from(event.x), f64::from(event.y))
                {
                    dispatch_to_niri(
                        state,
                        RemoteDesktopToNiri::TouchDown {
                            slot: event.touch_id,
                            position,
                        },
                    );
                }
                PostAction::Continue
            }
            EisRequest::TouchMotion(event) => {
                if let Some(position) =
                    self.absolute_position(&event.device, f64::from(event.x), f64::from(event.y))
                {
                    dispatch_to_niri(
                        state,
                        RemoteDesktopToNiri::TouchMotion {
                            slot: event.touch_id,
                            position,
                        },
                    );
                }
                PostAction::Continue
            }
            EisRequest::TouchUp(event) => {
                dispatch_to_niri(
                    state,
                    RemoteDesktopToNiri::TouchUp {
                        slot: event.touch_id,
                    },
                );
                PostAction::Continue
            }
            EisRequest::TouchCancel(event) => {
                dispatch_to_niri(
                    state,
                    RemoteDesktopToNiri::TouchUp {
                        slot: event.touch_id,
                    },
                );
                PostAction::Continue
            }
            EisRequest::Frame(_)
            | EisRequest::Ready(_)
            | EisRequest::DeviceStartEmulating(_)
            | EisRequest::DeviceStopEmulating(_)
            | EisRequest::TextKeysym(_)
            | EisRequest::TextUtf8(_) => PostAction::Continue,
        }
    }

    fn handle_bind(&mut self, connection: &Connection, state: &State, request: Bind) {
        if self.seat.is_none() {
            self.seat = Some(request.seat.clone());
        }
        self.add_requested_devices(connection, state, &request.seat, request.capabilities);
    }

    fn handle_request_device(
        &mut self,
        connection: &Connection,
        state: &State,
        request: RequestDevice,
    ) {
        self.add_requested_devices(connection, state, &request.seat, request.capabilities);
    }

    fn add_requested_devices(
        &mut self,
        connection: &Connection,
        state: &State,
        seat: &reis::request::Seat,
        capabilities: BitFlags<DeviceCapability>,
    ) {
        if self.keyboard.is_none()
            && self.device_types.keyboard()
            && capabilities.contains(DeviceCapability::Keyboard)
        {
            self.keyboard = self.add_device(
                connection,
                state,
                seat,
                "niri remote desktop keyboard",
                DeviceCapability::Keyboard.into(),
                &[],
            );
        }

        if self.pointer.is_none()
            && self.device_types.pointer()
            && capabilities.contains(DeviceCapability::Pointer)
        {
            self.pointer = self.add_device(
                connection,
                state,
                seat,
                "niri remote desktop pointer",
                pointer_capabilities(capabilities),
                &[],
            );
        }

        if self.pointer_absolute.is_none()
            && self.device_types.pointer()
            && capabilities.contains(DeviceCapability::PointerAbsolute)
        {
            self.pointer_absolute = self.add_device(
                connection,
                state,
                seat,
                "niri remote desktop absolute pointer",
                pointer_absolute_capabilities(capabilities),
                &self.regions,
            );
        }

        if self.device_types.pointer() && capabilities.contains(DeviceCapability::PointerAbsolute) {
            self.add_window_devices(
                connection,
                state,
                seat,
                "niri remote desktop window absolute pointer",
                pointer_absolute_capabilities(capabilities),
            );
        }

        if self.touch.is_none()
            && self.device_types.touchscreen()
            && capabilities.contains(DeviceCapability::Touch)
        {
            self.touch = self.add_device(
                connection,
                state,
                seat,
                "niri remote desktop touch",
                DeviceCapability::Touch.into(),
                &self.regions,
            );
        }

        if self.device_types.touchscreen() && capabilities.contains(DeviceCapability::Touch) {
            self.add_window_devices(
                connection,
                state,
                seat,
                "niri remote desktop window touch",
                DeviceCapability::Touch.into(),
            );
        }
    }

    fn add_window_devices(
        &mut self,
        connection: &Connection,
        state: &State,
        seat: &reis::request::Seat,
        name: &str,
        capabilities: BitFlags<DeviceCapability>,
    ) {
        for region in self.window_regions.clone() {
            let exists = self.window_devices.iter().any(|(device, existing)| {
                *existing == region
                    && capabilities
                        .iter()
                        .all(|capability| device.has_capability(capability))
            });
            if exists {
                continue;
            }

            let device = self.add_device(
                connection,
                state,
                seat,
                name,
                capabilities,
                std::slice::from_ref(&region),
            );
            if let Some(device) = device {
                self.window_devices.push((device, region));
            }
        }
    }

    fn add_device(
        &self,
        connection: &Connection,
        state: &State,
        seat: &reis::request::Seat,
        name: &str,
        capabilities: BitFlags<DeviceCapability>,
        regions: &[EisRegion],
    ) -> Option<Device> {
        if capabilities.is_empty() {
            return None;
        }

        let regions = if capabilities.contains(DeviceCapability::PointerAbsolute)
            || capabilities.contains(DeviceCapability::Touch)
        {
            regions
        } else {
            &[]
        };
        let keymap_file = if capabilities.contains(DeviceCapability::Keyboard) {
            match current_keymap_file(state) {
                Ok(file) => Some(file),
                Err(err) => {
                    warn!("error compiling remote desktop EIS keymap: {err}");
                    None
                }
            }
        } else {
            None
        };

        let device = seat.add_device(Some(name), DeviceType::Virtual, capabilities, |device| {
            if let Some(keymap_file) = keymap_file.as_ref() {
                if let Some(keyboard) = device.interface::<reis::eis::Keyboard>() {
                    if let Err(err) = keymap_file.with_fd(true, |fd, len| {
                        keyboard.keymap(reis::eis::keyboard::KeymapType::Xkb, len as u32, fd);
                    }) {
                        warn!("error creating remote desktop EIS keymap fd: {err:?}");
                    }
                }
            }

            for region in regions {
                device.device().region_mapping_id(&region.mapping_id);
                device.device().region(
                    region.x,
                    region.y,
                    region.width,
                    region.height,
                    region.scale as f32,
                );
            }
        });
        device.resumed();
        if connection.context_type() == reis::eis::handshake::ContextType::Receiver {
            device.start_emulating(1);
        }

        Some(device)
    }

    fn handle_device_closed(&mut self, request: DeviceClosed) {
        request.device.remove();
        clear_matching_device(&mut self.keyboard, &request.device);
        clear_matching_device(&mut self.pointer, &request.device);
        clear_matching_device(&mut self.pointer_absolute, &request.device);
        clear_matching_device(&mut self.touch, &request.device);
        self.window_devices
            .retain(|(device, _)| *device != request.device);
    }

    fn absolute_position(
        &self,
        device: &Device,
        x: f64,
        y: f64,
    ) -> Option<RemoteDesktopAbsolutePosition> {
        if !x.is_finite() || !y.is_finite() {
            warn!("remote desktop EIS absolute coordinates must be finite");
            return None;
        }

        if let Some((_, region)) = self.window_devices.iter().find(|(d, _)| d == device) {
            return Some(region.position(x, y));
        }

        let region = self.regions.iter().find(|region| {
            let region_x = f64::from(region.x);
            let region_y = f64::from(region.y);
            x >= region_x
                && y >= region_y
                && x < region_x + f64::from(region.width)
                && y < region_y + f64::from(region.height)
        });
        let Some(region) = region else {
            warn!("remote desktop EIS absolute coordinates did not match any output region");
            return None;
        };

        Some(region.position(x, y))
    }
}

impl EisRegion {
    fn position(&self, x: f64, y: f64) -> RemoteDesktopAbsolutePosition {
        let x = x - f64::from(self.x);
        let y = y - f64::from(self.y);
        match &self.target {
            EisRegionTarget::Output(output_name) => {
                RemoteDesktopAbsolutePosition::Output(RemoteDesktopPosition {
                    output_name: output_name.clone(),
                    x,
                    y,
                    width: i32::try_from(self.width).unwrap_or(i32::MAX),
                    height: i32::try_from(self.height).unwrap_or(i32::MAX),
                })
            }
            // Window regions are logical; window stream positions are buffer pixels.
            EisRegionTarget::Window(id) => RemoteDesktopAbsolutePosition::Window {
                id: *id,
                x: x * self.scale,
                y: y * self.scale,
            },
        }
    }
}

fn output_regions(
    registry: &RemoteDesktopSessionRegistry,
    session_id: &str,
) -> Result<Vec<EisRegion>, String> {
    let regions = registry
        .output_regions(session_id)
        .map_err(|err| format!("{err:?}"))?;
    normalize_regions(&regions)
}

fn normalize_regions(regions: &[RemoteDesktopOutputRegion]) -> Result<Vec<EisRegion>, String> {
    let min_x = regions.iter().map(|region| region.x).min().unwrap_or(0);
    let min_y = regions.iter().map(|region| region.y).min().unwrap_or(0);

    regions
        .iter()
        .map(|region| normalize_region(region, min_x, min_y))
        .collect()
}

fn normalize_region(
    region: &RemoteDesktopOutputRegion,
    min_x: i32,
    min_y: i32,
) -> Result<EisRegion, String> {
    let width = u32::try_from(region.width)
        .map_err(|_| format!("output {} has a negative width", region.output_name))?;
    let height = u32::try_from(region.height)
        .map_err(|_| format!("output {} has a negative height", region.output_name))?;
    if width == 0 || height == 0 {
        return Err(format!("output {} has an empty region", region.output_name));
    }

    Ok(EisRegion {
        mapping_id: region.mapping_id.to_string(),
        target: EisRegionTarget::Output(region.output_name.clone()),
        x: u32::try_from(region.x - min_x)
            .map_err(|_| format!("output {} has an invalid x offset", region.output_name))?,
        y: u32::try_from(region.y - min_y)
            .map_err(|_| format!("output {} has an invalid y offset", region.output_name))?,
        width,
        height,
        scale: 1.,
    })
}

/// Window stream regions in logical pixels at the window's current output scale.
///
/// Like output regions, they are not updated when the window resizes; positions are clamped to
/// the window when they are resolved.
fn window_regions(state: &State, regions: &[RemoteDesktopWindowRegion]) -> Vec<EisRegion> {
    regions
        .iter()
        .filter_map(|region| {
            let Some((monitor, mapped)) = state
                .niri
                .layout
                .windows()
                .find(|(_, mapped)| mapped.id().get() == region.window_id)
            else {
                warn!(
                    "remote desktop EIS window stream {} has no window",
                    region.mapping_id
                );
                return None;
            };
            let scale = monitor.map_or(1., |monitor| {
                monitor.output().current_scale().fractional_scale()
            });
            let size = window_stream_size(&mapped.window, scale)
                .to_f64()
                .to_logical(scale);
            Some(window_region(region, size.w, size.h, scale))
        })
        .collect()
}

fn window_region(
    region: &RemoteDesktopWindowRegion,
    width: f64,
    height: f64,
    scale: f64,
) -> EisRegion {
    EisRegion {
        mapping_id: region.mapping_id.to_string(),
        target: EisRegionTarget::Window(region.window_id),
        x: 0,
        y: 0,
        width: width.ceil().max(1.) as u32,
        height: height.ceil().max(1.) as u32,
        scale,
    }
}

fn pointer_capabilities(requested: BitFlags<DeviceCapability>) -> BitFlags<DeviceCapability> {
    let mut capabilities = DeviceCapability::Pointer.into();
    if requested.contains(DeviceCapability::Button) {
        capabilities |= DeviceCapability::Button;
    }
    if requested.contains(DeviceCapability::Scroll) {
        capabilities |= DeviceCapability::Scroll;
    }
    capabilities
}

fn pointer_absolute_capabilities(
    requested: BitFlags<DeviceCapability>,
) -> BitFlags<DeviceCapability> {
    let mut capabilities = DeviceCapability::PointerAbsolute.into();
    if requested.contains(DeviceCapability::Button) {
        capabilities |= DeviceCapability::Button;
    }
    if requested.contains(DeviceCapability::Scroll) {
        capabilities |= DeviceCapability::Scroll;
    }
    capabilities
}

fn clear_matching_device(slot: &mut Option<Device>, device: &Device) {
    if slot.as_ref() == Some(device) {
        *slot = None;
    }
}

fn keyboard_key_event(key: u32, state: reis::eis::keyboard::KeyState) -> RemoteDesktopToNiri {
    RemoteDesktopToNiri::KeyboardKeycode {
        keycode: Keycode::from(key.saturating_add(XKB_KEYCODE_OFFSET)),
        state: key_state(state),
    }
}

fn key_state(state: reis::eis::keyboard::KeyState) -> KeyState {
    match state {
        reis::eis::keyboard::KeyState::Released => KeyState::Released,
        reis::eis::keyboard::KeyState::Press => KeyState::Pressed,
    }
}

fn button_state(state: reis::eis::button::ButtonState) -> ButtonState {
    match state {
        reis::eis::button::ButtonState::Released => ButtonState::Released,
        reis::eis::button::ButtonState::Press => ButtonState::Pressed,
    }
}

fn smooth_scroll_frame(dx: f64, dy: f64) -> RemoteDesktopAxisFrame {
    RemoteDesktopAxisFrame {
        smooth: Some((dx, dy)),
        v120: None,
        source: AxisSource::Wheel,
        stop: (false, false),
    }
}

fn stop_scroll_frame(x: bool, y: bool) -> RemoteDesktopAxisFrame {
    RemoteDesktopAxisFrame {
        smooth: Some((0.0, 0.0)),
        v120: None,
        source: AxisSource::Wheel,
        stop: (x, y),
    }
}

fn cancel_scroll_frame(x: bool, y: bool) -> RemoteDesktopAxisFrame {
    RemoteDesktopAxisFrame {
        smooth: Some((if x { 0.01 } else { 0.0 }, if y { 0.01 } else { 0.0 })),
        v120: None,
        source: AxisSource::Wheel,
        stop: (x, y),
    }
}

fn discrete_scroll_frame(dx: i32, dy: i32) -> RemoteDesktopAxisFrame {
    RemoteDesktopAxisFrame {
        smooth: None,
        v120: Some((f64::from(dx), f64::from(dy))),
        source: AxisSource::Wheel,
        stop: (false, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eis_regions_are_normalized_to_positive_coordinates() {
        let regions = vec![
            RemoteDesktopOutputRegion {
                mapping_id: 11,
                output_name: "HDMI-A-1".to_owned(),
                x: -1920,
                y: 120,
                width: 1920,
                height: 1080,
            },
            RemoteDesktopOutputRegion {
                mapping_id: 12,
                output_name: "DP-1".to_owned(),
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
            },
        ];

        let normalized = normalize_regions(&regions).unwrap();

        assert_eq!(
            normalized,
            vec![
                EisRegion {
                    mapping_id: "11".to_owned(),
                    target: EisRegionTarget::Output("HDMI-A-1".to_owned()),
                    x: 0,
                    y: 120,
                    width: 1920,
                    height: 1080,
                    scale: 1.,
                },
                EisRegion {
                    mapping_id: "12".to_owned(),
                    target: EisRegionTarget::Output("DP-1".to_owned()),
                    x: 1920,
                    y: 0,
                    width: 2560,
                    height: 1440,
                    scale: 1.,
                },
            ]
        );
    }

    #[test]
    fn eis_window_regions_map_logical_coordinates_to_stream_pixels() {
        let region = window_region(
            &RemoteDesktopWindowRegion {
                mapping_id: 13,
                window_id: 42,
            },
            400.5,
            300.,
            1.5,
        );

        assert_eq!((region.x, region.y), (0, 0));
        assert_eq!((region.width, region.height), (401, 300));
        assert_eq!(region.mapping_id, "13");
        assert!(matches!(
            region.position(100., 20.),
            RemoteDesktopAbsolutePosition::Window {
                id: 42,
                x: 150.,
                y: 30.
            }
        ));
    }

    #[test]
    fn eis_output_regions_map_to_output_local_positions() {
        let regions = normalize_regions(&[RemoteDesktopOutputRegion {
            mapping_id: 11,
            output_name: "HDMI-A-1".to_owned(),
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        }])
        .unwrap();

        let RemoteDesktopAbsolutePosition::Output(position) = regions[0].position(10., 20.) else {
            panic!("expected an output position");
        };
        assert_eq!(position.output_name, "HDMI-A-1");
        assert_eq!((position.x, position.y), (10., 20.));
        assert_eq!((position.width, position.height), (1920, 1080));
    }
}
