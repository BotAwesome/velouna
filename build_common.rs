// Shared build script of the app (build.rs) and the installer (setup/build.rs).
// Embeds the Velour icon and version info into the Windows executable.
// The icon is rasterized from the same geometry the app draws (src/logo.rs).

include!("src/logo.rs");

use std::io::Write;

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges><requestedExecutionLevel level="asInvoker" uiAccess="false"/></requestedPrivileges></security>
  </trustInfo>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    </windowsSettings>
  </application>
  <dependency><dependentAssembly>
    <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
  </dependentAssembly></dependency>
</assembly>"#;

/// Installer only: the file types Velour plays, read from src/media.rs so
/// the "Open with" registration never drifts from what the player accepts.
fn extra_setup() {
    println!("cargo:rerun-if-changed=../src/media.rs");
    let media = std::fs::read_to_string("../src/media.rs").unwrap_or_default();
    let mut exts: Vec<String> = Vec::new();
    for name in ["VIDEO_EXT", "AUDIO_EXT", "PLAYLIST_EXT"] {
        let Some(start) = media.find(&format!("pub const {name}")) else { continue };
        let body = &media[start..];
        let (Some(a), Some(b)) = (body.find("&["), body.find("];")) else { continue };
        exts.extend(body[a + 2..b].split(',').map(|s| s.trim().trim_matches('"').to_string()).filter(|s| !s.is_empty()));
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("exts.rs");
    let list = exts.iter().map(|e| format!("{e:?}")).collect::<Vec<_>>().join(", ");
    std::fs::write(out, format!("pub const MEDIA_EXT: &[&str] = &[{list}];
")).unwrap();
}
use std::path::Path;

/// Writes a multi-size .ico with 32-bit BMP images.
fn write_ico(path: &Path, sizes: &[usize]) -> std::io::Result<()> {
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&n| {
            let rgba = logo_rasterize(n);
            let mask_row = n.div_ceil(32) * 4;
            let mut b = Vec::with_capacity(40 + n * n * 4 + mask_row * n);
            for v in [40u32, n as u32, (n * 2) as u32] {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b.extend_from_slice(&1u16.to_le_bytes());
            b.extend_from_slice(&32u16.to_le_bytes());
            for v in [0u32, (n * n * 4 + mask_row * n) as u32, 0, 0, 0, 0] {
                b.extend_from_slice(&v.to_le_bytes());
            }
            for y in (0..n).rev() {
                for x in 0..n {
                    let i = (y * n + x) * 4;
                    b.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
                }
            }
            b.extend(std::iter::repeat_n(0u8, mask_row * n));
            b
        })
        .collect();
    let mut f = std::fs::File::create(path)?;
    f.write_all(&[0, 0, 1, 0])?;
    f.write_all(&(sizes.len() as u16).to_le_bytes())?;
    let mut offset = 6 + 16 * sizes.len();
    for (&n, img) in sizes.iter().zip(&images) {
        let d = if n >= 256 { 0 } else { n as u8 };
        f.write_all(&[d, d, 0, 0])?;
        f.write_all(&1u16.to_le_bytes())?;
        f.write_all(&32u16.to_le_bytes())?;
        f.write_all(&(img.len() as u32).to_le_bytes())?;
        f.write_all(&(offset as u32).to_le_bytes())?;
        offset += img.len();
    }
    for img in &images {
        f.write_all(img)?;
    }
    Ok(())
}

fn main() {
    // Shared by the app and the installer crate (setup/build.rs includes this file).
    let setup = std::env::var("CARGO_PKG_NAME").as_deref() == Ok("velour-setup");
    let up = if setup { "../" } else { "" };
    println!("cargo:rerun-if-changed={up}src/logo.rs");
    println!("cargo:rerun-if-changed={up}build_common.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = out.join("velour.ico");
    if let Err(e) = write_ico(&ico, &[16, 20, 24, 32, 40, 48, 64, 128, 256]) {
        println!("cargo:warning=could not write icon: {e}");
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico.to_str().unwrap());
    res.set("ProductName", "Velour");
    res.set("FileDescription", if setup { "Velour Setup" } else { "Velour Media Player" });
    if setup {
        // Without a manifest Windows treats "*setup*" exes as installers and
        // forces UAC; Velour installs per user and needs no elevation.
        res.set_manifest(MANIFEST);
        extra_setup();
    }
    if let Err(e) = res.compile() {
        println!("cargo:warning=icon not embedded: {e}");
    }
}
