use std::process::Command;

#[test]
fn tty_backend_failure_exits_without_panicking() {
    let output = Command::new(env!("CARGO_BIN_EXE_niri"))
        .arg("--session")
        .env("LIBSEAT_BACKEND", "seatd")
        .env("SEATD_SOCK", "/tmp/niri-test-missing-seatd.sock")
        .env("RUST_BACKTRACE", "0")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .expect("niri should start far enough to report the backend error");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert_ne!(output.status.code(), Some(101), "{stderr}");
    assert!(!stderr.contains("panicked at"), "{stderr}");
    assert!(
        stderr.contains("error initializing the TTY backend"),
        "{stderr}"
    );
}
