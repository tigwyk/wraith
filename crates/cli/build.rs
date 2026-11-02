fn main() {
    // Same icon resources as the client, so wr.exe shows the ghost in Explorer.
    println!("cargo:rerun-if-changed=../../assets/icon/wraith.ico");
    embed_resource::compile_for("../../assets/icon/wraith.rc", ["wr"], embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
