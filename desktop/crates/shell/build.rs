//! Compiles the window markup into Rust before the crate itself is built.

fn main() {
    slint_build::compile("ui/app.slint").expect("the window markup does not compile");
}
