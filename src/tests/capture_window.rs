use std::path::Path;

use smithay::output::Scale;
use smithay::wayland::selection::data_device::current_data_device_selection_userdata;

use super::*;

fn png_size(path: &Path) -> (u32, u32) {
    let data = std::fs::read(path).unwrap();
    assert_eq!(&data[..8], b"\x89PNG\r\n\x1a\n");
    let w = u32::from_be_bytes(data[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(data[20..24].try_into().unwrap());
    (w, h)
}

fn capture(
    f: &mut Fixture,
    id: Option<u64>,
    path: &Path,
) -> Result<niri_ipc::WindowCapture, String> {
    let (tx, rx) = async_channel::bounded(1);
    let path = path.to_str().unwrap().to_owned();
    f.niri_state().capture_window(id, path, false, tx)?;
    rx.recv_blocking().unwrap()
}

#[test]
fn egl_capture_window_writes_png_without_clipboard() {
    let mut f = Fixture::new();
    f.niri_state().backend.headless().add_renderer().unwrap();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1);
    output.change_current_state(None, None, Some(Scale::Fractional(1.5)), None);
    f.niri().output_resized(&output);

    let id = f.add_client();
    for (w, h) in [(100, 80), (300, 200)] {
        let window = f.client(id).create_window();
        let surface = window.surface.clone();
        window.commit();
        f.roundtrip(id);

        let window = f.client(id).window(&surface);
        window.attach_new_buffer();
        window.set_size(w, h);
        window.ack_last_and_commit();
        f.roundtrip(id);
    }

    // The second window is focused; capture the first one, which is not focused, by id.
    let first_id = f
        .niri()
        .layout
        .windows()
        .map(|(_, mapped)| mapped.id().get())
        .min()
        .unwrap();

    let dir = std::env::temp_dir().join(format!("niri-test-capture-window-{first_id}"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("first.png");
    let capture1 = capture(&mut f, Some(first_id), &path).unwrap();
    assert_eq!(capture1.id, first_id);
    assert_eq!((capture1.width_px, capture1.height_px), (150, 120));
    assert_eq!(capture1.scale, 1.5);
    assert_eq!(capture1.window_offset_px, (0, 0));
    assert_eq!(capture1.window_size_px, (150, 120));
    assert_eq!(png_size(&path), (150, 120));

    let path = dir.join("focused.png");
    let capture2 = capture(&mut f, None, &path).unwrap();
    assert_ne!(capture2.id, first_id);
    assert_eq!((capture2.width_px, capture2.height_px), (450, 300));
    assert_eq!(png_size(&path), (450, 300));

    f.dispatch();
    assert!(current_data_device_selection_userdata(&f.niri().seat).is_none());

    let err = capture(&mut f, Some(u64::MAX), &path).unwrap_err();
    assert_eq!(err, format!("no window with id {}", u64::MAX));

    let err = capture(&mut f, Some(first_id), &dir.join("missing/dir.png")).unwrap_err();
    assert!(err.starts_with("error writing "), "{err}");

    std::fs::remove_dir_all(&dir).unwrap();
}
