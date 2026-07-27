//! Backlight output for adaptive night light.
//!
//! Writing `/sys/class/backlight/*/brightness` requires root or a udev rule
//! granting the seat's user write access. A compositor started as a normal user
//! usually has neither, so the write fails with `EACCES` on every tick and the
//! backlight silently never moves.
//!
//! logind exposes `org.freedesktop.login1.Session.SetBrightness` for exactly
//! this case and performs it unprivileged for the active session. So: write
//! sysfs directly when we may (no IPC on the hot path), and fall back to logind
//! the first time the kernel says no.

use std::io;

use niri_config::night_light::AdaptiveNightLight;

use super::sysfs::{self, BacklightDevice};

/// The logind session object of the calling process.
#[cfg(feature = "dbus")]
const SESSION_PATH: &str = "/org/freedesktop/login1/session/auto";

pub struct Backlight {
    device: BacklightDevice,
    /// Set once sysfs has refused a write; we do not probe it again.
    use_logind: bool,
    #[cfg(feature = "dbus")]
    conn: Option<zbus::blocking::Connection>,
}

impl Backlight {
    pub fn new(config: &AdaptiveNightLight) -> io::Result<Self> {
        Ok(Self {
            device: sysfs::backlight_device(config)?,
            use_logind: false,
            #[cfg(feature = "dbus")]
            conn: None,
        })
    }

    /// Applies a 0.0..=1.0 ratio of the device's maximum brightness.
    pub fn set_ratio(&mut self, ratio: f64) -> io::Result<()> {
        let target = self.device.target_for(ratio);

        if !self.use_logind {
            match self.device.write_sysfs(target) {
                Ok(()) => return Ok(()),
                Err(err) if is_permission_denied(&err) => {
                    debug!(
                        "night-light: backlight sysfs write denied ({err}), \
                         switching to logind"
                    );
                    self.use_logind = true;
                }
                Err(err) => return Err(err),
            }
        }

        self.set_via_logind(target)
    }

    #[cfg(feature = "dbus")]
    fn set_via_logind(&mut self, target: u64) -> io::Result<()> {
        if self.conn.is_none() {
            self.conn = Some(
                zbus::blocking::Connection::system()
                    .map_err(|err| io::Error::other(format!("no system bus: {err}")))?,
            );
        }
        let conn = self.conn.as_ref().unwrap();

        let target = u32::try_from(target).unwrap_or(u32::MAX);
        conn.call_method(
            Some("org.freedesktop.login1"),
            SESSION_PATH,
            Some("org.freedesktop.login1.Session"),
            "SetBrightness",
            &("backlight", self.device.name.as_str(), target),
        )
        .map_err(|err| io::Error::other(format!("logind SetBrightness failed: {err}")))?;

        Ok(())
    }

    #[cfg(not(feature = "dbus"))]
    fn set_via_logind(&mut self, _target: u64) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "backlight is not writable and niri was built without D-Bus support",
        ))
    }
}

fn is_permission_denied(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives the real panel. Ignored by default because it needs an active
    /// logind session and visibly changes the screen.
    ///
    /// Run with: cargo test --release backlight -- --ignored --nocapture
    #[test]
    #[ignore = "requires a real backlight and an active logind session"]
    fn sets_real_backlight_when_sysfs_is_not_writable() {
        let config = AdaptiveNightLight::default();
        let mut backlight = Backlight::new(&config).expect("no backlight device");

        let path = backlight.device.directory.join("brightness");
        let read_panel = || -> u64 {
            std::fs::read_to_string(&path)
                .unwrap()
                .trim()
                .parse()
                .unwrap()
        };

        let original = read_panel();
        let max = backlight.device.max_brightness;
        println!("backlight {} at {original}/{max}", backlight.device.name);

        for ratio in [0.15, 0.60] {
            let expected = backlight.device.target_for(ratio);
            backlight.set_ratio(ratio).expect("failed to set backlight");

            let actual = read_panel();
            println!(
                "ratio {ratio} -> wanted {expected}, panel reports {actual} (via {})",
                if backlight.use_logind {
                    "logind"
                } else {
                    "sysfs"
                }
            );
            assert_eq!(actual, expected);
        }

        backlight
            .set_ratio(original as f64 / max as f64)
            .expect("failed to restore backlight");
        assert_eq!(read_panel(), original);
    }
}
