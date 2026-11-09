// Shared by the programs' build scripts (`include!`d): the commit a build is made from, as
// `TA_BUILD_COMMIT` (empty outside a git checkout), shown beside the version and written
// into crash reports so a report can be matched with the build's symbols.

fn emit_build_info() {
    let repo = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    // Build again when HEAD moves: a new commit, another branch checked out.
    let git = repo.join(".git");
    if git.is_dir() {
        let head = git.join("HEAD");
        println!("cargo:rerun-if-changed={}", head.display());
        if let Some(branch) = std::fs::read_to_string(&head).ok().as_deref().and_then(|h| h.strip_prefix("ref: ")) {
            let r = git.join(branch.trim());
            if r.exists() {
                println!("cargo:rerun-if-changed={}", r.display());
            }
        }
        let packed = git.join("packed-refs");
        if packed.exists() {
            println!("cargo:rerun-if-changed={}", packed.display());
        }
    }
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short=9", "HEAD"])
        .current_dir(&repo)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=TA_BUILD_COMMIT={commit}");
}
