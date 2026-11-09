//! ModDB (moddb.com), where Total Annihilation's mods are published. The launcher reads
//! its RSS feeds (what ModDB offers programs), a download's start page (for the mirror link
//! a browser would follow) and the files players ask for, with the system's curl as the
//! update check does.
//!
//! ModDB welcomes launchers that bring its mods to players, asking that the content and
//! ModDB are linked, the creators credited, its bandwidth respected and its files not
//! re-hosted. So every mod shows its ModDB page and its creators; nothing is fetched until
//! the mods page opens (feeds are kept for a day, pictures for a month); a file is only
//! downloaded on a player's click, one at a time, straight from ModDB's mirror; and the
//! archive is deleted once the mod is installed.

use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};

pub const SITE: &str = "https://www.moddb.com";

pub fn mod_page(slug: &str) -> String {
    format!("{SITE}/mods/{slug}")
}

pub fn file_page(slug: &str, file: &str) -> String {
    format!("{SITE}/mods/{slug}/downloads/{file}")
}

pub fn feed_url(slug: &str) -> String {
    format!("https://rss.moddb.com/mods/{slug}/downloads/feed/rss.xml")
}

pub fn start_url(id: u32) -> String {
    format!("{SITE}/downloads/start/{id}")
}

/// A page or feed, as text.
pub fn fetch_text(url: &str, referer: Option<&str>) -> Result<String, String> {
    let url = crate::http::address(url, crate::http::Policy::Web)?;
    let mut command = crate::http::curl(crate::http::Policy::Web);
    command.arg("--max-time").arg("30");
    if let Some(r) = referer {
        command.arg("-e").arg(r);
    }
    command.arg("--url").arg(url);
    let out = crate::http::text_output(command)?;
    if !out.status.success() {
        return Err(first_line(&out.stderr, ta_ui::i18n::t("the request failed")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A small file (a picture) into `to`.
pub fn fetch_file(url: &str, to: &Path) -> Result<(), String> {
    let url = crate::http::address(url, crate::http::Policy::Web)?;
    let mut command = crate::http::curl(crate::http::Policy::Web);
    command.args(["--max-time", "60", "--max-filesize", "16777216", "--url"]).arg(url);
    // Before curl 8.4, --max-filesize did not limit unknown-length bodies. Bound
    // stdout ourselves before writing any cache file, even for chunked images.
    let out = crate::http::bounded_output(command, 16 << 20)?;
    if !out.status.success() {
        return Err(first_line(&out.stderr, ta_ui::i18n::t("the download failed")));
    }
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(to, out.stdout).map_err(|e| e.to_string())
}

fn first_line(bytes: &[u8], fallback: &str) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.lines().map(str::trim).find(|l| !l.is_empty()).map_or(fallback.to_string(), |l| l.trim_start_matches("curl: ").to_string())
}

/// One file of a mod's downloads feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedItem {
    pub title: String,
    /// The file's page (`…/downloads/<file>`).
    pub link: String,
    /// When it was added, as the feed has it ("Sat, 29 Jun 2024 12:01:40 +0000").
    pub date: String,
    /// Its picture, as a thumbnail.
    pub image: Option<String>,
}

impl FeedItem {
    /// The file's name in its page's address (`totalm1130`).
    pub fn file(&self) -> &str {
        self.link.trim_end_matches('/').rsplit('/').next().unwrap_or("")
    }
}

/// The files of a downloads feed, newest first.
pub fn parse_feed(xml: &str) -> Vec<FeedItem> {
    let mut items = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<item>") {
        let Some(len) = rest[start..].find("</item>") else { break };
        let item = &rest[start..start + len];
        rest = &rest[start + len..];
        let tag = |name: &str| -> Option<String> {
            let open = item.find(&format!("<{name}>"))? + name.len() + 2;
            let close = item[open..].find(&format!("</{name}>"))? + open;
            Some(unescape(item[open..close].trim().trim_start_matches("<![CDATA[").trim_end_matches("]]>")))
        };
        let image = item.find("<media:content url=\"").and_then(|i| {
            let from = i + "<media:content url=\"".len();
            item[from..].find('"').map(|len| thumbnail(&item[from..from + len]))
        });
        if let (Some(title), Some(link)) = (tag("title"), tag("link")) {
            items.push(FeedItem { title, link, date: tag("pubDate").unwrap_or_default(), image });
        }
    }
    items
}

fn unescape(s: &str) -> String {
    s.replace("&#45;", "-").replace("&quot;", "\"").replace("&#039;", "'").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// The mirror link on a download's start page (what the page's "download … " link points at).
pub fn parse_mirror(html: &str) -> Option<String> {
    let at = html.find("/downloads/mirror/")?;
    let end = html[at..].find(['"', '\''])? + at;
    let path = &html[at..end];
    Some(if path.starts_with("http") { path.to_string() } else { format!("{SITE}{path}") })
}

/// The 620-pixel thumbnail ModDB keeps of a picture (`…/images/…/x.png` →
/// `…/cache/images/…/thumb_620x2000/x.png`).
pub fn thumbnail(url: &str) -> String {
    if url.contains("/cache/images/") || !url.contains("media.moddb.com/images/") {
        return url.to_string();
    }
    match url.rsplit_once('/') {
        Some((dir, file)) => format!("{}/thumb_620x2000/{file}", dir.replacen("media.moddb.com/images/", "media.moddb.com/cache/images/", 1)),
        None => url.to_string(),
    }
}

/// A download in progress (curl writing into a file). The process is shared, so that the
/// launcher can stop it when it closes ([`Transfer::stopper`]).
pub struct Transfer {
    child: Arc<Mutex<Option<Child>>>,
    pub path: PathBuf,
    limit: u64,
    errors: Option<std::thread::JoinHandle<Vec<u8>>>,
}

/// Stops a transfer from elsewhere (the launcher closing).
#[derive(Clone, Default)]
pub struct Stopper(Arc<Mutex<Option<Child>>>);

impl Stopper {
    pub fn stop(&self) {
        if let Ok(mut child) = self.0.lock()
            && let Some(mut c) = child.take()
        {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Transfer {
    /// Starts downloading `url` into `to` (the start page as the referrer, as a browser has).
    /// Stalls of a minute end it.
    pub fn start(url: &str, referer: &str, to: &Path, limit: u64) -> Result<Transfer, String> {
        Self::start_with_policy(url, referer, to, crate::http::Policy::Web, limit)
    }

    pub(crate) fn start_with_policy(url: &str, referer: &str, to: &Path, policy: crate::http::Policy, limit: u64) -> Result<Transfer, String> {
        let url = crate::http::address(url, policy)?;
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let _ = std::fs::remove_file(to);
        let mut child = crate::http::curl(policy)
            .arg("--max-filesize")
            .arg(limit.to_string())
            .arg("--connect-timeout")
            .arg("30")
            .arg("--speed-limit")
            .arg("1024")
            .arg("--speed-time")
            .arg("60")
            .arg("-e")
            .arg(referer)
            .arg("-o")
            .arg(to)
            .arg("--url").arg(url)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("curl: {e}"))?;
        let errors = child.stderr.take().expect("piped errors");
        let errors = Some(std::thread::spawn(move || crate::http::diagnostics(errors)));
        Ok(Transfer { child: Arc::new(Mutex::new(Some(child))), path: to.to_path_buf(), limit, errors })
    }

    /// What stops it from another thread.
    pub fn stopper(&self) -> Stopper {
        Stopper(self.child.clone())
    }

    /// Bytes written so far.
    pub fn received(&self) -> u64 {
        std::fs::metadata(&self.path).map_or(0, |m| m.len())
    }

    /// None while it runs; then whether it succeeded.
    pub fn poll(&mut self) -> Option<Result<(), String>> {
        // Older system curl versions only apply --max-filesize to Content-Length.
        // Also bound chunked/unknown-length responses while they are being written.
        if self.received() > self.limit {
            self.stopper().stop();
            return Some(Err(ta_ui::i18n::t("The download exceeded its expected size.").into()));
        }
        let Ok(mut guard) = self.child.lock() else { return Some(Err(ta_ui::i18n::t("stopped").into())) };
        let Some(child) = guard.as_mut() else { return Some(Err(ta_ui::i18n::t("stopped").into())) };
        match child.try_wait() {
            Ok(None) => None,
            Ok(Some(status)) if status.success() => Some(Ok(())),
            Ok(Some(_)) => {
                let errors = self.errors.take().and_then(|reader| reader.join().ok()).unwrap_or_default();
                Some(Err(first_line(&errors, ta_ui::i18n::t("the download failed"))))
            }
            Err(e) => Some(Err(e.to_string())),
        }
    }

    /// Stops it and deletes what came.
    pub fn cancel(self) {
        self.stopper().stop();
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Drop for Transfer {
    fn drop(&mut self) {
        // Every early error/cancellation path must also reap its download process.
        self.stopper().stop();
        if let Some(reader) = self.errors.take() { let _ = reader.join(); }
    }
}

/// A file's MD5, as ModDB lists it (lower-case hex).
#[cfg(test)]
pub fn md5_of(path: &Path) -> std::io::Result<String> {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    let mut file = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = std::io::Read::read(&mut file, &mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"<?xml version="1.0" encoding="utf-8"?><rss version="2.0"><channel>
<title>Files RSS feed &#45; Total Mayhem</title>
<item><title>Total Mayhem 11.3.0</title><link>https://www.moddb.com/mods/total-mayhem/downloads/totalm1130</link>
<description><![CDATA[<p>The most developed...</p>]]></description><pubDate>Sat, 29 Jun 2024 12:01:40 +0000</pubDate>
<media:content url="https://media.moddb.com/images/downloads/1/274/273174/mayhem1130.png" type="image/png" medium="image"></media:content></item>
<item><title>Total Mayhem 10.9.2</title><link>https://www.moddb.com/mods/total-mayhem/downloads/total-mayhem-1092</link><pubDate>Sat, 07 Oct 2023 07:47:24 +0000</pubDate></item>
</channel></rss>"#;

    #[test]
    fn reads_a_downloads_feed() {
        let items = parse_feed(FEED);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Total Mayhem 11.3.0");
        assert_eq!(items[0].file(), "totalm1130");
        assert_eq!(items[0].date, "Sat, 29 Jun 2024 12:01:40 +0000");
        assert_eq!(items[0].image.as_deref(), Some("https://media.moddb.com/cache/images/downloads/1/274/273174/thumb_620x2000/mayhem1130.png"));
        assert_eq!(items[1].image, None);
    }

    #[test]
    fn finds_the_mirror_on_a_start_page() {
        let page = r#"<p>Wait for the download to begin, if it doesn't click the link to <a href="/downloads/mirror/273174/130/4a21fe38485ba29571f928d0b751de9d">download TotalM1130.zip</a> or try another <a href="/downloads/start/273174/all">mirror</a>.</p>"#;
        assert_eq!(parse_mirror(page).as_deref(), Some("https://www.moddb.com/downloads/mirror/273174/130/4a21fe38485ba29571f928d0b751de9d"));
        assert_eq!(parse_mirror("<p>nothing</p>"), None);
    }

    #[test]
    fn downloads_reject_oversized_files_before_accepting_them() {
        let dir = std::env::temp_dir().join(format!("ta_transfer_limit_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("large.bin");
        let target = dir.join("download.part");
        std::fs::write(&source, vec![7; 4096]).unwrap();
        let url = url::Url::from_file_path(&source).unwrap();
        let mut transfer = Transfer::start_with_policy(url.as_str(), "", &target, crate::http::Policy::Update { local_files: true }, 1024).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(result) = transfer.poll() {
                assert!(result.is_err(), "oversized downloads cannot succeed");
                break;
            }
            assert!(std::time::Instant::now() < deadline, "download never ended");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        transfer.cancel();
        assert!(!target.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn md5_matches_the_usual_digest() {
        let path = std::env::temp_dir().join(format!("ta_md5_{}", std::process::id()));
        std::fs::write(&path, b"The quick brown fox jumps over the lazy dog").unwrap();
        assert_eq!(md5_of(&path).unwrap(), "9e107d9d372bb6826bd81d3542a419d6");
        let _ = std::fs::remove_file(path);
    }
}
