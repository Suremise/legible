//! Rebuilds the crate when the embedded web assets or vendored JavaScript change
//! (`include_dir!` doesn't tell cargo which files it read).

fn main() {
    println!("cargo:rerun-if-changed=../../web");
    println!("cargo:rerun-if-changed=../../third_party/js");
}
