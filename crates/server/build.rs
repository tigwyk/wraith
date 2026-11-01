//! Fetch Microsoft's redistributable ConPTY (conpty.dll + OpenConsole.exe) next to our binaries.
//! Inbox ConPTY strips the Kitty graphics APC; see docs/m0-findings.md.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

use sha2::{Digest, Sha256};

const VERSION: &str = "1.25.260930003";
const SHA256: &str = "02b07b349af66d801159bdf9e440d4a1ce78bb951f37fc8609731665afdae7ee";
const FILES: &[(&str, &str)] = &[
    ("runtimes/win-x64/native/conpty.dll", "conpty.dll"),
    ("build/native/runtimes/x64/OpenConsole.exe", "OpenConsole.exe"),
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=WRAITH_CONPTY_DIR");
    if env::var("CARGO_CFG_WINDOWS").is_err() {
        return;
    }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    // OUT_DIR = <target>/<profile>/build/<pkg>-<hash>/out
    let profile = out.ancestors().nth(3).unwrap().to_path_buf();
    let dests = [profile.clone(), profile.join("deps")];

    let result = match env::var_os("WRAITH_CONPTY_DIR") {
        Some(dir) => copy_dir(Path::new(&dir), &dests),
        None => fetch(&out).and_then(|pkg| extract(&pkg, &dests)),
    };
    if let Err(e) = result {
        println!("cargo:warning=bundled ConPTY unavailable ({e}); panes will use inbox ConPTY without inline images");
    }
}

fn fetch(out: &Path) -> Result<PathBuf, String> {
    let pkg = out.join(format!("conpty-{VERSION}.nupkg"));
    if pkg.exists() && sha256(&pkg)? == SHA256 {
        return Ok(pkg);
    }
    let url = format!(
        "https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/{VERSION}/microsoft.windows.console.conpty.{VERSION}.nupkg"
    );
    let status = Command::new("curl.exe")
        .args(["-sSfL", "--retry", "2", "-o"])
        .arg(&pkg)
        .arg(&url)
        .status()
        .map_err(|e| format!("curl.exe: {e}"))?;
    if !status.success() {
        return Err(format!("download failed: {status}"));
    }
    let got = sha256(&pkg)?;
    if got != SHA256 {
        let _ = fs::remove_file(&pkg);
        return Err(format!("sha256 mismatch: got {got}"));
    }
    Ok(pkg)
}

fn sha256(p: &Path) -> Result<String, String> {
    let bytes = fs::read(p).map_err(|e| e.to_string())?;
    Ok(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect())
}

fn extract(pkg: &Path, dests: &[PathBuf]) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(fs::File::open(pkg).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    for (inner, name) in FILES {
        let mut entry = zip.by_name(inner).map_err(|e| format!("{inner}: {e}"))?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut buf).map_err(|e| e.to_string())?;
        for d in dests {
            write_if_changed(&d.join(name), &buf)?;
        }
    }
    Ok(())
}

fn copy_dir(dir: &Path, dests: &[PathBuf]) -> Result<(), String> {
    for (_, name) in FILES {
        let buf = fs::read(dir.join(name)).map_err(|e| format!("{name}: {e}"))?;
        for d in dests {
            write_if_changed(&d.join(name), &buf)?;
        }
    }
    Ok(())
}

/// A running exe holds its file open; skip the write when content already matches.
fn write_if_changed(path: &Path, buf: &[u8]) -> Result<(), String> {
    if fs::read(path).is_ok_and(|old| old == buf) {
        return Ok(());
    }
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(path, buf).map_err(|e| format!("{}: {e}", path.display()))
}
