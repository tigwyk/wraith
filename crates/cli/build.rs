fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon/wraith.rc");
    // Same icon resources as the client, so wr.exe shows the ghost in Explorer.
    println!("cargo:rerun-if-changed=../../assets/icon/wraith.ico");
    embed_resource::compile_for("../../assets/icon/wraith.rc", ["wr"], version_macros())
        .manifest_optional()
        .unwrap();
}

/// The version block in wraith.rc, from Cargo.toml: `0,5,2,0` and `"0.5.2"`.
fn version_macros() -> [String; 2] {
    let v = std::env::var("CARGO_PKG_VERSION").unwrap();
    let num: Vec<&str> = v.split(['.', '-']).take(3).collect();
    [format!("WRAITH_VERSION_NUM={},0", num.join(",")), format!("WRAITH_VERSION_STR=\"{v}\"")]
}
