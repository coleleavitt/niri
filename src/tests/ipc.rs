use std::io::{BufRead as _, BufReader, ErrorKind, Write as _};
use std::os::unix::net::UnixStream;

use niri_ipc::{Reply, Request, Response};

use super::*;

fn ipc_request(f: &mut Fixture, request: &Request) -> Reply {
    let (client, server) = UnixStream::pair().unwrap();
    crate::ipc::server::on_new_ipc_client(f.niri_state(), server);

    let mut buf = serde_json::to_string(request).unwrap();
    buf.push('\n');
    (&client).write_all(buf.as_bytes()).unwrap();

    client.set_nonblocking(true).unwrap();
    let mut client = BufReader::new(client);
    let mut line = String::new();
    for _ in 0..100 {
        f.state.server.dispatch();
        match client.read_line(&mut line) {
            Ok(_) if line.ends_with('\n') => return serde_json::from_str(&line).unwrap(),
            Ok(_) => (),
            Err(err) if err.kind() == ErrorKind::WouldBlock => (),
            Err(err) => panic!("error reading IPC reply: {err:?}"),
        }
    }
    panic!("no IPC reply");
}

#[test]
fn window_geometry_round_trip() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));
    f.niri_focus_output(2);

    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(100, 100);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.niri_complete_animations();

    let output = f.niri_output(2);
    let output_geo = f.niri().global_space.output_geometry(&output).unwrap();
    let output_geo = output_geo.to_f64();
    assert_ne!(output_geo.loc.x, 0.);

    let reply = ipc_request(&mut f, &Request::WindowGeometry { id: None });
    let Ok(Response::WindowGeometry(geo)) = reply else {
        panic!("unexpected reply: {reply:?}");
    };

    let window_id = f.niri().layout.focus().unwrap().id().get();
    assert_eq!(geo.id, window_id);
    assert_eq!(geo.output.as_deref(), Some("headless-2"));
    assert_eq!(geo.scale, Some(1.));
    assert!(geo.on_screen);
    assert!(!geo.is_floating);
    assert!(!geo.animating);
    assert!(!geo.overview_open);
    assert!(geo.fully_visible);

    // Default gaps are 16 and the window has no border, so it sits at the working area corner
    // of the second output, in global coordinates.
    let window_rect = geo.window_rect.unwrap();
    assert_eq!(window_rect.x, output_geo.loc.x + 16.);
    assert_eq!(window_rect.y, output_geo.loc.y + 16.);
    assert_eq!((window_rect.width, window_rect.height), (100., 100.));
    assert_eq!(geo.visible_rect, Some(window_rect));

    let reply = ipc_request(
        &mut f,
        &Request::WindowGeometry {
            id: Some(window_id),
        },
    );
    assert!(matches!(reply, Ok(Response::WindowGeometry(by_id)) if by_id == geo));

    let reply = ipc_request(&mut f, &Request::WindowGeometry { id: Some(u64::MAX) });
    assert!(reply.is_err());
}
