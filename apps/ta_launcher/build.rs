//! The executable's icon and version information (Windows), and the commit it is built
//! from.

include!("../../branding/windows_resources.rs");
include!("../build_info.rs");

fn main() {
    embed_windows_resources("ta-launcher", "TA Reborn launcher");
    emit_build_info();
}
