use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use std::{fs, io};

use niri_config::night_light::AdaptiveNightLight;

const IIO_ROOT: &str = "/sys/bus/iio/devices";
const BACKLIGHT_ROOT: &str = "/sys/class/backlight";

pub fn read_ambient_lux(config: &AdaptiveNightLight) -> Option<f64> {
    if let Some(path) = config.sensor_path.as_deref() {
        return read_fresh_value(&expand_runtime_path(path), config.sensor_max_age_secs);
    }

    read_ambient_lux_from_root(Path::new(IIO_ROOT))
}

/// Reads the room's measured colour temperature in kelvin, if configured.
pub fn read_ambient_temperature(config: &AdaptiveNightLight) -> Option<f64> {
    let path = config.temperature_path.as_deref()?;
    read_fresh_value(&expand_runtime_path(path), config.sensor_max_age_secs)
        .filter(|kelvin| *kelvin >= 1000.0 && *kelvin <= 25000.0)
}

/// Reads a sensor file, ignoring it once it goes stale.
///
/// An external sampler that dies leaves its last value on disk forever. Without
/// an age check that is indistinguishable from a live reading, so the screen
/// would stay pinned to whatever the room looked like when the sampler died.
fn read_fresh_value(path: &Path, max_age_secs: u64) -> Option<f64> {
    if max_age_secs > 0 {
        let modified = fs::metadata(path).and_then(|meta| meta.modified()).ok()?;
        let age = SystemTime::now()
            .duration_since(modified)
            .unwrap_or(Duration::ZERO);
        if age > Duration::from_secs(max_age_secs) {
            return None;
        }
    }

    read_lux(path).ok()
}

/// A resolved backlight device.
///
/// `name` is what logind wants (the `/sys/class/backlight` device name);
/// `directory` is the sysfs path we write when we are allowed to.
pub struct BacklightDevice {
    pub name: String,
    pub directory: PathBuf,
    pub max_brightness: u64,
}

impl BacklightDevice {
    /// Converts a 0.0..=1.0 ratio into a raw brightness value.
    pub fn target_for(&self, ratio: f64) -> u64 {
        let ratio = ratio.clamp(0.0, 1.0);
        (ratio * self.max_brightness as f64).round() as u64
    }

    pub fn write_sysfs(&self, target: u64) -> io::Result<()> {
        fs::write(self.directory.join("brightness"), target.to_string())
    }
}

pub fn backlight_device(config: &AdaptiveNightLight) -> io::Result<BacklightDevice> {
    let directory = backlight_directory(config)?;
    let name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid_data("backlight device has no name"))?
        .to_owned();
    let max_brightness = read_u64(&directory.join("max_brightness"))?;

    Ok(BacklightDevice {
        name,
        directory,
        max_brightness,
    })
}

fn read_ambient_lux_from_root(root: &Path) -> Option<f64> {
    let mut entries = fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    entries.sort();

    entries.into_iter().find_map(|directory| {
        if !directory.is_dir() {
            return None;
        }

        read_lux(&directory.join("in_illuminance_input"))
            .ok()
            .or_else(|| read_scaled_lux(&directory))
    })
}

fn read_scaled_lux(directory: &Path) -> Option<f64> {
    let raw = read_lux(&directory.join("in_illuminance_raw")).ok()?;
    let scale = read_lux(&directory.join("in_illuminance_scale")).unwrap_or(1.0);
    Some(raw * scale)
}

fn read_lux(path: &Path) -> io::Result<f64> {
    let text = fs::read_to_string(path)?;
    parse_lux(&text).ok_or_else(|| invalid_data("invalid ambient light value"))
}

fn parse_lux(text: &str) -> Option<f64> {
    text.split_whitespace()
        .find_map(|word| word.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
}

fn backlight_directory(config: &AdaptiveNightLight) -> io::Result<PathBuf> {
    if let Some(path) = config.backlight_path.as_ref() {
        return Ok(expand_runtime_path(path));
    }

    if let Some(name) = config.backlight_name.as_ref() {
        return Ok(Path::new(BACKLIGHT_ROOT).join(name));
    }

    first_backlight_directory(Path::new(BACKLIGHT_ROOT))
}

fn first_backlight_directory(root: &Path) -> io::Result<PathBuf> {
    let mut entries = fs::read_dir(root)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    entries.sort();

    entries
        .into_iter()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no backlight device found"))
}

fn expand_runtime_path(path: &Path) -> PathBuf {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
    expand_runtime_path_with(path, runtime_dir.as_deref())
}

fn expand_runtime_path_with(path: &Path, runtime_dir: Option<&OsStr>) -> PathBuf {
    let Some(runtime_dir) = runtime_dir else {
        return path.to_owned();
    };
    let text = path.as_os_str().to_string_lossy();

    if text == "$XDG_RUNTIME_DIR" || text == "${XDG_RUNTIME_DIR}" {
        return PathBuf::from(runtime_dir);
    }
    if let Some(rest) = text
        .strip_prefix("$XDG_RUNTIME_DIR/")
        .or_else(|| text.strip_prefix("${XDG_RUNTIME_DIR}/"))
    {
        return PathBuf::from(runtime_dir).join(rest);
    }

    path.to_owned()
}

fn read_u64(path: &Path) -> io::Result<u64> {
    let text = fs::read_to_string(path)?;
    text.split_whitespace()
        .next()
        .and_then(|word| word.parse::<u64>().ok())
        .ok_or_else(|| invalid_data("invalid integer value"))
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("niri-night-light-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        directory
    }

    #[test]
    fn reads_direct_lux_file() {
        let root = temp_root("direct-lux");
        let path = root.join("lux");
        fs::write(&path, "42.5\n").unwrap();

        assert_eq!(read_lux(&path).unwrap(), 42.5);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovers_iio_lux_input() {
        let root = temp_root("iio-input");
        let device = root.join("iio:device0");
        fs::create_dir(&device).unwrap();
        fs::write(device.join("in_illuminance_input"), "18\n").unwrap();

        assert_eq!(read_ambient_lux_from_root(&root), Some(18.0));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovers_scaled_iio_lux() {
        let root = temp_root("iio-scaled");
        let device = root.join("iio:device0");
        fs::create_dir(&device).unwrap();
        fs::write(device.join("in_illuminance_raw"), "21\n").unwrap();
        fs::write(device.join("in_illuminance_scale"), "0.5\n").unwrap();

        assert_eq!(read_ambient_lux_from_root(&root), Some(10.5));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn writes_clamped_backlight_ratio() {
        let root = temp_root("backlight");
        fs::write(root.join("max_brightness"), "800\n").unwrap();
        fs::write(root.join("brightness"), "0\n").unwrap();

        let device = BacklightDevice {
            name: "test_backlight".to_owned(),
            directory: root.clone(),
            max_brightness: 800,
        };

        device.write_sysfs(device.target_for(0.125)).unwrap();
        assert_eq!(fs::read_to_string(root.join("brightness")).unwrap(), "100");

        device.write_sysfs(device.target_for(2.0)).unwrap();
        assert_eq!(fs::read_to_string(root.join("brightness")).unwrap(), "800");

        device.write_sysfs(device.target_for(-1.0)).unwrap();
        assert_eq!(fs::read_to_string(root.join("brightness")).unwrap(), "0");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolves_backlight_device_name_and_maximum() {
        let root = temp_root("backlight-device");
        let device_dir = root.join("intel_backlight");
        fs::create_dir(&device_dir).unwrap();
        fs::write(device_dir.join("max_brightness"), "800\n").unwrap();

        let config = AdaptiveNightLight {
            backlight_path: Some(device_dir.clone()),
            ..Default::default()
        };
        let device = backlight_device(&config).unwrap();

        // logind identifies the device by name, not by path.
        assert_eq!(device.name, "intel_backlight");
        assert_eq!(device.max_brightness, 800);
        assert_eq!(device.directory, device_dir);

        fs::remove_dir_all(root).unwrap();
    }

    fn age_file(path: &Path, secs: u64) {
        let when = SystemTime::now() - Duration::from_secs(secs);
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(when).unwrap();
    }

    #[test]
    fn ignores_a_stale_sensor_file() {
        let root = temp_root("stale-sensor");
        let path = root.join("lux");
        fs::write(&path, "42.5\n").unwrap();

        let config = AdaptiveNightLight {
            sensor_path: Some(path.clone()),
            sensor_max_age_secs: 60,
            ..Default::default()
        };

        assert_eq!(read_ambient_lux(&config), Some(42.5));

        // A sampler that died an hour ago leaves a perfectly readable file.
        age_file(&path, 3600);
        assert_eq!(read_ambient_lux(&config), None);

        // Age checking is opt-out for sensors that legitimately never change.
        let forever = AdaptiveNightLight {
            sensor_max_age_secs: 0,
            ..config.clone()
        };
        assert_eq!(read_ambient_lux(&forever), Some(42.5));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_ambient_temperature_within_a_sane_range() {
        let root = temp_root("ambient-temp");
        let path = root.join("temp");
        let config = AdaptiveNightLight {
            temperature_path: Some(path.clone()),
            ..Default::default()
        };

        fs::write(&path, "2700\n").unwrap();
        assert_eq!(read_ambient_temperature(&config), Some(2700.0));

        // Nonsense from a broken sampler must not reach the gamma ramp.
        fs::write(&path, "12\n").unwrap();
        assert_eq!(read_ambient_temperature(&config), None);
        fs::write(&path, "99999\n").unwrap();
        assert_eq!(read_ambient_temperature(&config), None);

        // Unset path means the feature is simply off.
        assert_eq!(
            read_ambient_temperature(&AdaptiveNightLight::default()),
            None
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn expands_xdg_runtime_dir_paths() {
        let runtime_dir = OsStr::new("/run/user/1000");

        assert_eq!(
            expand_runtime_path_with(
                Path::new("$XDG_RUNTIME_DIR/niri-ambient-lux"),
                Some(runtime_dir)
            ),
            PathBuf::from("/run/user/1000/niri-ambient-lux")
        );
        assert_eq!(
            expand_runtime_path_with(
                Path::new("${XDG_RUNTIME_DIR}/niri-ambient-lux"),
                Some(runtime_dir)
            ),
            PathBuf::from("/run/user/1000/niri-ambient-lux")
        );
    }
}
