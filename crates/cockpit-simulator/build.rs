fn main() {
    let target = std::env::var("TARGET").expect("Cargo always provides TARGET");
    println!("cargo:rustc-env=COCKPIT_TARGET={target}");
}
