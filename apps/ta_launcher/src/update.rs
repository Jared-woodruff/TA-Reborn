//! Updates: the update source is asked for the newest release on a background thread and its
//! version compared with this build's. A release with a setup is downloaded into the cache,
//! checked against the release's own list of SHA-256 sums and run ([`Download`], [`run_setup`]:
//! with `/UPDATE` only its progress shows, and it starts the launcher again), and the launcher
//! closes for it; a copy Setup did not install is shown the release's page instead.
//!
//! The source is a GitHub "latest release" API address ([`DEFAULT_SOURCE`]: this project's) or a
//! small TOML file (`version = "0.4.0"`, `url = "…"`, optional `title` and `notes`, and `setup`
//! and `sums` HTTPS addresses). An explicitly configured local `file:` source may also name
//! local assets, for testing a whole update. It is fetched with the
//! system's curl (part of Windows 10 and later, and of every desktop Linux), so the launcher
//! carries no network or TLS code of its own.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ta_ui::i18n::{t, tf};

use crate::moddb::{Stopper, Transfer};

/// Where the launcher looks for updates, unless `[updates] url` in config.toml says otherwise:
/// this project's releases on GitHub. (While the repository is not public, GitHub answers that
/// there is nothing there: [`UpdateState::Unpublished`].)
pub const DEFAULT_SOURCE: &str = "https://api.github.com/repos/Jared-woodruff/TA-Reborn/releases/latest";

/// A file of a release.
#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    /// In bytes (0: not given).
    pub size: u64,
}

/// The newest release, as the source describes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Release {
    /// As the source names it ("v2.2.0").
    pub version: String,
    /// Its page.
    pub url: String,
    /// Its title ("TA Reborn 2.2.0").
    pub title: String,
    /// What it brings, in Markdown (the release's notes).
    pub notes: String,
    /// The setup that installs it, and the list of SHA-256 sums it is checked against.
    pub setup: Option<Asset>,
    pub sums: Option<Asset>,
    /// True only when the user explicitly configured a local file as the update source.
    /// Remote release metadata cannot opt into reading local files.
    pub local_files: bool,
}

impl Release {
    /// The version as it is read: "2.2.0".
    pub fn number(&self) -> &str {
        self.version.trim().trim_start_matches(['v', 'V'])
    }

    /// Offline updates may only read assets from an explicitly local source.
    pub fn available_offline(&self) -> bool {
        self.local_files && [&self.setup, &self.sums].iter().all(|asset| asset.as_ref().is_some_and(|asset| {
            crate::http::address(&asset.url, crate::http::Policy::Update { local_files: true }).is_ok_and(|url| url.starts_with("file:"))
        }))
    }

    /// The first paragraph of its notes (what it is about), its lines joined.
    pub fn summary(&self) -> String {
        let mut summary = String::new();
        for line in self.notes.lines().map(str::trim) {
            if line.is_empty() && !summary.is_empty() || line.starts_with('#') || line.starts_with("- ") {
                break;
            }
            if !line.is_empty() {
                if !summary.is_empty() {
                    summary.push(' ');
                }
                summary.push_str(line);
            }
        }
        summary
    }
}

pub enum UpdateState {
    /// Checks are switched off.
    Off,
    /// No update source is set.
    NoSource,
    Checking(Receiver<Result<Option<Release>, String>>),
    UpToDate,
    /// The source has no release to offer (a GitHub repository with none, or not public).
    Unpublished,
    Available(Release),
    Failed(String),
}

impl UpdateState {
    /// Starts a check of `source` (or explains why none runs).
    pub fn start(enabled: bool, source: &str) -> UpdateState {
        let source = if source.trim().is_empty() { DEFAULT_SOURCE } else { source.trim() };
        if !enabled {
            return UpdateState::Off;
        }
        if source.is_empty() {
            return UpdateState::NoSource;
        }
        let (tx, rx) = mpsc::channel();
        let source = source.to_string();
        std::thread::spawn(move || {
            let _ = tx.send(fetch(&source, true).and_then(|answer| answer.map(|text| {
                let mut release = parse(&text)?;
                release.local_files = crate::http::address(&source, crate::http::Policy::Update { local_files: true })
                    .is_ok_and(|url| url.starts_with("file:"));
                Ok(release)
            }).transpose()));
        });
        UpdateState::Checking(rx)
    }

    /// Picks up a finished check. True when the state changed.
    pub fn poll(&mut self) -> bool {
        let UpdateState::Checking(rx) = self else { return false };
        let next = match rx.try_recv() {
            Ok(Ok(Some(release))) if newer(&release.version, env!("CARGO_PKG_VERSION")) => UpdateState::Available(release),
            Ok(Ok(Some(_))) => UpdateState::UpToDate,
            Ok(Ok(None)) => UpdateState::Unpublished,
            Ok(Err(e)) => UpdateState::Failed(e),
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => UpdateState::Failed(t("the check stopped").into()),
        };
        match &next {
            UpdateState::Available(r) => log::info!("update available: {} ({})", r.version, r.url),
            UpdateState::Unpublished => log::info!("update check: no release is published"),
            UpdateState::Failed(e) => log::warn!("update check: {e}"),
            _ => log::info!("update check: up to date"),
        }
        *self = next;
        true
    }

    /// A line for the About page.
    pub fn describe(&self) -> String {
        let here = env!("CARGO_PKG_VERSION");
        match self {
            UpdateState::Off => t("Update checks are off").into(),
            UpdateState::NoSource => t("This build names no update source, so the launcher does not look for new versions").into(),
            UpdateState::Checking(_) => t("Checking for updates…").into(),
            UpdateState::UpToDate => tf("Up to date (v{version})", &[("version", &here)]),
            UpdateState::Unpublished => tf("No release is published yet (this is v{version})", &[("version", &here)]),
            UpdateState::Available(r) => tf("Version {new} is out (this is v{version})", &[("new", &display_version(&r.version)), ("version", &here)]),
            UpdateState::Failed(e) => tf("Could not check for updates: {error}", &[("error", e)]),
        }
    }
}

/// The source's answer, or None when there is nothing there (HTTP 404: a GitHub repository
/// with no release, or one that is not public).
fn fetch(source: &str, local_files: bool) -> Result<Option<String>, String> {
    let policy = crate::http::Policy::Update { local_files };
    let source = crate::http::address(source, policy)?;
    let mut command = crate::http::curl(policy);
    command.args(["--max-time", "15", "--url"]).arg(source);
    let out = crate::http::text_output(command)?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim().trim_start_matches("curl: ");
        if not_found(err) {
            return Ok(None);
        }
        return Err(if err.is_empty() { tf("curl failed ({status})", &[("status", &out.status)]) } else { err.to_string() });
    }
    String::from_utf8(out.stdout).map(Some).map_err(|_| t("the answer is not text").into())
}

/// Whether curl's error is HTTP's "not found".
fn not_found(err: &str) -> bool {
    err.contains("error: 404")
}

/// Reads a release from a GitHub API answer (JSON) or a TOML file.
pub fn parse(text: &str) -> Result<Release, String> {
    parse_for(text, UpdateTarget::current())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateTarget { WindowsX64, WindowsArm64, Manual }

impl UpdateTarget {
    fn current() -> Self {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "x86_64") => Self::WindowsX64,
            ("windows", "aarch64") => Self::WindowsArm64,
            _ => Self::Manual,
        }
    }
}

/// Exact names prevent asset order (or another platform's installer) deciding what runs.
/// The old unsuffixed setup is the Windows x64 compatibility alias only.
fn release_setup(assets: &[Asset], version: &str, target: UpdateTarget) -> Option<Asset> {
    let version = version.trim().trim_start_matches(['v', 'V']);
    let find = |name: &str| assets.iter().find(|a| a.name.eq_ignore_ascii_case(name)).cloned();
    match target {
        UpdateTarget::WindowsX64 => find(&format!("TA-Reborn-{version}-windows-x64-setup.exe"))
            .or_else(|| find(&format!("TA-Reborn-{version}-setup.exe"))),
        UpdateTarget::WindowsArm64 => find(&format!("TA-Reborn-{version}-windows-arm64-setup.exe")),
        UpdateTarget::Manual => None,
    }
}

fn parse_for(text: &str, target: UpdateTarget) -> Result<Release, String> {
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.starts_with('{') {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| tf("unreadable answer: {error}", &[("error", &e)]))?;
        let field = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
        let version = field("tag_name");
        if version.is_empty() {
            let message = field("message");
            return Err(if message.is_empty() { t("the answer names no release").into() } else { tf("the source answered “{message}”", &[("message", &message)]) });
        }
        let assets: Vec<Asset> = v
            .get("assets")
            .and_then(|a| a.as_array())
            .into_iter()
            .flatten()
            .filter_map(|a| {
                let text = |k: &str| a.get(k).and_then(|x| x.as_str()).map(str::to_string);
                Some(Asset { name: text("name")?, url: text("browser_download_url")?, size: a.get("size").and_then(|s| s.as_u64()).unwrap_or(0) })
            })
            .collect();
        let setup = release_setup(&assets, &version, target);
        let sums_name = format!("TA-Reborn-{}-SHA256SUMS.txt", version.trim().trim_start_matches(['v', 'V']));
        let sums = assets.iter().find(|a| a.name.eq_ignore_ascii_case(&sums_name)).cloned();
        return Ok(Release { version, url: field("html_url"), title: field("name"), notes: field("body"), setup, sums, local_files: false });
    }
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| tf("unreadable update file: {error}", &[("error", &e.message())]))?;
    let field = |k: &str| table.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let version = field("version");
    if version.is_empty() {
        return Err(t("the update file names no version").into());
    }
    let asset = |key: &str| {
        let url = field(key);
        // The checked filename is the decoded final path component, not query
        // credentials or a fragment. Keep the original URL for the actual request.
        let name = url::Url::parse(&url).ok().and_then(|parsed| {
            let encoded = parsed.path_segments()?.next_back()?;
            percent_encoding::percent_decode_str(encoded).decode_utf8().ok().map(|name| name.into_owned())
        }).unwrap_or_default();
        (!url.is_empty()).then(|| Asset { name, url, size: table.get(&format!("{key}_size")).and_then(|x| x.as_integer()).unwrap_or(0).max(0) as u64 })
    };
    // An explicit per-architecture key can name a custom feed's installer. Historical
    // `setup` feeds are x64 only; ARM64 may use it only when its name explicitly matches.
    let setup = match target {
        UpdateTarget::WindowsX64 => asset("setup_windows_x64").or_else(|| asset("setup")
            .filter(|a| !a.name.to_ascii_lowercase().ends_with("-windows-arm64-setup.exe"))),
        UpdateTarget::WindowsArm64 => asset("setup_windows_arm64").or_else(|| asset("setup")
            .and_then(|a| release_setup(&[a], &version, target))),
        UpdateTarget::Manual => None,
    };
    Ok(Release { version: version.clone(), url: field("url"), title: field("title"), notes: field("notes"), setup, sums: asset("sums"), local_files: false })
}

/// "v0.4.0" → [0, 4, 0], and whether it is a pre-release ("0.4.0-beta"). A mod's version may
/// lead with a word and end with more ("Gold 10.2.0", "5.1 Beta 1", "Alpha 0.8v"): the number is
/// what is read (10.2.0, not 0.2.0 with the word taken for its first part).
fn numbers(v: &str) -> ([u64; 3], bool) {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let v = v.find(|c: char| c.is_ascii_digit()).map_or(v, |at| &v[at..]);
    let v = v.split_whitespace().next().unwrap_or("");
    // Build metadata may contain hyphens without making a stable release a prerelease.
    let v = v.split_once('+').map_or(v, |(version, _)| version);
    let (core, pre) = match v.split_once('-') {
        Some((c, rest)) => (c, !rest.is_empty()),
        None => (v, false),
    };
    let mut out = [0; 3];
    for (slot, part) in out.iter_mut().zip(core.split('.')) {
        let digits: String = part.trim().chars().take_while(char::is_ascii_digit).collect();
        *slot = digits.parse().unwrap_or(0);
    }
    (out, pre)
}

/// SemVer precedence, including the catalogue's historical spellings ("5.1 Beta 1",
/// "Alpha 0.8v", "Gold 10.2.0"). Unknown suffixes keep the numeric fallback below.
fn comparable(v: &str) -> Option<semver::Version> {
    fn qualifier(word: &str) -> Option<&'static str> {
        match word.to_ascii_lowercase().as_str() { "alpha" => Some("alpha"), "beta" => Some("beta"), "rc" => Some("rc"), _ => None }
    }
    let v = v.trim().trim_start_matches(['v', 'V']);
    let at = v.find(|c: char| c.is_ascii_digit())?;
    let leading = qualifier(v[..at].trim());
    let mut words = v[at..].split_whitespace();
    let number = words.next()?;
    let pre = if let Some(word) = words.next() {
        let tag = qualifier(word)?;
        let pre = match words.next() {
            Some(revision) => format!("{tag}.{}", revision.parse::<u64>().ok()?),
            None => tag.to_string(),
        };
        if words.next().is_some() { return None; }
        pre
    } else { leading.unwrap_or("").to_string() };
    let number = if leading.is_some() { number.strip_suffix('v').unwrap_or(number) } else { number };
    let split = number.find(['-', '+']).unwrap_or(number.len());
    let (core, suffix) = number.split_at(split);
    let mut parts = core.split('.');
    let mut core = [0u64; 3];
    for slot in &mut core {
        if let Some(part) = parts.next() { *slot = part.parse().ok()?; }
    }
    if parts.next().is_some() { return None; }
    let mut version = semver::Version::parse(&format!("{}.{}.{}{suffix}", core[0], core[1], core[2])).ok()?;
    if !pre.is_empty() {
        if !version.pre.is_empty() { return None; }
        version.pre = semver::Prerelease::new(&pre).ok()?;
    }
    Some(version)
}

/// Whether `remote` is a later version than `local` (a pre-release counts as earlier than
/// its release). Build metadata never changes precedence.
pub fn newer(remote: &str, local: &str) -> bool {
    if let (Some(remote), Some(local)) = (comparable(remote), comparable(local)) {
        return remote.cmp_precedence(&local).is_gt();
    }
    let (r, r_pre) = numbers(remote);
    let (l, l_pre) = numbers(local);
    r > l || r == l && l_pre && !r_pre
}

pub fn display_version(v: &str) -> String {
    format!("v{}", v.trim().trim_start_matches(['v', 'V']))
}

/// How a release's setup is coming along.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Downloading { got: u64, total: u64, since: Instant },
    /// Being checked against the release's SHA-256.
    Checking,
    /// Checked, and ready to run.
    Ready(PathBuf),
    Failed(String),
}

/// A release's setup on its way, on a thread of its own: downloaded into the cache, checked
/// against the release's list of SHA-256 sums, then ready to run. Dropping it stops unfinished
/// work and removes its files, unless [`Download::retain_for_setup`] hands them to the installer.
pub struct Download {
    /// The version it brings ("2.2.0").
    pub version: String,
    pub step: Step,
    steps: Option<Receiver<Step>>,
    stop: Arc<AtomicBool>,
    /// The transfer under way, to stop at once.
    current: Arc<Mutex<Option<Stopper>>>,
    /// Owns only this request's generated directory, including an unpolled Ready result.
    workspace: Arc<Mutex<Option<SetupWorkspace>>>,
}

impl Download {
    /// Starts downloading `release`'s setup into `dir`. A release without a setup, or without a
    /// list of sums to check it against, is not downloaded.
    pub fn start(release: &Release, dir: &Path) -> Download {
        let (tx, rx) = mpsc::channel();
        let total = release.setup.as_ref().map_or(0, |a| a.size);
        let mut download = Download {
            version: release.number().to_string(),
            step: Step::Downloading { got: 0, total, since: Instant::now() },
            steps: Some(rx),
            stop: Arc::default(),
            current: Arc::default(),
            workspace: Arc::default(),
        };
        let (setup, sums, page, dir, local_files) = (release.setup.clone(), release.sums.clone(), release.url.clone(), dir.to_path_buf(), release.local_files);
        let (stop, current, workspace) = (download.stop.clone(), download.current.clone(), download.workspace.clone());
        let spawned = std::thread::Builder::new().name("update download".into()).spawn(move || {
            let result = match (setup, sums) {
                (Some(setup), Some(sums)) => fetch_setup(&setup, &sums, &page, &dir, &tx, &stop, &current, local_files),
                _ => Err(t("this release has no setup to check and install").into()),
            };
            let _ = tx.send(match result {
                Ok((path, lease)) => {
                    *workspace.lock().unwrap_or_else(|e| e.into_inner()) = Some(lease);
                    Step::Ready(path)
                }
                Err(e) => Step::Failed(e),
            });
        });
        if let Err(e) = spawned {
            download.step = Step::Failed(e.to_string());
            download.steps = None;
        }
        download
    }

    /// A download shown as it would be, with no thread behind it (captures, tests).
    pub fn shown(version: &str, step: Step) -> Download {
        Download { version: version.to_string(), step, steps: None, stop: Arc::default(), current: Arc::default(), workspace: Arc::default() }
    }

    /// Called only after Setup starts successfully, so it can still read its executable
    /// when the launcher closes. Cancelling Ready or failing to launch keeps cleanup armed.
    pub fn retain_for_setup(&self) {
        if let Some(workspace) = self.workspace.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            workspace.keep = true;
        }
    }

    /// Takes in the thread's news; true when something changed.
    pub fn poll(&mut self) -> bool {
        let Some(steps) = &self.steps else { return false };
        let mut changed = false;
        loop {
            match steps.try_recv() {
                Ok(step) => {
                    match &step {
                        Step::Ready(path) => log::info!("update {}: checked, {}", self.version, path.display()),
                        Step::Failed(e) => log::warn!("update {}: {e}", self.version),
                        _ => {}
                    }
                    self.step = step;
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.steps = None;
                    if matches!(self.step, Step::Downloading { .. } | Step::Checking) {
                        self.step = Step::Failed(t("the download failed").into());
                        changed = true;
                    }
                    break;
                }
            }
        }
        changed
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Ok(mut current) = self.current.lock()
            && let Some(stopper) = current.take()
        {
            stopper.stop();
        }
    }
}

/// Downloads the setup into `dir` and checks it against the release's list of sums (fetched
/// first: a release that cannot be checked is not downloaded); returns its path and cleanup
/// lease. Whatever goes wrong, nothing of it is left behind.
fn fetch_setup(setup: &Asset, sums: &Asset, page: &str, dir: &Path, tx: &Sender<Step>, stop: &AtomicBool, current: &Mutex<Option<Stopper>>, local_files: bool) -> Result<(PathBuf, SetupWorkspace), String> {
    let check_cancelled = || if stop.load(Ordering::SeqCst) { Err(t("cancelled").to_string()) } else { Ok(()) };
    check_cancelled()?;
    if !ta_vfs::package::safe_component(&setup.name) || !setup.name.to_ascii_lowercase().ends_with(".exe") {
        return Err(t("the release names an invalid setup file").into());
    }
    let policy = crate::http::Policy::Update { local_files };
    crate::http::address(&setup.url, policy)?;
    let list = fetch(&sums.url, local_files)?.ok_or_else(|| t("the release's list of SHA-256 sums is not there").to_string())?;
    check_cancelled()?;
    let expected = sum_for(&list, &setup.name).ok_or_else(|| tf("the release's list of SHA-256 sums does not name {file}", &[("file", &setup.name)]))?;
    // Keep the verified result private too, not just the partial download. Another
    // launcher or feed may offer different bytes under the same setup filename;
    // it must never replace a Ready request's checked executable before it runs.
    let workspace = SetupWorkspace::create(dir).map_err(|e| e.to_string())?;
    let path = workspace.path.join(&setup.name);
    let part = workspace.path.join("download.part");
    let result = (|| {
        // Release metadata normally supplies a size. Keep even a size-less custom
        // source bounded; this is far above a normal setup's size.
        let limit = if setup.size > 0 { setup.size } else { 2 << 30 };
        let mut transfer = Transfer::start_with_policy(&setup.url, page, &part, policy, limit)?;
        if let Ok(mut c) = current.lock() {
            *c = Some(transfer.stopper());
        }
        let since = Instant::now();
        loop {
            check_cancelled()?;
            if let Some(result) = transfer.poll() {
                result?;
                break;
            }
            let _ = tx.send(Step::Downloading { got: transfer.received(), total: setup.size, since });
            std::thread::sleep(Duration::from_millis(150));
        }
        let _ = tx.send(Step::Checking);
        let size = std::fs::metadata(&part).map_err(|e| e.to_string())?.len();
        if setup.size > 0 && size != setup.size {
            return Err(tf("the download is {got} bytes, not the release's {size}", &[("got", &size), ("size", &setup.size)]));
        }
        let actual = sha256_checked(&part, &|| stop.load(Ordering::SeqCst)).map_err(|e| {
            if e.kind() == std::io::ErrorKind::Interrupted { t("cancelled").to_string() } else { e.to_string() }
        })?;
        if actual != expected {
            return Err(t("the download does not match the release's SHA-256").into());
        }
        check_cancelled()?;
        // Publish only within this request's private directory.
        std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
        Ok(path.clone())
    })();
    if let Ok(mut c) = current.lock() {
        *c = None;
    }
    result.map(|path| (path, workspace))
}

#[derive(Debug)]
struct SetupWorkspace { path: PathBuf, keep: bool }

impl SetupWorkspace {
    fn create(parent: &Path) -> std::io::Result<Self> {
        use std::sync::atomic::AtomicU64;
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        std::fs::create_dir_all(parent)?;
        for _ in 0..100 {
            let path = parent.join(format!("setup-{}-{}", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path, keep: false }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {},
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
    }
}

impl Drop for SetupWorkspace {
    fn drop(&mut self) {
        if !self.keep { let _ = std::fs::remove_dir_all(&self.path); }
    }
}

/// The SHA-256 a list of sums (`sha256sum`'s: "<hex>  <name>") gives for `name`, in lower case.
pub fn sum_for(list: &str, name: &str) -> Option<String> {
    list.lines().find_map(|line| {
        let (hex, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim().trim_start_matches('*');
        (file == name && hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| hex.to_ascii_lowercase())
    })
}

/// A file's SHA-256, in lower-case hex (as `sha256sum` lists it).
#[cfg(test)]
fn sha256_of(path: &Path) -> std::io::Result<String> {
    sha256_checked(path, &|| false)
}

fn sha256_checked(path: &Path, cancelled: &dyn Fn() -> bool) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut file = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancelled() {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        let n = std::io::Read::read(&mut file, &mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Whether this launcher was installed by Setup (its uninstaller beside it), and so can update
/// itself by running a release's setup. A copy unpacked from the zip, a build folder and other
/// systems are shown the release's page.
pub fn installed() -> bool {
    cfg!(windows) && crate::launch::app_dir().is_some_and(|dir| dir.join("Uninstall.exe").is_file())
}

/// Starts a release's setup to update the installation in `dir` (this launcher's): Windows asks
/// for the right to install, and `/UPDATE` shows only Setup's progress and starts the launcher
/// again when it is done. The launcher closes once it has started.
#[cfg(windows)]
pub fn run_setup(setup: &Path, dir: &Path) -> Result<(), String> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SW_SHOWNORMAL};
    use windows::core::{HSTRING, w};
    // (/D= last and unquoted, as NSIS takes it: this very installation is updated.)
    let parameters = HSTRING::from(format!("/UPDATE /D={}", dir.display()));
    let folder = HSTRING::from(setup.parent().unwrap_or(Path::new(".")));
    let code = unsafe { ShellExecuteW(Some(GetForegroundWindow()), w!("open"), &HSTRING::from(setup), &parameters, &folder, SW_SHOWNORMAL) }.0 as isize;
    match code {
        c if c > 32 => Ok(()),
        // (SE_ERR_ACCESSDENIED: Windows' question was answered no.)
        5 => Err(t("Windows' permission to install was not given").into()),
        c => Err(tf("Windows could not start the setup (error {code})", &[("code", &c)])),
    }
}

#[cfg(not(windows))]
pub fn run_setup(_setup: &Path, _dir: &Path) -> Result<(), String> {
    Err(t("Updates install themselves on Windows only: get this one from its page").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Existing download fixtures model the historical x64 feed on every test host.
    fn parse(text: &str) -> Result<Release, String> { super::parse_for(text, UpdateTarget::WindowsX64) }

    #[test]
    fn multiarch_releases_select_exact_native_setup_independent_of_asset_order() {
        let mut assets: Vec<_> = ["windows-arm64-setup.exe", "setup.exe", "windows-x64-setup.exe", "macos-arm64.dmg"]
            .into_iter().map(|suffix| Asset { name: format!("TA-Reborn-2.2.0-{suffix}"), url: "https://example.invalid/asset".into(), size: 1 }).collect();
        for _ in 0..assets.len() {
            assert_eq!(release_setup(&assets, "v2.2.0", UpdateTarget::WindowsX64).unwrap().name, "TA-Reborn-2.2.0-windows-x64-setup.exe");
            assert_eq!(release_setup(&assets, "v2.2.0", UpdateTarget::WindowsArm64).unwrap().name, "TA-Reborn-2.2.0-windows-arm64-setup.exe");
            assert!(release_setup(&assets, "v2.2.0", UpdateTarget::Manual).is_none());
            assets.rotate_left(1);
        }
        assets.retain(|a| !a.name.contains("windows-x64"));
        assert_eq!(release_setup(&assets, "v2.2.0", UpdateTarget::WindowsX64).unwrap().name, "TA-Reborn-2.2.0-setup.exe");
        assets.retain(|a| !a.name.contains("windows-arm64"));
        assert!(release_setup(&assets, "v2.2.0", UpdateTarget::WindowsArm64).is_none(), "ARM must never install the legacy x64 alias");
        assert!(release_setup(&assets, "v2.3.0", UpdateTarget::WindowsX64).is_none(), "A different release version is not interchangeable");
    }

    #[test]
    fn custom_feeds_and_non_windows_updates_respect_target_architecture() {
        let old = "version = '2.2.0'\nurl = 'https://example.org/releases/2.2.0'\nsetup = 'https://example.org/TA-Reborn-2.2.0-setup.exe'\nsums = 'https://example.org/sums.txt'";
        assert!(parse_for(old, UpdateTarget::WindowsX64).unwrap().setup.is_some());
        assert!(parse_for(old, UpdateTarget::WindowsArm64).unwrap().setup.is_none());
        let manual = parse_for(old, UpdateTarget::Manual).unwrap();
        assert!(manual.setup.is_none());
        assert_eq!(manual.url, "https://example.org/releases/2.2.0");
        let named = old.replace("2.2.0-setup.exe", "2.2.0-windows-arm64-setup.exe");
        assert!(parse_for(&named, UpdateTarget::WindowsX64).unwrap().setup.is_none());
        assert!(parse_for(&named, UpdateTarget::WindowsArm64).unwrap().setup.is_some());
        let explicit = format!("{old}\nsetup_windows_arm64 = 'https://example.org/custom-arm.exe'");
        assert_eq!(parse_for(&explicit, UpdateTarget::WindowsArm64).unwrap().setup.unwrap().name, "custom-arm.exe");
        assert!(parse_for(&explicit, UpdateTarget::Manual).unwrap().setup.is_none());
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ta_launcher_update_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A file's address as curl takes it (`file:///C:/...`).
    fn file_url(path: &Path) -> String {
        format!("file:///{}", path.display().to_string().replace('\\', "/").trim_start_matches('/'))
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("v0.4.0", "0.3.0"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(!newer("0.3.0", "0.3.0"));
        assert!(!newer("0.2.9", "0.3.0"));
        assert!(!newer("0.4.0-beta", "0.4.0"));
        assert!(newer("0.4.0", "0.4.0-beta"));
        assert!(newer("1.0", "0.9.0"));
    }

    #[test]
    fn build_metadata_with_a_hyphen_is_not_a_prerelease() {
        assert!(!newer("1.0.0", "1.0.0+build-local"));
        assert!(!newer("1.0.0+build-local", "1.0.0"));
        assert!(newer("1.0.0+build-local", "1.0.0-rc.1+build-local"));
    }

    #[test]
    fn prerelease_updates_compare_numeric_identifiers_and_stable_precedence() {
        assert!(newer("1.0.0-rc.10", "1.0.0-rc.2"));
        assert!(!newer("1.0.0-rc.2", "1.0.0-rc.10"));
        assert!(newer("1.0.0-rc.1", "1.0.0-beta.10"));
        assert!(newer("5.1", "5.1 Beta 1"));
        assert!(!newer("5.1 Beta 1", "5.1"));
        assert!(newer("Alpha 0.8v", "Alpha 0.7v"));
        assert!(newer("0.8", "Alpha 0.8v"));
        assert!(!newer("1.0.0+new-build", "1.0.0+old-build"));
    }

    #[test]
    fn custom_update_asset_names_ignore_queries_and_decode_url_paths() {
        let setup = "https://example.invalid/releases/TA-Reborn%20Setup%2B9.exe?token=abc#download";
        let feed = format!("version = '9.1.0'\nsetup = '{setup}'\nsums = 'https://example.invalid/SHA256SUMS.txt?token=abc'\n");
        let release = parse(&feed).unwrap();
        let asset = release.setup.unwrap();
        assert_eq!(asset.name, "TA-Reborn Setup+9.exe");
        assert_eq!(asset.url, setup, "signed addresses keep their query intact");
        assert_eq!(release.sums.unwrap().name, "SHA256SUMS.txt");
        let arm = "version = '9.1.0'\nsetup = 'https://example.invalid/TA-Reborn-9.1.0-windows-arm64-setup.exe?token=abc'\n";
        assert!(parse_for(arm, UpdateTarget::WindowsX64).unwrap().setup.is_none(), "a query must not hide the wrong architecture");
        assert!(parse_for(arm, UpdateTarget::WindowsArm64).unwrap().setup.is_some());
    }

    /// Mods' versions may lead with a word (TA: Escalation's "Gold 10.2.0", "Alpha 0.8v"): the
    /// number after it is what is compared, not a 0 for the word.
    #[test]
    fn mods_versions_compare_by_the_number_after_a_word() {
        assert!(newer("Gold 10.2.0", "Gold 9.4.0"), "the major number is not lost to the word");
        assert!(!newer("Gold 9.4.0", "Gold 10.2.0"));
        assert!(!newer("Gold 10.2.0", "Gold 10.2.0"));
        assert!(newer("Alpha 0.9", "Alpha 0.8v"), "a letter after a number");
        assert!(!newer("5.1 Beta 1", "5.1"), "a beta precedes the stable release");
    }

    #[test]
    fn toml_and_github_answers_are_read() {
        let r = parse("version = \"0.4.0\"\nurl = \"https://example.org/r\"\ntitle = \"Conquest\"\n").unwrap();
        assert_eq!(r, Release { version: "0.4.0".into(), url: "https://example.org/r".into(), title: "Conquest".into(), ..Default::default() });
        let gh = r#"{"url":"https://api.github.com/x","html_url":"https://github.com/o/r/releases/tag/v0.5.0","author":{"html_url":"https://github.com/o"},"tag_name":"v0.5.0","name":"Big one",
            "body":"A big one.\nTwo lines.\n\n### Highlights\n- More.",
            "assets":[{"name":"TA-Reborn-0.5.0-SHA256SUMS.txt","size":192,"browser_download_url":"https://github.com/o/r/releases/download/v0.5.0/TA-Reborn-0.5.0-SHA256SUMS.txt"},
                      {"name":"TA-Reborn-0.5.0-setup.exe","size":47033289,"browser_download_url":"https://github.com/o/r/releases/download/v0.5.0/TA-Reborn-0.5.0-setup.exe"},
                      {"name":"TA-Reborn-0.5.0-windows-x64.zip","size":64252776,"browser_download_url":"https://github.com/o/r/releases/download/v0.5.0/TA-Reborn-0.5.0-windows-x64.zip"}]}"#;
        let r = parse(gh).unwrap();
        assert_eq!((r.version.as_str(), r.number(), r.url.as_str(), r.title.as_str()), ("v0.5.0", "0.5.0", "https://github.com/o/r/releases/tag/v0.5.0", "Big one"));
        assert_eq!(r.summary(), "A big one. Two lines.");
        assert_eq!(r.setup.as_ref().map(|a| (a.name.as_str(), a.size)), Some(("TA-Reborn-0.5.0-setup.exe", 47033289)));
        assert_eq!(r.sums.as_ref().map(|a| a.name.as_str()), Some("TA-Reborn-0.5.0-SHA256SUMS.txt"));
        assert!(parse(r#"{"message":"Not Found"}"#).is_err());
        assert!(parse("nonsense = ").is_err());
        // A release with no files is still news.
        assert_eq!(parse(r#"{"tag_name":"v0.6.0"}"#).map(|r| r.setup), Ok(None));
        assert!(not_found("The requested URL returned error: 404") && !not_found("Could not resolve host: api.github.com"));
        assert_eq!(crate::http::address(" file:///C:/Claude Projects/r.toml ", crate::http::Policy::Update { local_files: true }).unwrap(), "file:///C:/Claude%20Projects/r.toml");
    }

    #[test]
    fn sums_are_read_and_files_hashed() {
        let list = "221a0c3e5260ef017df85162894ead04fa4a445ec73cfca55e0e8aa2708dda99  TA-Reborn-2.1.0-setup.exe\n70FC0285357E785EE13E35F2DA56B0CA414DA7906F75DCAC197BE0BF7B472D06 *TA-Reborn-2.1.0-windows-x64.zip\n";
        assert_eq!(sum_for(list, "TA-Reborn-2.1.0-setup.exe").as_deref(), Some("221a0c3e5260ef017df85162894ead04fa4a445ec73cfca55e0e8aa2708dda99"));
        assert_eq!(sum_for(list, "TA-Reborn-2.1.0-windows-x64.zip").as_deref(), Some("70fc0285357e785ee13e35f2da56b0ca414da7906f75dcac197be0bf7b472d06"));
        assert_eq!(sum_for(list, "TA-Reborn-2.1.0-symbols.zip"), None);
        let dir = temp_dir("hash");
        std::fs::write(dir.join("abc"), "abc").unwrap();
        assert_eq!(sha256_of(&dir.join("abc")).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stopped_download_workers_do_not_leave_a_permanent_busy_state() {
        let (tx, rx) = mpsc::channel();
        let mut download = Download::shown("9.9.9", Step::Checking);
        download.steps = Some(rx);
        drop(tx);
        assert!(download.poll());
        assert!(matches!(download.step, Step::Failed(_)));
        assert!(!download.poll());
    }

    #[test]
    fn checksum_verification_honours_cancellation_between_chunks() {
        let dir = temp_dir("cancel_checksum");
        let path = dir.join("setup.exe");
        std::fs::write(&path, vec![7; 3 << 20]).unwrap();
        let reads = std::cell::Cell::new(0);
        let result = sha256_checked(&path, &|| {
            reads.set(reads.get() + 1);
            reads.get() > 2
        });
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
        assert_eq!(reads.get(), 3, "the rest of the file is not read after cancellation");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cancelled_setup_requests_do_not_touch_another_downloads_files() {
        let dir = temp_dir("cancel_before_fetch");
        let cache = dir.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let setup = Asset { name: "setup.exe".into(), url: "https://example.invalid/setup.exe".into(), size: 0 };
        let sums = Asset { name: "sums.txt".into(), url: "https://example.invalid/sums.txt".into(), size: 0 };
        std::fs::write(cache.join("setup.exe.part"), b"another transfer").unwrap();
        std::fs::write(cache.join("setup.exe"), b"previously checked").unwrap();
        let (tx, _) = mpsc::channel();
        let result = fetch_setup(&setup, &sums, "", &cache, &tx, &AtomicBool::new(true), &Mutex::default(), false);
        assert_eq!(result.unwrap_err(), "cancelled");
        assert_eq!(std::fs::read(cache.join("setup.exe.part")).unwrap(), b"another transfer");
        assert_eq!(std::fs::read(cache.join("setup.exe")).unwrap(), b"previously checked");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn remote_release_metadata_cannot_enable_local_asset_reads() {
        let release = parse("version = \"9.9.9\"\nlocal_files = true\nsetup = \"file:///C:/setup.exe\"\nsums = \"file:///C:/sums.txt\"").unwrap();
        assert!(!release.local_files);
        assert!(fetch("--version", false).is_err());
        assert!(fetch("file:///C:/sums.txt", false).is_err());
        assert!(fetch("http://example.org/sums.txt", false).is_err());
        let dir = temp_dir("untrusted_release");
        let mut download = Download::start(&release, &dir.join("cache"));
        assert!(matches!(wait(&mut download), Step::Failed(_)));
        assert!(!dir.join("cache").exists(), "invalid assets are rejected before touching disk");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn offline_updates_require_both_assets_to_be_local() {
        let local = url::Url::from_file_path(std::env::temp_dir().join("update.exe")).unwrap().to_string();
        let asset = Asset { name: "setup.exe".into(), url: local, size: 0 };
        let mut release = Release { local_files: true, setup: Some(asset.clone()), sums: Some(asset), ..Default::default() };
        assert!(release.available_offline());
        release.setup.as_mut().unwrap().url = "https://example.invalid/setup.exe".into();
        assert!(!release.available_offline());
        std::mem::swap(&mut release.setup, &mut release.sums);
        assert!(!release.available_offline(), "remote checksums also use the network");
        release.sums = None;
        assert!(!release.available_offline());
    }

    #[test]
    fn unsafe_setup_names_are_rejected_before_fetching_or_writing() {
        let dir = temp_dir("unsafe_setup_name");
        for name in ["../setup.exe", "C:\\setup.exe", "setup.exe:payload.exe", "CON.exe", "setup.exe.", "setup.bat"] {
            let release = Release {
                setup: Some(Asset { name: name.into(), url: "https://example.invalid/setup.exe".into(), size: 0 }),
                sums: Some(Asset { name: "sums.txt".into(), url: "https://example.invalid/sums.txt".into(), size: 0 }),
                ..Default::default()
            };
            let mut download = Download::start(&release, &dir.join("cache"));
            assert!(matches!(wait(&mut download), Step::Failed(error) if error.contains("invalid setup file")), "{name}");
            assert!(!dir.join("cache").exists());
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn oversized_update_metadata_is_rejected() {
        let dir = temp_dir("oversized_metadata");
        let source = dir.join("release.toml");
        std::fs::write(&source, vec![b'x'; crate::http::METADATA_LIMIT + 1]).unwrap();
        assert!(fetch(&file_url(&source), true).unwrap_err().contains("too large"));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn wait(download: &mut Download) -> Step {
        let until = Instant::now() + Duration::from_secs(30);
        while Instant::now() < until {
            download.poll();
            if matches!(download.step, Step::Ready(_) | Step::Failed(_)) {
                return download.step.clone();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the download never finished: {:?}", download.step);
    }

    /// A release's setup is downloaded (here from files, as a TOML source may name them),
    /// checked against its list of sums, and kept only when it matches.
    #[test]
    fn a_setup_is_downloaded_and_checked() {
        let dir = temp_dir("download");
        let source = dir.join("release");
        std::fs::create_dir_all(&source).unwrap();
        let setup = source.join("TA-Reborn-9.9.9-setup.exe");
        std::fs::write(&setup, vec![7u8; 300_000]).unwrap();
        let sum = sha256_of(&setup).unwrap();
        let sums = source.join("TA-Reborn-9.9.9-SHA256SUMS.txt");
        std::fs::write(&sums, format!("{sum}  TA-Reborn-9.9.9-setup.exe\n")).unwrap();
        let toml = format!("version = \"9.9.9\"\nurl = \"https://example.org\"\nsetup = \"{}\"\nsetup_size = 300000\nsums = \"{}\"\n", file_url(&setup), file_url(&sums));
        let release = Release { local_files: true, ..parse(&toml).unwrap() };
        let cache = dir.join("cache");
        let mut download = Download::start(&release, &cache);
        assert_eq!(download.version, "9.9.9");
        let Step::Ready(path) = wait(&mut download) else { panic!("{:?}", download.step) };
        assert!(path.starts_with(&cache));
        assert_eq!(path.file_name().unwrap(), "TA-Reborn-9.9.9-setup.exe");
        assert_eq!(std::fs::read(&path).unwrap().len(), 300_000);
        wait_for_download_worker(&download);
        drop(download);
        assert!(!path.exists(), "a Ready update that never starts Setup is cleaned up");
        // A file that is not the one listed is not kept.
        std::fs::write(&sums, format!("{}  TA-Reborn-9.9.9-setup.exe\n", "0".repeat(64))).unwrap();
        let mut download = Download::start(&release, &cache);
        assert!(matches!(wait(&mut download), Step::Failed(_)));
        assert_eq!(std::fs::read_dir(&cache).map(|d| d.count()).unwrap_or(0), 0, "nothing left in the cache");
        // Nor is one with no list to check it against.
        let bare = Release { sums: None, ..release };
        let mut download = Download::start(&bare, &cache);
        assert!(matches!(wait(&mut download), Step::Failed(_)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn encoded_setup_filenames_are_downloaded_and_verified_using_the_decoded_name() {
        let dir = temp_dir("encoded_setup");
        let source = dir.join("release");
        std::fs::create_dir_all(&source).unwrap();
        let name = "TA-Reborn Setup+9.exe";
        let setup = source.join(name);
        let bytes = vec![7u8; 4096];
        std::fs::write(&setup, &bytes).unwrap();
        let sums = source.join("SHA256 sums.txt");
        std::fs::write(&sums, format!("{}  {name}\n", sha256_of(&setup).unwrap())).unwrap();
        let feed = format!("version = '9.9.9'\nsetup = '{}'\nsetup_size = 4096\nsums = '{}'\n", file_url(&setup), file_url(&sums));
        let release = Release { local_files: true, ..parse(&feed).unwrap() };
        let mut download = Download::start(&release, &dir.join("cache"));
        let Step::Ready(path) = wait(&mut download) else { panic!("{:?}", download.step) };
        assert_eq!(path.file_name().unwrap(), name);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        wait_for_download_worker(&download);
        drop(download);
        assert!(!path.exists());
        assert!(setup.exists(), "the original setup remains untouched");
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn local_setup_release(source: &Path, byte: u8) -> Release {
        std::fs::create_dir_all(source).unwrap();
        let setup = source.join("setup.exe");
        let sums = source.join("sums.txt");
        std::fs::write(&setup, vec![byte; 4096]).unwrap();
        std::fs::write(&sums, format!("{}  setup.exe\n", sha256_of(&setup).unwrap())).unwrap();
        Release { version: "9.9.9".into(), local_files: true,
            setup: Some(Asset { name: "setup.exe".into(), url: file_url(&setup), size: 4096 }),
            sums: Some(Asset { name: "sums.txt".into(), url: file_url(&sums), size: 0 }), ..Default::default() }
    }

    fn wait_for_download_worker(download: &Download) {
        let until = Instant::now() + Duration::from_secs(30);
        while Arc::strong_count(&download.workspace) > 1 {
            assert!(Instant::now() < until, "the download worker never finished");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn ready_setups_keep_their_verified_bytes_when_another_feed_reuses_the_name() {
        let dir = temp_dir("same_setup_name");
        let cache = dir.join("cache");
        let mut ready = Vec::new();
        for byte in [7u8, 9u8] {
            let release = local_setup_release(&dir.join(format!("release-{byte}")), byte);
            let mut download = Download::start(&release, &cache);
            let Step::Ready(path) = wait(&mut download) else { panic!("{:?}", download.step) };
            wait_for_download_worker(&download);
            ready.push((path, byte, download));
        }
        assert_ne!(ready[0].0, ready[1].0);
        for (path, byte, _) in &ready { assert_eq!(std::fs::read(path).unwrap(), vec![*byte; 4096]); }
        let (cancelled, _, download) = ready.remove(0);
        drop(download);
        assert!(!cancelled.parent().unwrap().exists());
        assert!(ready[0].0.is_file(), "cancelling one request leaves the other's checked setup intact");
        let (handed_off, _, download) = ready.pop().unwrap();
        download.retain_for_setup();
        drop(download);
        assert!(handed_off.is_file(), "Setup can still read its executable after the launcher exits");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_completed_download_dropped_before_polling_cleans_up_its_workspace() {
        let dir = temp_dir("unpolled_setup");
        let cache = dir.join("cache");
        let release = local_setup_release(&dir.join("release"), 7);
        let download = Download::start(&release, &cache);
        wait_for_download_worker(&download);
        assert!(matches!(download.step, Step::Downloading { .. }), "no UI poll consumed the result");
        let workspace = download.workspace.lock().unwrap().as_ref().unwrap().path.clone();
        assert!(workspace.join("setup.exe").is_file());
        drop(download);
        assert!(!workspace.exists(), "an unread Ready result still owns its cleanup lease");
        assert!(dir.join("release/setup.exe").is_file());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn shown_downloads_never_own_arbitrary_ready_paths() {
        let dir = temp_dir("shown_setup");
        let path = dir.join("setup.exe");
        std::fs::write(&path, b"not managed by this download").unwrap();
        drop(Download::shown("9.9.9", Step::Ready(path.clone())));
        assert_eq!(std::fs::read(path).unwrap(), b"not managed by this download");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
