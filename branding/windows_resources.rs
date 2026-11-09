// Shared by the game's and the launcher's build scripts (`include!`d): embeds the icon
// (packed by `ta-tools brand` from the emblem `tools/branding/emblem.py` draws) and the
// version information in a Windows executable.
// Where no resource compiler is found the executable is built without them.

fn embed_windows_resources(file_name: &str, description: &str) {
    let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon = dir.join("../../branding/ta-reborn.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-changed=../../branding/windows_resources.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let numbers: Vec<u16> = version.split(['.', '-', '+']).take(3).map(|n| n.parse().unwrap_or(0)).collect();
    let (major, minor, patch) = (numbers[0], numbers.get(1).copied().unwrap_or(0), numbers.get(2).copied().unwrap_or(0));
    // The resource script is C-like: backslashes in the path are escapes.
    let icon = icon.display().to_string().replace('\\', "/");
    let rc = format!(
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "TA Reborn contributors"
      VALUE "FileDescription", "{description}"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "{file_name}"
      VALUE "OriginalFilename", "{file_name}.exe"
      VALUE "ProductName", "TA Reborn"
      VALUE "ProductVersion", "{version}"
      VALUE "LegalCopyright", "TA Reborn contributors. Total Annihilation is the property of its respective owners."
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("app.rc");
    std::fs::write(&out, rc).unwrap();
    match embed_resource::compile(&out, embed_resource::NONE) {
        embed_resource::CompilationResult::NotAttempted(why) => println!("cargo:warning=built without an icon: {why}"),
        embed_resource::CompilationResult::Failed(why) => panic!("resource compiler failed: {why}"),
        _ => {}
    }
}
