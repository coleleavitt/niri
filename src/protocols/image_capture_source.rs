//! Foreign toplevel source manager for `ext-image-capture-source-v1`.
//!
//! Smithay's toplevel source manager only understands Smithay's own `ext-foreign-toplevel-list`
//! handles, while niri implements that protocol itself, so this global resolves niri's handles.

use std::sync::Arc;

use smithay::reexports::wayland_protocols::ext::foreign_toplevel_list::v1::server::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;
use smithay::reexports::wayland_protocols::ext::image_capture_source::v1::server::ext_foreign_toplevel_image_capture_source_manager_v1::{
    self, ExtForeignToplevelImageCaptureSourceManagerV1,
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
};
use smithay::wayland::image_capture_source::{
    ImageCaptureSource, ImageCaptureSourceData, ImageCaptureSourceHandler,
};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::protocols::EmptyData;

const VERSION: u32 = 1;

pub struct ToplevelImageCaptureSourceGlobalData {
    filter: Arc<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub trait ToplevelImageCaptureSourceHandler: ImageCaptureSourceHandler {
    /// Called when a client creates a capture source from a foreign toplevel handle.
    ///
    /// The handle may be inert (its window already closed); the source then stays unresolved and
    /// sessions created from it are stopped right away.
    fn toplevel_source_created(
        &mut self,
        source: ImageCaptureSource,
        toplevel: &ExtForeignToplevelHandleV1,
    );
}

pub fn create_global<D, F>(display: &DisplayHandle, filter: F)
where
    D: GlobalDispatch<
        ExtForeignToplevelImageCaptureSourceManagerV1,
        ToplevelImageCaptureSourceGlobalData,
    >,
    D: 'static,
    F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
{
    let global_data = ToplevelImageCaptureSourceGlobalData {
        filter: Arc::new(filter),
    };
    display
        .create_global::<D, ExtForeignToplevelImageCaptureSourceManagerV1, _>(VERSION, global_data);
}

impl<D> GlobalDispatch2<ExtForeignToplevelImageCaptureSourceManagerV1, D>
    for ToplevelImageCaptureSourceGlobalData
where
    D: Dispatch<ExtForeignToplevelImageCaptureSourceManagerV1, EmptyData>,
    D: 'static,
{
    fn bind(
        &self,
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        manager: New<ExtForeignToplevelImageCaptureSourceManagerV1>,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, EmptyData);
    }

    fn can_view(&self, client: &Client) -> bool {
        (self.filter)(client)
    }
}

impl<D> Dispatch2<ExtForeignToplevelImageCaptureSourceManagerV1, D> for EmptyData
where
    D: ToplevelImageCaptureSourceHandler,
    D: 'static,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        _manager: &ExtForeignToplevelImageCaptureSourceManagerV1,
        request: ext_foreign_toplevel_image_capture_source_manager_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            ext_foreign_toplevel_image_capture_source_manager_v1::Request::CreateSource {
                source,
                toplevel_handle,
            } => {
                let capture_source = ImageCaptureSource::new();
                let resource = data_init.init(
                    source,
                    ImageCaptureSourceData {
                        source: capture_source.clone(),
                    },
                );
                capture_source.add_instance(&resource);
                state.toplevel_source_created(capture_source, &toplevel_handle);
            }
            ext_foreign_toplevel_image_capture_source_manager_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }
}
