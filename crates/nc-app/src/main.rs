//! `nca-gui` entry point.
//!
//! The binary only assembles launch options and hands control to the library
//! shell. It never connects to a controller and never writes a file.

fn main() {
    let options = nc_app::LaunchOptions::from_args(std::env::args().skip(1));
    nc_app::run(options);
}
