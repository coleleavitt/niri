use smithay::reexports::wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1;
use smithay::reexports::wayland_protocols::ext::image_capture_source::v1::client::ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1;
use smithay::reexports::wayland_protocols::ext::image_capture_source::v1::client::ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1;
use smithay::reexports::wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_frame_v1::FailureReason;
use smithay::reexports::wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1;
use wayland_client::protocol::wl_shm;
use wayland_client::protocol::wl_surface::WlSurface;

use super::client::ClientId;
use super::*;

fn set_up_window(f: &mut Fixture, id: ClientId) -> WlSurface {
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(100, 200);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    surface
}

fn window_identifier(f: &mut Fixture) -> String {
    let mapped = f.niri().layout.windows().next().unwrap().1;
    mapped.id().get().to_string()
}

#[test]
fn output_session_sends_constraints() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    f.roundtrip(id);

    let client = f.client(id);
    let manager = client.bind::<ExtImageCopyCaptureManagerV1>();
    let sources = client.bind::<ExtOutputImageCaptureSourceManagerV1>();
    let output = client.output("headless-1");
    let source = sources.create_source(&output, &client.qh, ());
    let (_session, events) = client.create_capture_session(&manager, &source);
    f.roundtrip(id);

    let events = events.lock().unwrap();
    assert_eq!(events.buffer_size, Some((1920, 1080)));
    assert_eq!(
        events.shm_formats,
        [wl_shm::Format::Xrgb8888, wl_shm::Format::Argb8888]
    );
    // The headless backend has no renderer, so there is no dmabuf device.
    assert!(!events.dmabuf_device);
    assert_eq!(events.done, 1);
    assert!(!events.stopped);
}

#[test]
fn output_session_stops_when_output_is_removed() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));
    let id = f.add_client();
    f.roundtrip(id);

    let client = f.client(id);
    let manager = client.bind::<ExtImageCopyCaptureManagerV1>();
    let sources = client.bind::<ExtOutputImageCaptureSourceManagerV1>();
    let output = client.output("headless-2");
    let source = sources.create_source(&output, &client.qh, ());
    let (_session, events) = client.create_capture_session(&manager, &source);
    f.roundtrip(id);
    assert_eq!(events.lock().unwrap().buffer_size, Some((1280, 720)));

    let output = f.niri_output(2);
    f.niri().remove_output(&output);
    f.double_roundtrip(id);

    assert!(events.lock().unwrap().stopped);
    assert!(f.niri().image_copy_capture_state.sessions.is_empty());
}

#[test]
fn toplevel_session_follows_window() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    f.client(id).bind::<ExtForeignToplevelListV1>();
    let surface = set_up_window(&mut f, id);
    f.double_roundtrip(id);
    let identifier = window_identifier(&mut f);

    let client = f.client(id);
    let manager = client.bind::<ExtImageCopyCaptureManagerV1>();
    let sources = client.bind::<ExtForeignToplevelImageCaptureSourceManagerV1>();
    let handle = client.foreign_toplevel(&identifier).handle.clone();
    let source = sources.create_source(&handle, &client.qh, ());
    let (_session, events) = client.create_capture_session(&manager, &source);
    f.roundtrip(id);

    {
        let events = events.lock().unwrap();
        assert_eq!(events.buffer_size, Some((100, 200)));
        assert_eq!(events.done, 1);
        assert!(!events.stopped);
    }

    // Resizing the window sends new constraints.
    let window = f.client(id).window(&surface);
    window.set_size(150, 250);
    window.commit();
    f.double_roundtrip(id);

    {
        let events = events.lock().unwrap();
        assert_eq!(events.buffer_size, Some((150, 250)));
        assert_eq!(events.done, 2);
    }

    // Closing the window stops the session.
    let window = f.client(id).window(&surface);
    window.attach_null();
    window.commit();
    f.double_roundtrip(id);

    assert!(events.lock().unwrap().stopped);
    assert!(f.niri().image_copy_capture_state.sessions.is_empty());
}

#[test]
fn toplevel_session_from_closed_handle_is_stopped() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    f.client(id).bind::<ExtForeignToplevelListV1>();
    let surface = set_up_window(&mut f, id);
    f.double_roundtrip(id);
    let identifier = window_identifier(&mut f);

    let window = f.client(id).window(&surface);
    window.attach_null();
    window.commit();
    f.double_roundtrip(id);
    assert!(f.client(id).foreign_toplevel(&identifier).closed);

    let client = f.client(id);
    let manager = client.bind::<ExtImageCopyCaptureManagerV1>();
    let sources = client.bind::<ExtForeignToplevelImageCaptureSourceManagerV1>();
    let handle = client.foreign_toplevel(&identifier).handle.clone();
    let source = sources.create_source(&handle, &client.qh, ());
    let (_session, events) = client.create_capture_session(&manager, &source);
    f.roundtrip(id);

    let events = events.lock().unwrap();
    assert!(events.stopped);
    assert_eq!(events.buffer_size, None);
}

#[test]
fn frame_with_non_shm_buffer_fails() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    f.roundtrip(id);

    let client = f.client(id);
    let manager = client.bind::<ExtImageCopyCaptureManagerV1>();
    let sources = client.bind::<ExtOutputImageCaptureSourceManagerV1>();
    let output = client.output("headless-1");
    let source = sources.create_source(&output, &client.qh, ());
    let (session, _) = client.create_capture_session(&manager, &source);
    let (frame, events) = client.create_capture_frame(&session);
    let spbm = client.state.spbm.clone().unwrap();
    let buffer = spbm.create_u32_rgba_buffer(0, 0, 0, 0, &client.qh, ());
    frame.attach_buffer(&buffer);
    frame.capture();
    f.double_roundtrip(id);

    let events = events.lock().unwrap();
    assert_eq!(events.failed, Some(FailureReason::BufferConstraints));
    assert!(!events.ready);
}

#[test]
fn restricted_clients_cannot_capture() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let restricted = f.add_restricted_client();

    let client = f.client(id);
    assert!(client.has_global::<ExtImageCopyCaptureManagerV1>());
    assert!(client.has_global::<ExtOutputImageCaptureSourceManagerV1>());
    assert!(client.has_global::<ExtForeignToplevelImageCaptureSourceManagerV1>());

    let client = f.client(restricted);
    assert!(!client.has_global::<ExtImageCopyCaptureManagerV1>());
    assert!(!client.has_global::<ExtOutputImageCaptureSourceManagerV1>());
    assert!(!client.has_global::<ExtForeignToplevelImageCaptureSourceManagerV1>());
}
