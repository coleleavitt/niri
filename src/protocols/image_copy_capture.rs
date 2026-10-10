//! Session bookkeeping for `ext-image-copy-capture-v1`.
//!
//! Smithay implements the protocol objects; this module tracks niri's per-session state: what is
//! being captured, the damage tracker, and the frame waiting for new damage.

use std::time::Duration;

use calloop::generic::Generic;
use calloop::{Interest, LoopHandle, Mode, PostAction};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer as _, Fourcc};
use smithay::backend::egl::EGLDevice;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::sync::SyncPoint;
use smithay::output::{Output, WeakOutput};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_protocols::ext::image_capture_source::v1::server::ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1;
use smithay::reexports::wayland_server::{Client, DisplayHandle, GlobalDispatch};
use smithay::utils::{Buffer, Physical, Rectangle, Scale, Size, Transform};
use smithay::wayland::image_capture_source::{OutputCaptureSourceHandler, OutputCaptureSourceState};
use smithay::wayland::image_copy_capture::{
    DmabufConstraints, Frame, FrameRef, ImageCopyCaptureHandler, ImageCopyCaptureState,
    Session, SessionRef,
};
use smithay::wayland::{dmabuf, shm};

use crate::protocols::image_capture_source::{
    self, ToplevelImageCaptureSourceGlobalData, ToplevelImageCaptureSourceHandler,
};
use crate::window::mapped::MappedId;

/// What an image capture source refers to, stored in the source's user data.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageCaptureTarget {
    Output(WeakOutput),
    Window(MappedId),
}

/// Where and how an image copy capture target is rendered right now.
pub struct ImageCaptureParams {
    pub output: Output,
    /// Buffer size, before the transform.
    pub size: Size<i32, Physical>,
    pub scale: Scale<f64>,
    pub transform: Transform,
}

pub struct ImageCopyCaptureSession {
    // Must be dropped before pending_frame so that the frame fails with "stopped".
    pub session: Session,
    pub target: ImageCaptureTarget,
    pub damage_tracker: OutputDamageTracker,
    /// Frame waiting for damage on the next redraw of the target's output.
    pub pending_frame: Option<Frame>,
    /// Whether this session has delivered a frame yet.
    pub has_captured: bool,
}

pub struct ImageCopyCaptureManagerState {
    pub protocol: ImageCopyCaptureState,
    pub output_source: OutputCaptureSourceState,
    pub sessions: Vec<ImageCopyCaptureSession>,
}

pub enum CaptureBuffer {
    Shm(WlBuffer),
    Dmabuf(Dmabuf),
}

impl ImageCopyCaptureManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: ImageCopyCaptureHandler,
        D: OutputCaptureSourceHandler,
        D: ToplevelImageCaptureSourceHandler,
        D: GlobalDispatch<
            ExtForeignToplevelImageCaptureSourceManagerV1,
            ToplevelImageCaptureSourceGlobalData,
        >,
        F: Fn(&Client) -> bool + Copy + Send + Sync + 'static,
    {
        image_capture_source::create_global::<D, _>(display, filter);
        Self {
            protocol: ImageCopyCaptureState::new_with_filter::<D, _>(display, filter),
            output_source: OutputCaptureSourceState::new_with_filter::<D, _>(display, filter),
            sessions: Vec::new(),
        }
    }

    pub fn add_session(&mut self, session: Session, target: ImageCaptureTarget) {
        self.sessions.push(ImageCopyCaptureSession {
            session,
            target,
            damage_tracker: OutputDamageTracker::new((0, 0), 1., Transform::Normal),
            pending_frame: None,
            has_captured: false,
        });
    }

    pub fn session_mut(&mut self, session: &SessionRef) -> Option<&mut ImageCopyCaptureSession> {
        self.sessions.iter_mut().find(|s| s.session == *session)
    }

    pub fn remove_session(&mut self, session: &SessionRef) {
        self.sessions.retain(|s| s.session != *session);
        self.protocol.cleanup();
    }

    pub fn remove_frame(&mut self, frame: &FrameRef) {
        for session in &mut self.sessions {
            if session.pending_frame.as_ref().is_some_and(|f| f == frame) {
                session.pending_frame = None;
            }
        }
    }
}

impl CaptureBuffer {
    /// Returns the frame's buffer if it can be rendered into at exactly this size.
    pub fn new(buffer: WlBuffer, size: Size<i32, Physical>) -> Option<Self> {
        if let Ok(dmabuf) = dmabuf::get_dmabuf(&buffer) {
            let matches = dmabuf.size() == size.to_logical(1).to_buffer(1, Transform::Normal);
            return matches.then(|| Self::Dmabuf(dmabuf.clone()));
        }

        let matches = shm::with_buffer_contents(&buffer, |_, len, data| {
            data.width == size.w
                && data.height == size.h
                && data.stride == size.w * 4
                && len == data.stride as usize * data.height as usize
        })
        .unwrap_or(false);
        matches.then_some(Self::Shm(buffer))
    }
}

/// Dmabuf constraints for rendering with the given renderer, if it has a render node.
pub fn image_capture_dmabuf_constraints(renderer: &mut GlesRenderer) -> Option<DmabufConstraints> {
    let egl = renderer.egl_context();
    let node = EGLDevice::device_for_display(egl.display())
        .ok()?
        .try_get_render_node()
        .ok()??;

    let render_formats = egl.dmabuf_render_formats();
    let formats: Vec<_> = [Fourcc::Xrgb8888, Fourcc::Argb8888]
        .into_iter()
        .map(|code| {
            let modifiers = render_formats
                .iter()
                .filter(|format| format.code == code)
                .map(|format| format.modifier)
                .collect::<Vec<_>>();
            (code, modifiers)
        })
        .filter(|(_, modifiers)| !modifiers.is_empty())
        .collect();

    (!formats.is_empty()).then_some(DmabufConstraints { node, formats })
}

/// Converts output damage from the damage tracker into buffer coordinates.
pub fn damage_to_buffer(
    damage: &[Rectangle<i32, Physical>],
    size: Size<i32, Physical>,
    transform: Transform,
) -> Vec<Rectangle<i32, Buffer>> {
    let physical_size = transform.transform_size(size).to_logical(1);
    damage
        .iter()
        .map(|dmg| {
            dmg.to_logical(1)
                .to_buffer(1, transform.invert(), &physical_size)
        })
        .collect()
}

pub fn submit_after_sync<T>(
    frame: Frame,
    transform: Transform,
    damage: Vec<Rectangle<i32, Buffer>>,
    presented: Duration,
    sync_point: Option<SyncPoint>,
    event_loop: &LoopHandle<'_, T>,
) {
    match sync_point.and_then(|s| s.export()) {
        None => frame.success(transform, damage, presented),
        Some(sync_fd) => {
            let source = Generic::new(sync_fd, Interest::READ, Mode::OneShot);
            let mut frame = Some((frame, damage));
            event_loop
                .insert_source(source, move |_, _, _| {
                    let (frame, damage) = frame.take().unwrap();
                    frame.success(transform, damage, presented);
                    Ok(PostAction::Remove)
                })
                .unwrap();
        }
    }
}
