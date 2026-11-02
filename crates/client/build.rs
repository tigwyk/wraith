fn main() {
    // Icon 1 (app): Explorer and the taskbar pick it from the exe; window.rs loads it for the window class.
    // Icon 2 (badge): the taskbar overlay notify.rs shows when an agent needs attention.
    println!("cargo:rerun-if-changed=../../assets/icon/wraith.ico");
    println!("cargo:rerun-if-changed=../../assets/icon/wraith-badge.ico");
    embed_resource::compile_for(
        "../../assets/icon/wraith.rc",
        ["wraith"],
        embed_resource::NONE,
    )
    .manifest_optional()
    .unwrap();
}
