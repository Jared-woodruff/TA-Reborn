use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use ta_formats::hpi::HpiArchive;

mod brand;
mod i18n;
mod inspect;
mod mapgen;
mod notices;
mod sprites;

#[derive(Parser)]
#[command(name = "ta-tools", about = "Inspect and convert Total Annihilation data")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
    /// Mounts a mod (its folder with a mod.toml, or a TA archive) over the game; repeatable,
    /// in load order.
    #[arg(long = "mod", global = true)]
    mods: Vec<PathBuf>,
}

#[derive(Subcommand)]
enum Cmd {
    /// List the files inside an HPI-family archive.
    Ls { archive: PathBuf },
    /// Extract an archive (or files matching a substring) to a directory.
    Extract {
        archive: PathBuf,
        out: PathBuf,
        #[arg(long)]
        filter: Option<String>,
    },
    /// Decompress every file of every archive in a directory, reporting failures.
    Verify { dir: PathBuf },
    /// Show files that appear in more than one archive of a game directory.
    Overlaps { dir: PathBuf },
    /// Locate the TA install and summarise the mounted virtual file system.
    Info,
    /// List VFS files under a directory (optionally filtered by extension).
    Find {
        dir: String,
        #[arg(long)]
        ext: Option<String>,
        #[arg(long, short)]
        recursive: bool,
    },
    /// Print a VFS text file (TDF/FBI/GUI/OTA...) to stdout, or save a binary file.
    Cat {
        path: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Parse every GAF/3DO/COB/TNT/PCX/TDF file in the VFS and report failures.
    Validate,
    /// Check a mod as the launcher does: its content over the original game, Reborn's own
    /// and the `--mod`s (its dependencies), compared with the same without it, so what it
    /// breaks in inherited files counts and what was broken before does not. Prints what
    /// counts (`--inherited`: everything), or the report as JSON; exits 1 when a finding of
    /// `--fail-on`'s severity or worse counts against it.
    ValidateMod {
        /// The mod's folder (with or without a mod.toml) or a TA archive.
        path: PathBuf,
        #[arg(long)]
        json: bool,
        /// error (the default), warning, or never.
        #[arg(long, default_value = "error")]
        fail_on: String,
        /// Also list findings there without the mod (the original game's, a dependency's).
        #[arg(long)]
        inherited: bool,
    },
    /// Dump every frame of a GAF to PNG files.
    Gaf { path: String, out: PathBuf },
    /// Render a map (by name, e.g. "Green Hills") to PNGs plus feature statistics.
    Map { name: String, out: PathBuf },
    /// Print the piece hierarchy of a 3DO model (`--obj <dir>`: also write it, its materials and
    /// textures there as OBJ/MTL/PNG to look at in a 3D program).
    Model {
        path: String,
        #[arg(long)]
        obj: Option<PathBuf>,
    },
    /// Disassemble a COB unit script.
    Cob { path: String },
    /// Load all typed game data and cross-check references (weapons, models, scripts, menus).
    Data {
        /// Print one unit's parsed definition and build menu.
        #[arg(long)]
        unit: Option<String>,
        /// Print every builder's build menu, one builder a line.
        #[arg(long)]
        menus: bool,
    },
    /// Write an original-art-free unit/texture reference for skin creators, including mods.
    SkinCatalogue { out: PathBuf },
    /// List campaigns and check every mission: goals, placed units, scripts, unit names
    Missions,
    /// Draw TA Reborn's own art (the icon, the installer's pictures) into a directory, from the
    /// emblem tools/branding/emblem.py draws.
    Brand { out: PathBuf },
    /// Make one of TA Reborn's own maps (or `all`) into a folder holding `maps/` (the game's
    /// built-in maps are `assets/reborn`), with pictures of it in `--preview`.
    Mapgen {
        id: String,
        out: PathBuf,
        #[arg(long)]
        preview: Option<PathBuf>,
    },
    /// Pack paintings (RGBA PNGs, listed in a `manifest.json` as tools/features/sprites.py
    /// writes it) into a GAF in TA's palette: TA Reborn's own feature art.
    GafPack { manifest: PathBuf, out: PathBuf },
    /// Collect the interface's translatable text into <dir>/template.toml and add new
    /// strings to the languages there (run from the repository root).
    I18n { dir: PathBuf },
    /// Install a downloaded mod package (zip, rar, 7z) into a mods folder, as the launcher
    /// does: unpack, pick out the TA content, write the mod folder and its mod.toml.
    ModInstall {
        package: PathBuf,
        mods_dir: PathBuf,
        /// The mod's id (default: the package's name).
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        name: Option<String>,
    },
    /// Write the notices of the third-party libraries compiled into the game and the
    /// launcher, with their licence texts (run from the repository root).
    Notices {
        out: PathBuf,
        /// Target triple the programs are built for.
        #[arg(long, default_value = "x86_64-pc-windows-msvc")]
        target: String,
    },
}

/// Mods to mount over the game (`--mod`), set once from the command line.
static MODS: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();

/// Discovers the install and mounts it, Reborn's own content and the mods asked for (`--mod`, in
/// the order given, each read as the game reads it from a mods folder), as the game mounts them:
/// what the mods replace of the install left out.
fn open_vfs() -> Result<(ta_vfs::install::TaInstall, ta_vfs::Vfs)> {
    let install = ta_vfs::install::discover(None)?;
    let mods = given_mods()?;
    // (Each mod's layer named after its folder or archive.)
    let layer = |m: &ta_vfs::mods::ModInfo| m.path.file_stem().map_or(String::new(), |s| s.to_string_lossy().to_string());
    let mounted = ta_vfs::mods::mount(Some(&install.root), ta_vfs::builtin_dir().as_deref(), &mods.iter().collect::<Vec<_>>(), &layer);
    mounted.install?;
    if let Err(e) = mounted.builtin { eprintln!("built-in content: {e}"); }
    mounted.mods.map_err(|e| anyhow::anyhow!("{e}"))?;
    if !mounted.replaced.is_empty() {
        eprintln!("the mods replace the install's {}", mounted.replaced.join(", "));
    }
    Ok((install, mounted.vfs))
}

/// The mods given with `--mod`, read as the game reads them from a mods folder: one whose
/// manifest does not read is refused, as the game leaves it out.
fn given_mods() -> Result<Vec<ta_vfs::mods::ModInfo>> {
    MODS.get().into_iter().flatten().map(|p| {
        ta_vfs::mods::read_mod(p).map_err(|e| anyhow::anyhow!("--mod {}: the game leaves this mod out: {e}", p.display()))?
            .with_context(|| format!("--mod {} is neither a folder nor a TA archive", p.display()))
    }).collect()
}

/// What `validate-mod` mounts with the mod: the mods given with `--mod` and the mod, in the order
/// the game would mount them together.
#[derive(Debug)]
struct ModPlan {
    order: Vec<ta_vfs::mods::ModInfo>,
    /// What the mod needs, however indirectly (what its dependencies need too), not given.
    missing: Vec<String>,
    /// Why the game would not load them together, all being given (a cycle, nesting too deep, a
    /// given mod lacking one of its own).
    problem: Option<String>,
}

fn mod_plan(target: &ta_vfs::mods::ModInfo, given: &[ta_vfs::mods::ModInfo]) -> ModPlan {
    use std::collections::{BTreeMap, BTreeSet};
    let mut mods: Vec<ta_vfs::mods::ModInfo> = given.iter().filter(|m| m.manifest.id != target.manifest.id).cloned().collect();
    let by_id: BTreeMap<&str, &ta_vfs::mods::ModInfo> = mods.iter().map(|m| (m.manifest.id.as_str(), m)).collect();
    let (mut seen, mut missing) = (BTreeSet::new(), BTreeSet::new());
    let mut pending: Vec<String> = target.manifest.depends.clone();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) { continue; }
        match by_id.get(id.as_str()) {
            Some(m) => pending.extend(m.manifest.depends.iter().cloned()),
            // (Needing the mod itself is a cycle: the order says so.)
            None if id == target.manifest.id => {}
            None => { missing.insert(id); }
        }
    }
    mods.push(target.clone());
    if !missing.is_empty() {
        // (Mounted as given, the check incomplete.)
        return ModPlan { order: mods, missing: missing.into_iter().collect(), problem: None };
    }
    let enabled: Vec<String> = mods.iter().map(|m| m.manifest.id.clone()).collect();
    match ta_vfs::mods::resolve_load_order(&enabled, &mods) {
        Ok(order) => ModPlan { order: order.into_iter().cloned().collect(), missing: Vec::new(), problem: None },
        Err(error) => ModPlan { order: mods, missing: Vec::new(), problem: Some(error.to_string()) },
    }
}

/// The install at `install`, TA Reborn's own content and `mods` (in that order, each as the layer
/// `layer` names), as the game mounts them: what the mods replace of the install left out.
fn game_vfs(install: &Path, mods: &[&ta_vfs::mods::ModInfo], layer: &dyn Fn(&ta_vfs::mods::ModInfo) -> String) -> Result<ta_vfs::mods::Mounted> {
    let mut mounted = ta_vfs::mods::mount(Some(install), ta_vfs::builtin_dir().as_deref(), mods, layer);
    std::mem::replace(&mut mounted.install, Ok(()))?;
    if let Err(e) = std::mem::replace(&mut mounted.builtin, Ok(())) { eprintln!("built-in content: {e}"); }
    Ok(mounted)
}

/// `ta-tools validate-mod`: see [`Cmd::ValidateMod`]. Whether it failed.
fn validate_mod(path: &Path, json: bool, fail_on: &str, inherited: bool) -> Result<bool> {
    use ta_data::validation::{self, Options, Severity};
    let fail = match fail_on {
        "error" => Some(Severity::Error),
        "warning" => Some(Severity::Warning),
        "never" => None,
        other => anyhow::bail!("--fail-on takes error, warning or never, not {other}"),
    };
    // The mod as the game reads it from a mods folder: one whose manifest does not read is left
    // out of the game, and so is refused here.
    let module = ta_vfs::mods::read_mod(path).map_err(|e| anyhow::anyhow!("the game leaves this mod out: {e}"))?
        .with_context(|| format!("{} is neither a folder nor a TA archive ({}): the game would not load it", path.display(), ta_vfs::ARCHIVE_EXTENSIONS.join(", ")))?;
    // The same layer name as the launcher's.
    let layer = format!("mod:{}", module.manifest.id);
    // The mods given with --mod, read as the game reads them, and mounted with the mod in the
    // order the game would. A dependency not given (its dependencies' too): what the mod uses of
    // it reads as missing, so the check is incomplete (as the launcher's is when one is not
    // installed).
    let given = given_mods()?;
    let plan = mod_plan(&module, &given);
    let target = |m: &ta_vfs::mods::ModInfo| m.manifest.id == module.manifest.id;
    // (Each mounted as the game mounts them, what they replace of the install left out: without
    // the mod, its dependencies' replacements alone.)
    let dependencies: Vec<&ta_vfs::mods::ModInfo> = plan.order.iter().filter(|m| !target(m)).collect();
    let install = ta_vfs::install::discover(None)?.root;
    let baseline_vfs = game_vfs(&install, &dependencies, &|m| m.manifest.id.clone())?;
    baseline_vfs.mods.map_err(|e| anyhow::anyhow!("{e}"))?;
    eprintln!("Checking the original game without the mod…");
    let baseline = validation::validate(&baseline_vfs.vfs, &Options::default(), |_| true);
    // Archives the game would skip, as the launcher reports them: the mod's own count.
    let unreadable = |archive: &Path, error: String| validation::Issue {
        severity: Severity::Error, code: "ARCHIVE_UNREADABLE".into(), layer: layer.clone(),
        path: archive.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()),
        message: format!("The game skips this archive because it cannot be read: {error}"),
        suggestion: "Replace the corrupt archive. Remove it only if it is an intentional empty placeholder.".into(), ..Default::default()
    };
    let mut archives = Vec::new();
    if path.is_dir() {
        // (An empty placeholder named like one of the install's archives blanks that one on
        // purpose: no archive the game fails to read.)
        let replaced = ta_vfs::mods::replaced_archives(&install, &[&module]);
        for entry in std::fs::read_dir(path)?.flatten() {
            let file = entry.path();
            let archive = file.extension().and_then(|e| e.to_str()).is_some_and(|e| ta_vfs::ARCHIVE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
            let blank = || file.file_name().is_some_and(|n| replaced.iter().any(|r| r.eq_ignore_ascii_case(&n.to_string_lossy()))) && ta_vfs::package::is_placeholder(&file);
            if archive && file.is_file() && let Err(error) = ta_formats::hpi::HpiArchive::open(&file) && !blank() { archives.push(unreadable(&file, error.to_string())); }
        }
    }
    let order: Vec<&ta_vfs::mods::ModInfo> = plan.order.iter().collect();
    let mounted = game_vfs(&install, &order, &|m| if target(m) { layer.clone() } else { m.manifest.id.clone() })?;
    let unopened = match mounted.mods {
        Ok(_) => None,
        // One archive the game cannot open: that is the whole report (the reader's own words,
        // without the folder).
        Err(ta_vfs::mods::ModError::Unreadable { id, message }) if id == module.manifest.id && module.kind == ta_vfs::mods::ModKind::Archive => {
            Some(ta_formats::hpi::HpiArchive::open(path).err().map_or(message, |e| e.to_string()))
        }
        Err(error) => anyhow::bail!("{error}"),
    };
    if !mounted.replaced.is_empty() {
        eprintln!("With it on, the install's {} are left out: it replaces them.", mounted.replaced.join(", "));
    }
    let vfs = mounted.vfs;
    let opened = unopened.is_none();
    let mut report = match unopened {
        Some(error) => validation::Report { target_layer: Some(layer.clone()), complete: true, issues: vec![unreadable(path, error)], ..Default::default() },
        None => {
            eprintln!("Checking {}…", path.display());
            validation::validate(&vfs, &Options { target_layer: Some(layer.clone()), ..Options::default() }, |_| true)
        }
    };
    // (A folder whose content is all in archives the game cannot open: those are its findings.)
    validation::content_unreadable(&mut report, &archives);
    report.issues.extend(archives);
    for dependency in &plan.missing {
        report.complete = false;
        report.issues.push(validation::Issue {
            severity: Severity::Error, code: "DEPENDENCY".into(), layer: layer.clone(),
            message: format!("Required mod '{dependency}' was not given with --mod (this mod needs it, or a mod it needs does): what this mod uses of it reads as missing, and the check is incomplete."),
            suggestion: "Give each mod it depends on with --mod <its folder or archive>, then check again.".into(), ..Default::default()
        });
    }
    if let Some(problem) = &plan.problem {
        report.complete = false;
        report.issues.push(validation::Issue {
            severity: Severity::Error, code: "DEPENDENCY".into(), layer: layer.clone(),
            message: format!("The game would not load these mods together: {problem}"),
            suggestion: "Give every mod the others need with --mod, and break any circle of mods needing each other, then check again.".into(), ..Default::default()
        });
    }
    // (Told apart only from a whole check of the content without it: against part of one,
    // what the rest has would all look the mod's doing.)
    if baseline.complete { validation::compare(&mut report, &baseline); } else { eprintln!("The check without the mod did not complete: what the mod breaks in inherited content is not told apart."); }
    // (Which of the original game's archives it takes the place of, mounted: notes.)
    if opened {
        report.issues.extend(ta_vfs::mods::replaced_archives(&install, &[&module]).iter().map(|archive| validation::replaced_archive(&layer, archive)));
    }
    if !inherited { report.issues.retain(|i| i.affects_status); }
    if json { println!("{}", serde_json::to_string_pretty(&report)?); } else { print!("{}", report.to_text()); }
    // (`never`: whatever it found, an incomplete check too.)
    let failed = fail.is_some_and(|worst| !report.complete || report.issues.iter().any(|i| i.affects_status && match worst {
        Severity::Warning => matches!(i.severity, Severity::Error | Severity::Warning),
        _ => i.severity == Severity::Error,
    }));
    Ok(failed)
}

/// `ta-tools mod-install`: see [`Cmd::ModInstall`].
fn mod_install(package: &Path, mods_dir: &Path, id: Option<String>, name: Option<String>) -> Result<()> {
    let stem = package.file_stem().map_or(String::new(), |s| s.to_string_lossy().to_string());
    if let Some(id) = &id { anyhow::ensure!(ta_vfs::package::safe_component(id), "mod id must be a single ordinary directory name"); }
    let unpacker = ta_vfs::package::Unpacker::find().context("no unpacker (7-Zip or tar) found")?;
    // Neither a supplied id nor an abandoned directory may choose what gets deleted.
    // Exclusively create a fresh workspace, independent of the package's filenames.
    let temp = loop {
        let path = std::env::temp_dir().join(format!("ta-mod-{}", ta_config::profiles::new_id()));
        match std::fs::create_dir(&path) {
            Ok(()) => break path,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    };
    // (Removed however the install ends.)
    struct Workspace(PathBuf);
    impl Drop for Workspace { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    let _workspace = Workspace(temp.clone());
    let t0 = std::time::Instant::now();
    unpacker.unpack(package, &temp)?;
    println!("unpacked with {} in {:.1} s", unpacker.name(), t0.elapsed().as_secs_f32());
    let plan = ta_vfs::package::plan(&temp)?;
    println!("content in: {}", plan.root.strip_prefix(&temp).unwrap_or(&plan.root).display());
    for a in &plan.archives {
        println!("  archive {} ({} KB)", a.file_name().unwrap_or_default().to_string_lossy(), std::fs::metadata(a).map_or(0, |m| m.len()) / 1024);
    }
    for f in &plan.folders {
        println!("  folder  {}/", f.file_name().unwrap_or_default().to_string_lossy());
    }
    for d in &plan.docs {
        println!("  doc     {}", d.file_name().unwrap_or_default().to_string_lossy());
    }
    for (from, to) in &plan.rename {
        println!("  rename  {from}/ -> {to}/");
    }
    for name in &plan.replaces {
        println!("  replaces {name} (an empty placeholder: the original game's is left out with the mod on)");
    }
    println!("  {} units, {} maps; skipped {}: {}", plan.units, plan.maps, plan.skipped.len(), plan.skipped.join(", "));
    // A package laid out as a mod already keeps its own manifest (its id, what it needs), as the
    // launcher installs it; --id and --name still choose.
    let own = [plan.root.as_path(), temp.as_path()].into_iter().find(|dir| dir.join("mod.toml").is_file())
        .and_then(|dir| ta_vfs::mods::read_mod(dir).ok().flatten()).map(|m| m.manifest);
    let mut manifest = own.unwrap_or_else(|| ta_vfs::mods::ModManifest {
        id: stem.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect(),
        name: stem.clone(),
        version: String::new(),
        description: String::new(),
        authors: Vec::new(),
        depends: Vec::new(),
        load_after: Vec::new(),
        source: String::new(),
        rename: Default::default(),
        replaces: Vec::new(),
    });
    if let Some(id) = id { manifest.id = id; }
    if let Some(name) = name { manifest.name = name; }
    anyhow::ensure!(ta_vfs::package::safe_component(&manifest.id), "mod id must be a single ordinary directory name");
    let dir = ta_vfs::package::install(&plan, &manifest, mods_dir)?;
    println!("installed: {}", dir.display());
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let _ = MODS.set(cli.mods);
    match cli.cmd {
        Cmd::ModInstall { package, mods_dir, id, name } => mod_install(&package, &mods_dir, id, name)?,
        Cmd::Brand { out } => brand::run(&out)?,
        Cmd::Mapgen { id, out, preview } => {
            let (_, vfs) = open_vfs()?;
            let palette = inspect::palette(&vfs)?;
            let ids: Vec<&str> = if id == "all" { mapgen::IDS.to_vec() } else { vec![id.as_str()] };
            for id in ids {
                mapgen::run(id, &out, preview.as_deref(), &palette)?;
            }
        }
        Cmd::GafPack { manifest, out } => {
            let (_, vfs) = open_vfs()?;
            sprites::pack(&manifest, &out, &inspect::palette(&vfs)?)?;
        }
        Cmd::I18n { dir } => i18n::run(&dir)?,
        Cmd::Notices { out, target } => notices::run(&out, &target)?,
        Cmd::Ls { archive } => {
            let a = HpiArchive::open(&archive)?;
            for f in a.files() {
                println!("{:>10} {:?} {}", f.size, f.compression, f.path);
            }
            eprintln!("{} files, encrypted={}", a.files().len(), a.is_encrypted());
        }
        Cmd::Extract { archive, out, filter } => {
            let a = HpiArchive::open(&archive)?;
            let filter = filter.map(|f| f.to_ascii_lowercase());
            let mut n = 0;
            for f in a.files() {
                if let Some(flt) = &filter && !f.path.to_ascii_lowercase().contains(flt.as_str()) {
                    continue;
                }
                let data = a.read(f).with_context(|| format!("reading {}", f.path))?;
                extract_file(&out, &f.path, &data)?;
                n += 1;
            }
            eprintln!("extracted {n} files");
        }
        Cmd::Verify { dir } => {
            use rayon::prelude::*;
            for path in archives_in(&dir)? {
                let a = HpiArchive::open(&path)?;
                let failures: Vec<String> = a
                    .files()
                    .par_iter()
                    .filter_map(|f| a.read(f).err().map(|e| format!("  {}: {e}", f.path)))
                    .collect();
                let total: u64 = a.files().iter().map(|f| f.size as u64).sum();
                println!(
                    "{:<16} {:>6} files {:>10} KiB  enc={:<5} failures={}",
                    path.file_name().unwrap().to_string_lossy(),
                    a.files().len(),
                    total / 1024,
                    a.is_encrypted(),
                    failures.len()
                );
                for f in failures.iter().take(10) {
                    println!("{f}");
                }
            }
        }
        Cmd::Overlaps { dir } => {
            // file -> [(archive name, content hash)]
            let mut owners: BTreeMap<String, Vec<(String, u64)>> = BTreeMap::new();
            for path in archives_in(&dir)? {
                let a = HpiArchive::open(&path)?;
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                for f in a.files() {
                    let hash = fnv1a(&a.read(f)?);
                    owners.entry(f.path.to_ascii_lowercase()).or_default().push((name.clone(), hash));
                }
            }
            let mut by_combo: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
            for (file, versions) in owners.iter().filter(|(_, v)| v.len() > 1) {
                let names: Vec<&str> = versions.iter().map(|(n, _)| n.as_str()).collect();
                let entry = by_combo.entry(names.join(" + ")).or_default();
                if versions.iter().all(|(_, h)| *h == versions[0].1) {
                    entry.0 += 1;
                } else {
                    // Group archives by identical content, e.g. "totala1.hpi=ccdata.ccx | rev31.gp3".
                    let mut groups: Vec<(u64, Vec<&str>)> = Vec::new();
                    for (n, h) in versions {
                        match groups.iter_mut().find(|(gh, _)| gh == h) {
                            Some((_, g)) => g.push(n),
                            None => groups.push((*h, vec![n])),
                        }
                    }
                    let desc: Vec<String> = groups.iter().map(|(_, g)| g.join("=")).collect();
                    entry.1.push(format!("{file}  [{}]", desc.join(" | ")));
                }
            }
            for (combo, (identical, differing)) in by_combo {
                println!("{combo}: {identical} identical, {} differ", differing.len());
                for f in differing.iter().take(6) {
                    println!("    {f}");
                }
            }
        }
        Cmd::Info => {
            let (install, vfs) = open_vfs()?;
            println!("install: {} (via {})", install.root.display(), install.source);
            println!("store: {:?}, build {:?}", install.store, install.build);
            println!("content: {:?}", install.content);
            println!("{} files from {} sources", vfs.file_count(), vfs.sources().count());
            for d in vfs.subdirs("") {
                println!("  {d:<16} {:>6} files", vfs.list(&d, None, true).len());
            }
            // Mods their installers put in the game's folder: left out of the original game.
            for m in ta_vfs::mods::game_folder_mods(&install.root) {
                let archives: Vec<String> = m.archives.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
                let folders: Vec<String> = m.rename.iter().map(|(from, to)| format!("{from} as {to}")).collect();
                println!("mod in the game's folder, left out: {} (id {}): {}; reads {}", m.name, m.id, archives.join(", "), folders.join(", "));
            }
        }
        Cmd::Find { dir, ext, recursive } => {
            let (_, vfs) = open_vfs()?;
            for f in vfs.list(&dir, ext.as_deref(), recursive) {
                let origin = vfs.origin(&f).map(|o| o.path.file_name().unwrap_or_default().to_string_lossy().to_string());
                println!("{f}  <- {}", origin.unwrap_or_default());
            }
        }
        Cmd::Validate => inspect::validate(&open_vfs()?.1)?,
        Cmd::ValidateMod { path, json, fail_on, inherited } => {
            if validate_mod(&path, json, &fail_on, inherited)? { std::process::exit(1); }
        }
        Cmd::Gaf { path, out } => inspect::dump_gaf(&open_vfs()?.1, &path, &out)?,
        Cmd::Map { name, out } => inspect::dump_map(&open_vfs()?.1, &name, &out)?,
        Cmd::Model { path, obj } => {
            let vfs = open_vfs()?.1;
            inspect::dump_model(&vfs, &path)?;
            if let Some(dir) = obj {
                inspect::export_model_obj(&vfs, &path, &dir)?;
            }
        }
        Cmd::Cob { path } => inspect::dump_script(&open_vfs()?.1, &path)?,
        Cmd::Data { unit, menus } => inspect::check_data(&open_vfs()?.1, unit.as_deref(), menus)?,
        Cmd::SkinCatalogue { out } => {
            let reference = ta_data::skin_catalogue(&open_vfs()?.1);
            // Author guides must not silently replace a file the player has edited.
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&out)
                .with_context(|| format!("create {} (the destination must not exist)", out.display()))?;
            file.write_all(reference.as_bytes())?;
            println!("Skin reference: {}", out.display());
        }
        Cmd::Missions => inspect::check_missions(&open_vfs()?.1)?,
        Cmd::Cat { path, out } => {
            let (_, vfs) = open_vfs()?;
            let data = vfs.read(&path)?;
            match out {
                Some(o) => std::fs::write(o, data)?,
                None => print!("{}", data.iter().map(|&b| b as char).collect::<String>()),
            }
        }
    }
    Ok(())
}

/// Never follow package-controlled links in an existing extraction tree. Replace files
/// through a new temporary inode so an existing hard link cannot overwrite its target.
fn extract_file(root: &Path, name: &str, data: &[u8]) -> Result<()> {
    use std::io::Write;
    let normalized = name.replace('\\', "/");
    let parts: Vec<_> = normalized.split('/').collect();
    anyhow::ensure!(parts.iter().all(|p| ta_vfs::package::safe_component(p)), "unsafe archive path");
    let ordinary = |path: &Path| -> Result<()> {
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                anyhow::ensure!(meta.file_attributes() & 0x400 == 0, "linked extraction path: {}", path.display());
            }
            anyhow::ensure!(!meta.file_type().is_symlink(), "linked extraction path: {}", path.display());
        }
        Ok(())
    };
    std::fs::create_dir_all(root)?;
    ordinary(root)?;
    let mut dir = root.to_path_buf();
    for component in &parts[..parts.len() - 1] {
        dir.push(component);
        ordinary(&dir)?;
        if !dir.exists() { std::fs::create_dir(&dir)?; }
    }
    let destination = dir.join(parts.last().unwrap());
    ordinary(&destination)?;
    let temporary = dir.join(format!(".{}.{}.extracting", parts.last().unwrap(), std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    let result = file.write_all(data).and_then(|()| { drop(file); std::fs::rename(&temporary, &destination) });
    if result.is_err() { let _ = std::fs::remove_file(&temporary); }
    result?;
    Ok(())
}

fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod mod_plan_tests {
    use super::*;

    fn m(id: &str, depends: &[&str]) -> ta_vfs::mods::ModInfo {
        let manifest = ta_vfs::mods::ModManifest {
            id: id.into(), name: id.into(), version: String::new(), description: String::new(), authors: Vec::new(),
            depends: depends.iter().map(|d| d.to_string()).collect(), load_after: Vec::new(), source: String::new(), rename: Default::default(), replaces: Vec::new(),
        };
        ta_vfs::mods::ModInfo { manifest, path: PathBuf::from(id), kind: ta_vfs::mods::ModKind::Directory }
    }

    fn ids(plan: &ModPlan) -> Vec<&str> { plan.order.iter().map(|m| m.manifest.id.as_str()).collect() }

    /// The mod needs b, which needs c: given b alone, c is missing (the game would refuse them),
    /// and given both, they are mounted in the game's order, whatever order they were given in.
    #[test]
    fn a_mod_is_checked_with_the_whole_chain_it_needs_in_the_games_order() {
        let target = m("t", &["b"]);
        let plan = mod_plan(&target, &[m("b", &["c"])]);
        assert_eq!(plan.missing, ["c"]);
        let plan = mod_plan(&target, &[m("b", &["c"]), m("c", &[])]);
        assert!(plan.missing.is_empty() && plan.problem.is_none(), "{plan:?}");
        assert_eq!(ids(&plan), ["c", "b", "t"]);
        // Mods needing each other: no order the game can make.
        let plan = mod_plan(&m("t", &["b"]), &[m("b", &["t"])]);
        assert!(plan.problem.as_deref().is_some_and(|p| p.contains("cycle")), "{plan:?}");
        // A given mod lacking one of its own: the game would not load them together either.
        let plan = mod_plan(&m("t", &[]), &[m("x", &["y"])]);
        assert!(plan.problem.is_some(), "{plan:?}");
        // Nothing needed: the mod alone.
        assert_eq!(ids(&mod_plan(&m("t", &[]), &[])), ["t"]);
    }
}

#[cfg(test)]
mod extraction_tests {
    use super::*;

    #[test]
    fn mod_install_rejects_unsafe_ids_before_unpacking_or_touching_temp_paths() {
        for id in ["../victim", "nested/../../victim", "x:stream", "NUL", ""] {
            let result = mod_install(Path::new("nonexistent.zip"), Path::new("unused"), Some(id.into()), None);
            assert!(result.unwrap_err().to_string().contains("mod id"), "{id}");
        }
    }

    #[test]
    fn extraction_does_not_write_through_paths_or_hardlinks() {
        let root = std::env::temp_dir().join(format!("ta_extract_security_{}", std::process::id()));
        let out = root.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let canary = root.join("canary");
        std::fs::write(&canary, b"keep").unwrap();
        for path in ["../canary", "..\\canary", "C:/canary", "NUL", "x:stream"] {
            assert!(extract_file(&out, path, b"bad").is_err());
        }
        std::fs::hard_link(&canary, out.join("linked")).unwrap();
        extract_file(&out, "linked", b"new").unwrap();
        assert_eq!(std::fs::read(&canary).unwrap(), b"keep");
        assert_eq!(std::fs::read(out.join("linked")).unwrap(), b"new");
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&root, out.join("escape"));
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(&root, out.join("escape"));
        if linked.is_ok() {
            assert!(extract_file(&out, "escape/canary", b"bad").is_err());
            assert_eq!(std::fs::read(&canary).unwrap(), b"keep");
            std::fs::remove_dir(out.join("escape")).or_else(|_| std::fs::remove_file(out.join("escape"))).unwrap();
        } else {
            eprintln!("symlink test unavailable without platform symlink permission");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

fn archives_in(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                matches!(e.to_ascii_lowercase().as_str(), "hpi" | "ufo" | "ccx" | "gp3" | "gpf")
            })
        })
        .collect();
    v.sort();
    Ok(v)
}
