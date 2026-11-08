//! Links the firmware with the esp-hal linker scripts.

fn main() {
    // `linkall.x` pulls in the memory layout for the selected chip and must
    // be the last linker script.
    println!("cargo:rustc-link-arg-bins=-Tlinkall.x");
}
