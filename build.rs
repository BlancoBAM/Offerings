// build.rs - Slint Compilation
fn main() {
    slint_build::compile("ui/main.slint").unwrap();
}
