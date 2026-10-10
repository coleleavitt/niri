use smithay::input::SeatHandler;
use smithay::reexports::wayland_protocols_misc::zwp_virtual_keyboard_v1::server::{
    zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1,
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::wayland::virtual_keyboard::VirtualKeyboardUserData;
use smithay::wayland::{Dispatch2, GlobalData, GlobalDispatch2};
use zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

const VERSION: u32 = 1;

/// Wraps Smithay's virtual keyboard manager to observe and refuse keyboard creation.
///
/// Smithay delivers virtual keyboard events straight to clients, so creation is the only point
/// where niri can see or stop a virtual keyboard.
pub struct VirtualKeyboardManagerState;

pub struct VirtualKeyboardManagerGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub struct VirtualKeyboardManagerData;

/// User data of a virtual keyboard created while synthetic input is blocked; ignores requests.
pub struct BlockedVirtualKeyboard;

pub trait VirtualKeyboardHandler {
    fn virtual_keyboards_blocked(&self) -> bool;
    fn virtual_keyboard_created(&mut self, manager: &ZwpVirtualKeyboardManagerV1);
}

impl VirtualKeyboardManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ZwpVirtualKeyboardManagerV1, VirtualKeyboardManagerGlobalData>,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = VirtualKeyboardManagerGlobalData {
            filter: Box::new(filter),
        };
        display.create_global::<D, ZwpVirtualKeyboardManagerV1, _>(VERSION, global_data);
        Self
    }
}

impl<D> GlobalDispatch2<ZwpVirtualKeyboardManagerV1, D> for VirtualKeyboardManagerGlobalData
where
    D: Dispatch<ZwpVirtualKeyboardManagerV1, VirtualKeyboardManagerData>,
    D: 'static,
{
    fn bind(
        &self,
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        manager: New<ZwpVirtualKeyboardManagerV1>,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, VirtualKeyboardManagerData);
    }

    fn can_view(&self, client: &Client) -> bool {
        (self.filter)(client)
    }
}

impl<D> Dispatch2<ZwpVirtualKeyboardManagerV1, D> for VirtualKeyboardManagerData
where
    D: Dispatch<ZwpVirtualKeyboardV1, VirtualKeyboardUserData<D>>,
    D: Dispatch<ZwpVirtualKeyboardV1, BlockedVirtualKeyboard>,
    D: SeatHandler + VirtualKeyboardHandler,
    D: 'static,
{
    fn request(
        &self,
        state: &mut D,
        client: &Client,
        resource: &ZwpVirtualKeyboardManagerV1,
        request: <ZwpVirtualKeyboardManagerV1 as Resource>::Request,
        dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwp_virtual_keyboard_manager_v1::Request::CreateVirtualKeyboard { id, .. }
                if state.virtual_keyboards_blocked() =>
            {
                data_init.init(id, BlockedVirtualKeyboard);
            }
            request => {
                state.virtual_keyboard_created(resource);
                GlobalData.request(state, client, resource, request, dhandle, data_init);
            }
        }
    }
}

impl<D> Dispatch2<ZwpVirtualKeyboardV1, D> for BlockedVirtualKeyboard {
    fn request(
        &self,
        _state: &mut D,
        _client: &Client,
        _resource: &ZwpVirtualKeyboardV1,
        _request: <ZwpVirtualKeyboardV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
    }
}
