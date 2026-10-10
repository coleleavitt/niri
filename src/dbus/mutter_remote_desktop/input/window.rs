use smithay::desktop::Window;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size, Transform};

use super::RemoteDesktopPosition;
use crate::niri::Niri;

/// Resolves a position in a window stream to the output position that lands on that window now.
pub(super) fn window_position(
    niri: &Niri,
    id: u64,
    x: f64,
    y: f64,
) -> Result<RemoteDesktopPosition, String> {
    let geometry = niri
        .window_geometry(id)
        .ok_or_else(|| "the window does not exist".to_owned())?;
    if geometry.overview_open {
        return Err("the overview is open".to_owned());
    }
    let (Some(window_rect), Some(visible_rect), Some(output_name), Some(scale)) = (
        geometry.window_rect,
        geometry.visible_rect,
        geometry.output,
        geometry.scale,
    ) else {
        return Err("the window is not on screen".to_owned());
    };

    let (_, mapped) = niri
        .layout
        .windows()
        .find(|(_, mapped)| mapped.id().get() == id)
        .ok_or_else(|| "the window does not exist".to_owned())?;
    let point = stream_point_to_global(
        logical_rect(window_rect),
        window_offset_in_stream(&mapped.window, scale),
        scale,
        Point::from((x, y)),
    );
    if !logical_rect(visible_rect).contains(point) {
        return Err(format!(
            "point ({}, {}) is outside the visible part of the window",
            point.x, point.y
        ));
    }

    let output = niri
        .global_space
        .outputs()
        .find(|output| output.name() == output_name)
        .ok_or_else(|| format!("output {output_name} does not exist"))?;
    let output_geo = niri
        .global_space
        .output_geometry(output)
        .ok_or_else(|| format!("output {output_name} is not mapped"))?;

    Ok(output_position(
        output_name,
        output.current_transform(),
        output_geo,
        point,
    ))
}

/// Location of the window geometry inside a window stream buffer.
///
/// Window casts render `bbox_with_popups()` at the output scale, see
/// `Niri::render_windows_for_screen_cast()`.
fn window_offset_in_stream(window: &Window, scale: f64) -> Point<f64, Logical> {
    let bbox: Rectangle<i32, Physical> = window.bbox_with_popups().to_physical_precise_up(scale);
    window.geometry().loc.to_f64() - bbox.loc.to_f64().to_logical(scale)
}

/// Size of a window stream buffer.
pub(in crate::dbus::mutter_remote_desktop) fn window_stream_size(
    window: &Window,
    scale: f64,
) -> Size<i32, Physical> {
    window.bbox_with_popups().to_physical_precise_up(scale).size
}

fn logical_rect(rect: niri_ipc::LogicalRect) -> Rectangle<f64, Logical> {
    Rectangle::new(
        Point::from((rect.x, rect.y)),
        (rect.width, rect.height).into(),
    )
}

/// Maps a stream buffer pixel to a global point, clamped to the window geometry.
fn stream_point_to_global(
    window_rect: Rectangle<f64, Logical>,
    window_offset: Point<f64, Logical>,
    scale: f64,
    point: Point<f64, Physical>,
) -> Point<f64, Logical> {
    let local = point.to_logical(scale) - window_offset;
    // Stop at the last physical pixel, so that the point stays inside the window.
    let pixel = 1. / scale;
    let x = local.x.clamp(0., (window_rect.size.w - pixel).max(0.));
    let y = local.y.clamp(0., (window_rect.size.h - pixel).max(0.));
    window_rect.loc + Point::from((x, y))
}

/// Inverse of the mapping in `State::compute_absolute_location()`.
fn output_position(
    output_name: String,
    transform: Transform,
    output_geo: Rectangle<i32, Logical>,
    point: Point<f64, Logical>,
) -> RemoteDesktopPosition {
    let size = transform.invert().transform_size(output_geo.size);
    // Transform::invert() does not invert transform_point_in() for flipped transforms, which are
    // their own inverse there.
    let inverse = match transform {
        Transform::_90 => Transform::_270,
        Transform::_270 => Transform::_90,
        transform => transform,
    };
    let local =
        inverse.transform_point_in(point - output_geo.loc.to_f64(), &output_geo.size.to_f64());
    RemoteDesktopPosition {
        output_name,
        x: local.x,
        y: local.y,
        width: size.w,
        height: size.h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_pixels_are_scaled_and_offset_by_the_window_geometry() {
        let window_rect = Rectangle::new(Point::from((1920., 40.)), (400., 300.).into());
        let offset = Point::from((10., 5.));

        let point = stream_point_to_global(window_rect, offset, 2., Point::from((120., 50.)));

        assert_eq!(point, Point::from((1970., 60.)));
    }

    #[test]
    fn stream_points_are_clamped_to_the_window() {
        let window_rect = Rectangle::new(Point::from((100., 200.)), (400., 300.).into());
        let offset = Point::from((10., 5.));

        let top_left = stream_point_to_global(window_rect, offset, 2., Point::from((0., 0.)));
        let bottom_right =
            stream_point_to_global(window_rect, offset, 2., Point::from((5000., 5000.)));

        assert_eq!(top_left, Point::from((100., 200.)));
        assert_eq!(bottom_right, Point::from((499.5, 499.5)));
        assert!(window_rect.contains(bottom_right));
    }

    #[test]
    fn output_positions_round_trip_through_output_transforms() {
        let output_geo = Rectangle::new(Point::from((1920, 0)), (1280, 720).into());
        let point = Point::from((2000.25, 100.5));

        for transform in [
            Transform::Normal,
            Transform::_90,
            Transform::_180,
            Transform::_270,
            Transform::Flipped,
            Transform::Flipped90,
            Transform::Flipped180,
            Transform::Flipped270,
        ] {
            let position = output_position("DP-1".to_owned(), transform, output_geo, point);

            // Same steps as State::compute_absolute_location().
            let size = transform.invert().transform_size(output_geo.size).to_f64();
            let transformed = Point::from((
                position.x * size.w / f64::from(position.width),
                position.y * size.h / f64::from(position.height),
            ));
            let resolved =
                transform.transform_point_in(transformed, &size) + output_geo.loc.to_f64();

            assert_eq!(resolved, point, "{transform:?}");
        }
    }
}
