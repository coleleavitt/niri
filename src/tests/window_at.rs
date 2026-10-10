use client::ClientId;
use insta::assert_snapshot;
use niri_config::Config;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Anchor;
use smithay::utils::Point;
use wayland_client::protocol::wl_surface::WlSurface;

use super::*;
use crate::tests::client::LayerConfigureProps;

// Sets up a fixture with one output and a 100×100 tiled window.
fn set_up(config: Config) -> (Fixture, ClientId, WlSurface) {
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));

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

    (f, id, surface)
}

fn window_at(f: &mut Fixture, x: f64, y: f64) -> String {
    let niri = f.niri();
    let hit = niri.window_at(Point::from((x, y)));

    // Window ids come from a global counter, so print whether it's our only window instead.
    let only_window = niri.layout.windows().next().map(|(_, w)| w.id().get());
    let window = match hit.window_id {
        None => "none",
        Some(id) if Some(id) == only_window => "the window",
        Some(_) => "other",
    };

    format!(
        "output: {:?}, window: {window}, local: {:?}, input: {}, layer: {:?}",
        hit.output, hit.window_local, hit.is_input, hit.layer
    )
}

#[test]
fn tiled_window() {
    let (mut f, _id, _surface) = set_up(Config::default());

    assert_snapshot!(window_at(&mut f, 20., 30.), @r#"output: Some("headless-1"), window: the window, local: Some((4.0, 14.0)), input: true, layer: None"#);
    assert_snapshot!(window_at(&mut f, 115.5, 115.), @r#"output: Some("headless-1"), window: the window, local: Some((99.5, 99.0)), input: true, layer: None"#);
}

#[test]
fn gap_and_outside() {
    let (mut f, _id, _surface) = set_up(Config::default());

    assert_snapshot!(window_at(&mut f, 8., 8.), @r#"output: Some("headless-1"), window: none, local: None, input: false, layer: None"#);
    assert_snapshot!(window_at(&mut f, 1000., 500.), @r#"output: Some("headless-1"), window: none, local: None, input: false, layer: None"#);
    assert_snapshot!(window_at(&mut f, -10., 10.), @"output: None, window: none, local: None, input: false, layer: None");
}

#[test]
fn border_activates_only() {
    let config = r##"
layout {
    border {
        on
        width 10
    }
}
"##;
    let config = Config::parse_mem(config).unwrap();
    let (mut f, _id, _surface) = set_up(config);

    assert_snapshot!(window_at(&mut f, 20., 30.), @r#"output: Some("headless-1"), window: the window, local: None, input: false, layer: None"#);
    assert_snapshot!(window_at(&mut f, 30., 40.), @r#"output: Some("headless-1"), window: the window, local: Some((4.0, 14.0)), input: true, layer: None"#);
}

#[test]
fn layer_on_top_of_window() {
    let (mut f, id, _surface) = set_up(Config::default());

    let layer = f.client(id).create_layer(None, Layer::Top, "bar");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Left | Anchor::Right | Anchor::Top),
        size: Some((0, 50)),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);

    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(1920, 50);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    assert_snapshot!(window_at(&mut f, 20., 30.), @r#"output: Some("headless-1"), window: none, local: None, input: true, layer: Some(LayerHit { namespace: "bar", layer: Top })"#);
    assert_snapshot!(window_at(&mut f, 20., 60.), @r#"output: Some("headless-1"), window: the window, local: Some((4.0, 44.0)), input: true, layer: None"#);
}
