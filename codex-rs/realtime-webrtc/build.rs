fn main() {
    println!("cargo:rustc-check-cfg=cfg(codex_bazel)");
}
