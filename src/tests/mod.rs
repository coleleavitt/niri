use fixture::Fixture;

mod client;
mod fixture;
mod server;

mod animations;
mod floating;
mod fullscreen;
mod ipc;
mod layer_shell;
#[cfg(feature = "xdp-gnome-screencast")]
mod remote_desktop;
mod remove_output;
mod transactions;
mod window_opening;
