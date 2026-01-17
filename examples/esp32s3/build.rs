fn main() {
    // Link to esp-hal's linker scripts
    println!("cargo:rustc-link-arg=-Tlinkall.x");
}
