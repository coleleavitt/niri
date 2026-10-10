use fixture::Fixture;

mod client;
mod fixture;
mod server;

mod animations;
mod capture_window;
mod floating;
mod fullscreen;
mod image_copy_capture;
mod ipc;
mod layer_shell;
mod pointer_warp;
#[cfg(feature = "xdp-gnome-screencast")]
mod remote_desktop;
mod remove_output;
mod transactions;
mod virtual_input;
mod window_at;
mod window_opening;
