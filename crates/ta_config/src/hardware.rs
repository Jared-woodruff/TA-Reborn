//! What the computer is, for the first start: the graphics card and the memory of its own,
//! the processor's threads, the memory, the main screen, and whether it is a handheld (a
//! Steam Deck). The first start (the launcher's or the game's, whichever comes first) sets the
//! game up from it ([`Config::set_up_for`]): the graphics level for the card, the screen's own
//! size, full screen; on a handheld also the interface's scale and the launcher maximised.
//! After that the settings are the player's ([`crate::Graphics::set_up`]).

use crate::{Config, GpuKind, Graphics, Preset, WindowMode, limits};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hardware {
    /// The graphics card (of several, the one with the most memory of its own).
    pub gpu: String,
    pub gpu_kind: Option<GpuKind>,
    /// The card's own memory in MB (0 when unknown; integrated graphics have little).
    pub vram_mb: u64,
    /// The processor's threads.
    pub threads: usize,
    /// The memory in MB (0 when unknown).
    pub ram_mb: u64,
    /// The main screen's size in pixels, as it is set now, and how often it refreshes (Hz; 0
    /// when unknown).
    pub screen: Option<(u32, u32)>,
    pub refresh_hz: u32,
    /// The processor, as Windows names it ("AMD Ryzen 9 7950X 16-Core Processor").
    pub cpu: String,
    /// The handheld it is, if one ([`Handheld::detect`]).
    pub handheld: Option<Handheld>,
    /// The system it runs, as Rust names it ("windows", "macos", "linux"; empty when not
    /// looked at).
    pub os: &'static str,
}

impl Hardware {
    /// Looks at this computer (on Windows; on Linux its memory and processor and whether it is
    /// a Steam Deck; on a Mac its memory and processor, and on Apple's own chips their graphics;
    /// elsewhere only the threads are known).
    pub fn detect() -> Hardware {
        let mut hw = Hardware { threads: std::thread::available_parallelism().map_or(0, |n| n.get()), os: std::env::consts::OS, ..Default::default() };
        #[cfg(windows)]
        win::detect(&mut hw);
        #[cfg(target_os = "linux")]
        linux::detect(&mut hw);
        #[cfg(target_os = "macos")]
        mac::detect(&mut hw);
        hw.handheld = Handheld::detect(&hw.gpu);
        hw
    }

    /// Linux: its look asks the graphics drivers nothing (the game picks the level from the card
    /// it opens), and a handheld's system draws at the screen's own pixels (SteamOS), leaving
    /// the interface's size to the game.
    fn linux(&self) -> bool {
        self.os == "linux"
    }

    /// The graphics level the first start sets ([`Hardware::preset`]); None on Linux while the
    /// card is not known: the level stays Auto, for the game to pick from the card it opens.
    /// (Elsewhere a card not found, an Intel Mac's, still gets [`Preset::for_gpu`]'s Medium.)
    pub fn first_level(&self) -> Option<Preset> {
        (self.gpu_kind.is_some() || !self.linux()).then(|| self.preset())
    }

    /// The main screen's size in pixels: as it is set now, else a handheld's own.
    pub fn screen_size(&self) -> Option<(u32, u32)> {
        self.screen.or(self.handheld.map(Handheld::screen))
    }

    /// The graphics level for this computer: the card's kind and name (see
    /// [`Preset::for_gpu`]), on a dedicated card its memory (Ultra from 10 GB, High from
    /// 6 GB, Medium from 3 GB), one level less on a 4K screen without 12 GB (but on integrated
    /// graphics, whose render scale is fitted to the screen instead), and no more than Medium
    /// with fewer than four threads or 8 GB.
    pub fn preset(&self) -> Preset {
        let kind = self.gpu_kind.unwrap_or(GpuKind::Other);
        let by_name = Preset::for_gpu(kind, &self.gpu);
        let mut level = match kind {
            // (Entry-level cards of their day stay at what their name says.)
            GpuKind::Discrete if self.vram_mb > 0 && by_name == Preset::High => match self.vram_mb {
                m if m >= 9_500 => Preset::Ultra,
                m if m >= 5_500 => Preset::High,
                m if m >= 2_800 => Preset::Medium,
                _ => Preset::Low,
            },
            _ => by_name,
        };
        // (Integrated graphics draw the battlefield at fewer pixels than such a screen's instead:
        // see `Preset::pixel_budget`.)
        let big_screen = self.screen.is_some_and(|(w, h)| w as u64 * h as u64 >= 3840 * 2160);
        if big_screen && self.vram_mb < 11_500 && kind != GpuKind::Integrated {
            level = lower(level);
        }
        if (self.threads > 0 && self.threads < 4) || (self.ram_mb > 0 && self.ram_mb < 7_500) {
            level = level.min_level(Preset::Medium);
        }
        level
    }

    /// A line for the launcher: "NVIDIA GeForce RTX 3050 (8 GB) · 8 threads · 32 GB" ("Steam
    /// Deck · 8 threads · 15 GB" on one).
    pub fn summary(&self) -> String {
        let gb = |mb: u64| format!("{:.0} GB", mb as f64 / 1024.0);
        let mut parts = Vec::new();
        if let Some(handheld) = self.handheld {
            parts.push(handheld.name().to_string());
        }
        if !self.gpu.is_empty() {
            parts.push(if self.vram_mb >= 1024 { format!("{} ({})", self.gpu, gb(self.vram_mb)) } else { self.gpu.clone() });
        }
        if self.threads > 0 {
            parts.push(format!("{} threads", self.threads));
        }
        if self.ram_mb > 0 {
            parts.push(gb(self.ram_mb));
        }
        parts.join(" · ")
    }
}

/// The game's memory in use now and at most (its working set, MB): the benchmark's.
pub fn process_memory_mb() -> Option<(u64, u64)> {
    process_memory().map(|m| (m.working_set_mb, m.peak_working_set_mb))
}

/// The game's own memory, MB.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessMemory {
    /// What the system has committed to this process alone (Windows' private bytes, Linux's
    /// anonymous resident memory): what grows when memory is kept and never given back.
    pub private_mb: u64,
    /// What of it is in physical memory now, and at most so far.
    pub working_set_mb: u64,
    pub peak_working_set_mb: u64,
}

/// [`ProcessMemory`] now (Windows and Linux; none elsewhere).
pub fn process_memory() -> Option<ProcessMemory> {
    #[cfg(windows)]
    {
        use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
        use windows::Win32::System::Threading::GetCurrentProcess;
        let mut counters = PROCESS_MEMORY_COUNTERS_EX { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
        // SAFETY: a query of this process, into a structure of our own of the size given (the
        // extended counters begin with the plain ones, as the call reads by `cb`).
        unsafe { GetProcessMemoryInfo(GetCurrentProcess(), (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>(), counters.cb) }.ok()?;
        Some(ProcessMemory {
            private_mb: counters.PrivateUsage as u64 >> 20,
            working_set_mb: counters.WorkingSetSize as u64 >> 20,
            peak_working_set_mb: counters.PeakWorkingSetSize as u64 >> 20,
        })
    }
    #[cfg(target_os = "linux")]
    {
        status_memory(&std::fs::read_to_string("/proc/self/status").ok()?)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

/// The computer's memory, MB: all of it, and how much is free for programs now (Windows and
/// Linux; none elsewhere). Cheap enough to ask every few seconds.
pub fn system_memory_mb() -> Option<(u64, u64)> {
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut memory = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        // SAFETY: a query into a structure of our own of the size it says.
        unsafe { GlobalMemoryStatusEx(&mut memory) }.ok()?;
        Some((memory.ullTotalPhys >> 20, memory.ullAvailPhys >> 20))
    }
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        Some((meminfo_mb(&text)?, meminfo_available_mb(&text)?))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

/// The memory free for programs in MB from /proc/meminfo's text (its `MemAvailable: <n> kB`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn meminfo_available_mb(text: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with("MemAvailable:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

/// [`ProcessMemory`] from /proc/self/status's text (`VmRSS`, `VmHWM`, `RssAnon`, in kB).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn status_memory(text: &str) -> Option<ProcessMemory> {
    let kb = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)?.trim().strip_suffix("kB")?.trim().parse::<u64>().ok());
    let rss = kb("VmRSS:")?;
    Some(ProcessMemory { private_mb: kb("RssAnon:").unwrap_or(rss) >> 10, working_set_mb: rss >> 10, peak_working_set_mb: kb("VmHWM:").unwrap_or(rss) >> 10 })
}

/// What Linux tells of the computer (from /proc): its memory and its processor.
#[cfg(target_os = "linux")]
mod linux {
    pub fn detect(hw: &mut super::Hardware) {
        if let Ok(text) = std::fs::read_to_string("/proc/meminfo")
            && let Some(mb) = super::meminfo_mb(&text)
        {
            hw.ram_mb = mb;
        }
        if let Ok(text) = std::fs::read_to_string("/proc/cpuinfo")
            && let Some(name) = super::cpuinfo_name(&text)
        {
            hw.cpu = name;
        }
    }
}

/// What a Mac tells of itself (`sysctl`): its memory and processor; Apple's own chips (M1 and
/// after) carry their graphics, integrated, named as the chip is ("Apple M5", as Metal names
/// it too).
#[cfg(target_os = "macos")]
mod mac {
    pub fn detect(hw: &mut super::Hardware) {
        let ask = |key: &str| {
            let out = std::process::Command::new("/usr/sbin/sysctl").args(["-n", key]).output().ok()?;
            let text = String::from_utf8(out.stdout).ok()?;
            let text = text.trim();
            (out.status.success() && !text.is_empty()).then(|| text.to_string())
        };
        if let Some(bytes) = ask("hw.memsize").and_then(|v| v.parse::<u64>().ok()) {
            hw.ram_mb = bytes >> 20;
        }
        if let Some(name) = ask("machdep.cpu.brand_string") {
            if let Some(gpu) = super::apple_chip_gpu(&name) {
                hw.gpu = gpu;
                hw.gpu_kind = Some(crate::GpuKind::Integrated);
            }
            hw.cpu = name;
        }
    }
}

/// The graphics of one of Apple's own chips, from the processor's name ("Apple M5 Pro"): the
/// chip's name, as Metal gives it; None for other processors (an Intel Mac's graphics are not
/// its processor's).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn apple_chip_gpu(cpu: &str) -> Option<String> {
    let name = cpu.split_whitespace().collect::<Vec<_>>().join(" ");
    name.starts_with("Apple M").then_some(name)
}

/// The memory in MB from /proc/meminfo's text (its `MemTotal: <n> kB`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn meminfo_mb(text: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

/// The processor's name from /proc/cpuinfo's text (its first `model name`), its spaces single.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn cpuinfo_name(text: &str) -> Option<String> {
    let line = text.lines().find(|l| l.starts_with("model name"))?;
    let name = line.split_once(':')?.1.split_whitespace().collect::<Vec<_>>().join(" ");
    (!name.is_empty()).then_some(name)
}

/// A handheld gaming computer: the first start sets the game up for its small screen, held
/// close ([`Config::set_up_for`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handheld {
    /// Valve's Steam Deck: the LCD model ("Jupiter") or the OLED one ("Galileo").
    SteamDeck,
}

/// Where Linux shows the firmware's maker and model.
const DMI_VENDOR: &str = "/sys/devices/virtual/dmi/id/sys_vendor";
const DMI_PRODUCT: &str = "/sys/devices/virtual/dmi/id/product_name";
/// The system's name and version (the second where the first is missing).
const OS_RELEASE: [&str; 2] = ["/etc/os-release", "/usr/lib/os-release"];

impl Handheld {
    pub fn name(self) -> &'static str {
        match self {
            Handheld::SteamDeck => "Steam Deck",
        }
    }

    /// Its screen's own size in pixels, held level (both Steam Decks': 1280 × 800).
    pub fn screen(self) -> (u32, u32) {
        match self {
            Handheld::SteamDeck => (1280, 800),
        }
    }

    /// The handheld this computer is, if one: from its graphics' name (`gpu`, "" when not known;
    /// Windows on a Deck names them), and on Linux from what the system says of itself
    /// ([`Handheld::detect_with`]). Elsewhere nothing is read.
    pub fn detect(gpu: &str) -> Option<Handheld> {
        let linux = cfg!(target_os = "linux");
        let env = |name: &str| if linux { std::env::var(name).ok() } else { None };
        let read = |path: &str| if linux { std::fs::read_to_string(path).ok() } else { None };
        Handheld::detect_with(env, read, gpu)
    }

    /// A Steam Deck is known by any one of: Steam's `SteamDeck=1` among the variables of what it
    /// starts; the firmware's maker and model (Valve's "Jupiter" or "Galileo"); SteamOS's
    /// os-release (`ID=steamos`); its graphics' name ([`steam_deck_gpu`]). `env` gives a variable,
    /// `read` a file's text (None where there is none).
    pub fn detect_with(env: impl Fn(&str) -> Option<String>, read: impl Fn(&str) -> Option<String>, gpu: &str) -> Option<Handheld> {
        let by_steam = || env("SteamDeck").is_some_and(|v| v.trim() == "1");
        let by_firmware = || {
            read(DMI_VENDOR).is_some_and(|v| v.trim().eq_ignore_ascii_case("valve"))
                && read(DMI_PRODUCT).is_some_and(|p| ["jupiter", "galileo"].iter().any(|model| p.trim().eq_ignore_ascii_case(model)))
        };
        let by_system = || OS_RELEASE.iter().find_map(|path| read(path)).is_some_and(|text| os_release_id(&text).is_some_and(|id| id.eq_ignore_ascii_case("steamos")));
        (steam_deck_gpu(gpu) || by_steam() || by_firmware() || by_system()).then_some(Handheld::SteamDeck)
    }
}

/// Whether a graphics adapter's name is the Steam Deck's (AMD's Van Gogh): "AMD Custom GPU 0405"
/// on the LCD model and "AMD Custom GPU 0932" on the OLED one, as Windows and Mesa name them
/// ("AMD Custom GPU 0405 (RADV VANGOGH)" on Vulkan), or Mesa's "vangogh" alone.
pub fn steam_deck_gpu(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["custom gpu 0405", "custom gpu 0932", "vangogh"].iter().any(|key| name.contains(key))
}

/// The `ID` an os-release file's text gives ("steamos"), without its quotes.
fn os_release_id(text: &str) -> Option<&str> {
    text.lines().find_map(|line| line.trim().strip_prefix("ID=")).map(|id| id.trim().trim_matches(['"', '\'']))
}

/// Whether this runs inside gamescope, the compositor of SteamOS's Game Mode (and of other
/// gaming sessions): it shows one program at a time, scaled to fill the screen, so a window
/// should fill the screen itself to be drawn pixel for pixel. Read on Linux alone
/// ([`gamescope_with`]); elsewhere never.
pub fn gamescope() -> bool {
    cfg!(target_os = "linux") && gamescope_with(|name| std::env::var(name).ok())
}

/// [`gamescope`] from what `env` gives of the variables gamescope sets for what runs in it
/// (`GAMESCOPE_WAYLAND_DISPLAY`) and its session's desktop (`XDG_CURRENT_DESKTOP=gamescope`).
pub fn gamescope_with(env: impl Fn(&str) -> Option<String>) -> bool {
    env("GAMESCOPE_WAYLAND_DISPLAY").is_some_and(|display| !display.trim().is_empty())
        || env("XDG_CURRENT_DESKTOP").is_some_and(|desktop| desktop.split(':').any(|d| d.eq_ignore_ascii_case("gamescope")))
}

/// The language of the computer's own interface, as a tag ("de-DE", "zh-CN", "pt-BR"): the
/// first start shows the game in it when there is a translation. Windows' display
/// language; elsewhere the locale's variables.
pub fn system_language() -> Option<String> {
    #[cfg(windows)]
    {
        use windows::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};
        // SAFETY: plain queries, into a buffer of our own.
        unsafe {
            let mut name = [0u16; 85];
            let n = LCIDToLocaleName(GetUserDefaultUILanguage() as u32, Some(&mut name), 0);
            (n > 1).then(|| String::from_utf16_lossy(&name[..n as usize - 1]))
        }
    }
    #[cfg(not(windows))]
    {
        ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty() && s != "C" && s != "POSIX"))
    }
}

/// One level less (Low stays Low).
fn lower(level: Preset) -> Preset {
    match level {
        Preset::Ultra => Preset::High,
        Preset::High => Preset::Medium,
        Preset::Medium | Preset::Low => Preset::Low,
        other => other,
    }
}

impl Preset {
    /// This level, or `cap` when that is lower.
    fn min_level(self, cap: Preset) -> Preset {
        if (self as u8) > (cap as u8) && Preset::LEVELS.contains(&self) { cap } else { self }
    }
}

impl Graphics {
    /// The first start's graphics ([`Config::set_up_for`]): the level for this computer
    /// ([`Graphics::pick_level_for`]), the screen's own size (a handheld's, where the screen is
    /// not known), full screen; vertical sync on a handheld.
    pub fn set_up_for(&mut self, hw: &Hardware) {
        self.pick_level_for(hw);
        if let Some((w, h)) = hw.screen_size().filter(|&(w, h)| w >= 640 && h >= 480) {
            self.width = w;
            self.height = h;
        }
        self.window_mode = WindowMode::Fullscreen;
        if hw.handheld.is_some() {
            self.vsync = true;
        }
        self.set_up = true;
    }

    /// The graphics level for this computer where the first start can tell it
    /// ([`Hardware::first_level`]), the card noted, and the render scale for the screen where
    /// both are known (the game fits it to its window too when it first starts). Else the level
    /// stays Auto, for the game to pick from the card it opens. Also for a level put back to
    /// Auto in the options, as the game next starts.
    pub fn pick_level_for(&mut self, hw: &Hardware) {
        let Some(level) = hw.first_level() else { return };
        level.apply(self);
        self.preset = level;
        if !hw.gpu.is_empty() {
            self.gpu = hw.gpu.clone();
        }
        if let (Some(kind), Some((w, h))) = (hw.gpu_kind, hw.screen) {
            self.fit_render_scale(kind, &hw.gpu, w as u64 * h as u64);
        }
    }
}

impl Config {
    /// Whether these settings are yet to be set up for the computer ([`Config::set_up_for`]): a
    /// new settings file, or one whose graphics were put back to how they came (RESET ALL
    /// SETTINGS, the graphics' RESTORE DEFAULTS).
    pub fn first_start(&self) -> bool {
        self.graphics.preset == Preset::Auto && !self.graphics.set_up
    }

    /// The first start, the launcher's or the game's, whichever comes first: the game set up for
    /// this computer, all of it here. The graphics ([`Graphics::set_up_for`]); on a handheld also
    /// the launcher maximised (in SteamOS's Game Mode it fills the screen anyway: [`gamescope`])
    /// and, under Linux (whose SteamOS draws at the screen's own pixels; Windows enlarges a small
    /// screen's interface itself), the interface at the largest scale that leaves the screen room
    /// for the battle and the launcher's pages, which share it (125% on a Steam Deck). After that
    /// the settings are the player's.
    pub fn set_up_for(&mut self, hw: &Hardware) {
        self.graphics.set_up_for(hw);
        if hw.handheld.is_none() {
            return;
        }
        self.launcher.maximized = true;
        if let Some(screen) = hw.screen_size().filter(|_| hw.linux()) {
            let fit = |points| limits::ui_scale_to_fit(screen, points);
            self.gameplay.ui_scale = fit(limits::GAME_POINTS).min(fit(limits::LAUNCHER_POINTS));
        }
    }
}

#[cfg(windows)]
mod win {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1};
    use windows::Win32::Graphics::Gdi::{DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW};
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows::core::PCWSTR;

    use super::Hardware;
    use crate::GpuKind;

    pub(super) fn detect(hw: &mut Hardware) {
        // The graphics cards (DXGI): the one with the most memory of its own.
        // SAFETY: plain DXGI queries; the factory and adapters are released when dropped.
        unsafe {
            if let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() {
                let mut best: Option<(u64, String, bool)> = None;
                let mut i = 0;
                while let Ok(adapter) = factory.EnumAdapters1(i) {
                    i += 1;
                    let Ok(desc) = adapter.GetDesc1() else { continue };
                    let software = desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0;
                    let len = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
                    let name = String::from_utf16_lossy(&desc.Description[..len]).trim().to_string();
                    let vram = desc.DedicatedVideoMemory as u64 / (1024 * 1024);
                    // (A software adapter only when there is nothing else.)
                    let better = match &best {
                        None => true,
                        Some((_, _, true)) => !software,
                        Some((v, _, false)) => !software && vram > *v,
                    };
                    if better {
                        best = Some((vram, name, software));
                    }
                }
                if let Some((vram, name, software)) = best {
                    hw.gpu_kind = Some(if software {
                        GpuKind::Software
                    } else if vram >= 1_000 {
                        GpuKind::Discrete
                    } else {
                        GpuKind::Integrated
                    });
                    hw.gpu = name;
                    hw.vram_mb = vram;
                }
            }
            let mut memory = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
            if GlobalMemoryStatusEx(&mut memory).is_ok() {
                hw.ram_mb = memory.ullTotalPhys / (1024 * 1024);
            }
            // The main screen as it is set now (its pixels, whatever the scaling).
            let mut mode = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
            if EnumDisplaySettingsW(PCWSTR::null(), ENUM_CURRENT_SETTINGS, &mut mode).as_bool() && mode.dmPelsWidth > 0 {
                hw.screen = Some((mode.dmPelsWidth, mode.dmPelsHeight));
                // (0 and 1 stand for the hardware's default rate.)
                hw.refresh_hz = if mode.dmDisplayFrequency > 1 { mode.dmDisplayFrequency } else { 0 };
            }
        }
        // The processor's name, as Windows has it.
        let key = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE).open_subkey(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0");
        if let Ok(name) = key.and_then(|k| k.get_value::<String, _>("ProcessorNameString")) {
            hw.cpu = name.split_whitespace().collect::<Vec<_>>().join(" ");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Linux's /proc tells the memory and the processor (read on any system: the text alone).
    #[test]
    fn linux_tells_its_memory_and_processor() {
        assert_eq!(meminfo_mb("MemTotal:       65839120 kB\nMemFree:          812344 kB\n"), Some(64_296));
        assert_eq!(meminfo_mb("MemFree: 1 kB\n"), None);
        let cpuinfo = "processor\t: 0\nvendor_id\t: AuthenticAMD\nmodel name\t: AMD Ryzen 9  5950X 16-Core Processor\nprocessor\t: 1\n";
        assert_eq!(cpuinfo_name(cpuinfo).as_deref(), Some("AMD Ryzen 9 5950X 16-Core Processor"));
        assert_eq!(cpuinfo_name("processor\t: 0\n"), None);
    }

    /// Linux's /proc tells how much memory is free for programs, and the game's own.
    #[test]
    fn linux_tells_the_memory_free_and_the_games_own() {
        assert_eq!(meminfo_available_mb("MemTotal: 65839120 kB\nMemFree: 812344 kB\nMemAvailable:   20971520 kB\n"), Some(20_480));
        assert_eq!(meminfo_available_mb("MemTotal: 65839120 kB\n"), None);
        let status = "Name:\tta-reborn\nVmHWM:\t 3145728 kB\nVmRSS:\t 2097152 kB\nRssAnon:\t 1572864 kB\nRssFile:\t 524288 kB\n";
        assert_eq!(status_memory(status), Some(ProcessMemory { private_mb: 1536, working_set_mb: 2048, peak_working_set_mb: 3072 }));
        // (An older kernel without the split: the resident memory for both.)
        assert_eq!(status_memory("VmRSS:\t 1048576 kB\n"), Some(ProcessMemory { private_mb: 1024, working_set_mb: 1024, peak_working_set_mb: 1024 }));
        assert_eq!(status_memory("Name:\tta-reborn\n"), None);
    }

    /// The game's own memory is known on Windows and Linux, and adds up.
    #[test]
    fn the_games_own_memory_is_known() {
        if cfg!(any(windows, target_os = "linux")) {
            let m = process_memory().expect("this process's memory");
            assert!(m.working_set_mb > 0 && m.peak_working_set_mb >= m.working_set_mb, "{m:?}");
            let (total, free) = system_memory_mb().expect("the computer's memory");
            assert!(total > 0 && free <= total, "{total} {free}");
        }
    }

    fn pc(gpu: &str, kind: GpuKind, vram_mb: u64, screen: (u32, u32)) -> Hardware {
        Hardware { gpu: gpu.into(), gpu_kind: Some(kind), vram_mb, threads: 8, ram_mb: 16_384, screen: Some(screen), ..Default::default() }
    }

    #[test]
    fn levels_follow_the_card_and_the_screen() {
        let hd = (1920, 1080);
        assert_eq!(pc("NVIDIA GeForce RTX 3050", GpuKind::Discrete, 8_000, hd).preset(), Preset::High);
        assert_eq!(pc("NVIDIA GeForce RTX 4070", GpuKind::Discrete, 12_000, hd).preset(), Preset::Ultra);
        assert_eq!(pc("NVIDIA GeForce GTX 1650", GpuKind::Discrete, 4_000, hd).preset(), Preset::Medium);
        assert_eq!(pc("NVIDIA GeForce GT 1030", GpuKind::Discrete, 2_000, hd).preset(), Preset::Medium);
        assert_eq!(pc("Intel(R) UHD Graphics 630", GpuKind::Integrated, 128, hd).preset(), Preset::Low);
        assert_eq!(pc("AMD Radeon 780M Graphics", GpuKind::Integrated, 512, hd).preset(), Preset::Medium);
        assert_eq!(pc("Microsoft Basic Render Driver", GpuKind::Software, 0, hd).preset(), Preset::Low);
        // A 4K screen asks for more.
        assert_eq!(pc("NVIDIA GeForce RTX 3050", GpuKind::Discrete, 8_000, (3840, 2160)).preset(), Preset::Medium);
        assert_eq!(pc("NVIDIA GeForce RTX 4090", GpuKind::Discrete, 24_000, (3840, 2160)).preset(), Preset::Ultra);
        // Few threads or little memory.
        let mut old = pc("NVIDIA GeForce RTX 3080", GpuKind::Discrete, 10_000, hd);
        old.threads = 2;
        assert_eq!(old.preset(), Preset::Medium);
    }

    /// Apple's chips: Medium on the base and Pro chips, High on the Max and Ultra; a Retina
    /// screen draws the battlefield at fewer pixels than its own (the interface keeps them).
    #[test]
    fn apple_chips_are_known_and_fitted_to_their_screens() {
        assert_eq!(apple_chip_gpu("Apple M5").as_deref(), Some("Apple M5"));
        assert_eq!(apple_chip_gpu("Apple  M4 Pro").as_deref(), Some("Apple M4 Pro"));
        assert_eq!(apple_chip_gpu("Intel(R) Core(TM) i9-9980HK CPU @ 2.40GHz"), None);
        let mac = |gpu: &str, screen| Hardware { gpu: gpu.into(), gpu_kind: Some(GpuKind::Integrated), threads: 10, ram_mb: 24_576, screen, ..Default::default() };
        assert_eq!(mac("Apple M5", None).preset(), Preset::Medium);
        assert_eq!(mac("Apple M4 Pro", None).preset(), Preset::Medium);
        assert_eq!(mac("Apple M4 Max", None).preset(), Preset::High);
        assert_eq!(mac("Apple M3 Ultra", None).preset(), Preset::High);
        // A 15-inch MacBook Air's screen (its default "looks like 1710 x 1112"), known: half.
        let mut g = Graphics::default();
        g.set_up_for(&mac("Apple M5", Some((3420, 2224))));
        assert_eq!((g.preset, g.render_scale), (Preset::Medium, 0.5));
        // Unknown (a Mac does not say): the level's own, until the game fits it to its window.
        let mut g = Graphics::default();
        g.set_up_for(&mac("Apple M5", None));
        assert_eq!((g.preset, g.render_scale), (Preset::Medium, 1.0));
        // A 4K screen on integrated graphics keeps the level: the render scale is fitted instead.
        let laptop = pc("AMD Radeon 780M Graphics", GpuKind::Integrated, 512, (3840, 2160));
        assert_eq!(laptop.preset(), Preset::Medium);
        let mut g = Graphics::default();
        g.set_up_for(&laptop);
        assert_eq!(g.render_scale, 0.5);
    }

    #[test]
    fn the_first_start_sets_up_full_screen() {
        let mut g = Graphics::default();
        assert_eq!(g.preset, Preset::Auto);
        g.set_up_for(&pc("NVIDIA GeForce RTX 3050", GpuKind::Discrete, 8_000, (2560, 1440)));
        assert_eq!((g.preset, g.width, g.height, g.window_mode), (Preset::High, 2560, 1440, WindowMode::Fullscreen));
        assert_eq!(g.msaa, 4);
        assert!(g.set_up);
    }

    #[test]
    fn this_computer_is_detected() {
        let hw = Hardware::detect();
        println!("{hw:?} -> {:?}", hw.preset());
        assert!(hw.threads > 0);
        assert_eq!(hw.os, std::env::consts::OS);
    }

    /// A Steam Deck under SteamOS, as its look finds it: Linux, no graphics card known before
    /// the game opens one.
    fn deck() -> Hardware {
        Hardware { os: "linux", handheld: Some(Handheld::SteamDeck), threads: 8, ram_mb: 14_830, cpu: "AMD Custom APU 0405".into(), ..Default::default() }
    }

    /// Any one of what tells a Steam Deck is enough alone; none of them, no handheld.
    #[test]
    fn a_steam_deck_is_known_by_any_one_of_its_signs() {
        use std::collections::HashMap;
        let detect = |vars: &[(&str, &str)], files: &[(&str, &str)], gpu: &str| {
            let vars: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            let files: HashMap<String, String> = files.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            Handheld::detect_with(|name| vars.get(name).cloned(), |path| files.get(path).cloned(), gpu)
        };
        let deck = Some(Handheld::SteamDeck);
        // None of them: an ordinary computer.
        assert_eq!(detect(&[], &[], ""), None);
        // Steam's variable, in what it starts on a Deck.
        assert_eq!(detect(&[("SteamDeck", "1")], &[], ""), deck);
        assert_eq!(detect(&[("SteamDeck", "0")], &[], ""), None);
        assert_eq!(detect(&[("SteamOS", "1")], &[], ""), None);
        // The firmware, as Linux shows it (a line each): Valve's Jupiter (LCD) or Galileo (OLED).
        assert_eq!(detect(&[], &[(DMI_VENDOR, "Valve\n"), (DMI_PRODUCT, "Jupiter\n")], ""), deck);
        assert_eq!(detect(&[], &[(DMI_VENDOR, "Valve\n"), (DMI_PRODUCT, "Galileo\n")], ""), deck);
        assert_eq!(detect(&[], &[(DMI_VENDOR, "Valve\n"), (DMI_PRODUCT, "Index\n")], ""), None);
        assert_eq!(detect(&[], &[(DMI_VENDOR, "LENOVO\n"), (DMI_PRODUCT, "Jupiter\n")], ""), None);
        assert_eq!(detect(&[], &[(DMI_PRODUCT, "Jupiter\n")], ""), None);
        // SteamOS, its ID with or without quotes; /usr/lib's where /etc has none.
        let steamos = "NAME=\"SteamOS\"\nPRETTY_NAME=\"SteamOS\"\nVERSION_CODENAME=holo\nID=steamos\nID_LIKE=arch\n";
        assert_eq!(detect(&[], &[("/etc/os-release", steamos)], ""), deck);
        assert_eq!(detect(&[], &[("/usr/lib/os-release", "ID=\"steamos\"\n")], ""), deck);
        assert_eq!(detect(&[], &[("/etc/os-release", "NAME=\"Arch Linux\"\nID=arch\n")], ""), None);
        assert_eq!(detect(&[], &[("/etc/os-release", "ID=bazzite\nID_LIKE=\"steamos fedora\"\n")], ""), None, "only its own ID");
        assert_eq!(detect(&[], &[("/etc/os-release", "ID=arch\n"), ("/usr/lib/os-release", "ID=steamos\n")], ""), None, "/etc's first");
        // Its graphics, as Vulkan (Mesa's RADV), Windows and OpenGL name them.
        for gpu in ["AMD Custom GPU 0405 (RADV VANGOGH)", "AMD Custom GPU 0932 (RADV VANGOGH)", "AMD Custom GPU 0405", "AMD Custom GPU 0405 (radeonsi, vangogh, LLVM 15.0.7, DRM 3.54)"] {
            assert_eq!(detect(&[], &[], gpu), deck, "{gpu}");
        }
        assert_eq!(detect(&[], &[], "AMD Radeon 780M Graphics"), None);
        assert_eq!(detect(&[], &[], "AMD Radeon RX 6800 XT (RADV NAVI21)"), None);
        // Off Linux nothing is read: only the graphics' name tells there.
        if !cfg!(target_os = "linux") {
            assert_eq!(Handheld::detect(""), None);
        }
        assert_eq!(Handheld::detect("AMD Custom GPU 0405"), deck);
        assert_eq!(Handheld::SteamDeck.screen(), (1280, 800));
    }

    /// Inside gamescope (SteamOS's Game Mode) by the variable it gives what it starts, or its
    /// session's desktop; a desktop's own session is not. Nothing is read off Linux.
    #[test]
    fn game_mode_is_known_by_gamescope() {
        let gamescope = |vars: &[(&str, &str)]| gamescope_with(|name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string()));
        assert!(gamescope(&[("GAMESCOPE_WAYLAND_DISPLAY", "gamescope-0")]));
        assert!(gamescope(&[("XDG_CURRENT_DESKTOP", "gamescope")]));
        assert!(!gamescope(&[("XDG_CURRENT_DESKTOP", "KDE")]));
        assert!(!gamescope(&[("XDG_CURRENT_DESKTOP", "ubuntu:GNOME"), ("GAMESCOPE_WAYLAND_DISPLAY", "")]));
        assert!(!gamescope(&[]));
        if !cfg!(target_os = "linux") {
            assert!(!super::gamescope());
        }
    }

    /// A Steam Deck's first start under SteamOS: full screen at its screen's size, vertical sync,
    /// the interface at 125% (the most the battle's 1024 × 600 points leave at 1280 × 800: the
    /// launcher, sharing it, keeps more than its 940 × 600), the launcher maximised. The level
    /// is left to the game, which picks Medium for the Deck's graphics and draws all its pixels.
    #[test]
    fn a_steam_deck_is_set_up_for_its_screen() {
        let mut config = Config::default();
        assert!(config.first_start());
        let hw = deck();
        assert_eq!(hw.first_level(), None);
        config.set_up_for(&hw);
        let g = &config.graphics;
        assert_eq!((g.preset, g.width, g.height, g.window_mode, g.vsync), (Preset::Auto, 1280, 800, WindowMode::Fullscreen, true));
        assert!(g.set_up && !config.first_start(), "once: the next start leaves the settings as they are");
        assert_eq!(config.gameplay.ui_scale, 1.25);
        assert!(config.launcher.maximized);
        // From the battle's cap: the largest step that leaves it its room, and the launcher its own.
        let room = |scale: f32, (w, h): (f32, f32)| 1280.0 / scale >= w && 800.0 / scale >= h;
        assert!(room(1.25, limits::GAME_POINTS) && room(1.25, limits::LAUNCHER_POINTS));
        assert!(!room(1.30, limits::GAME_POINTS));
        assert!(hw.summary().starts_with("Steam Deck · 8 threads"), "{}", hw.summary());
        // The game, on the Deck's graphics (Vulkan's name for them): Medium, at 100% of its pixels.
        let radv = "AMD Custom GPU 0405 (RADV VANGOGH)";
        let level = Preset::for_gpu(GpuKind::Integrated, radv);
        assert_eq!(level, Preset::Medium);
        let mut g = config.graphics.clone();
        level.apply(&mut g);
        g.preset = level;
        assert!(g.fit_render_scale_once(GpuKind::Integrated, radv, g.picture_pixels(Some((1280, 800)))));
        assert_eq!(g.render_scale, 1.0);
    }

    /// A Linux PC: the first start knows no graphics card, so the level stays Auto for the game
    /// to pick from the card it opens (it was Medium on every one); full screen, the interface as
    /// it comes.
    #[test]
    fn a_linux_pc_leaves_the_level_to_the_game() {
        let hw = Hardware { os: "linux", threads: 16, ram_mb: 32_000, cpu: "AMD Ryzen 7 5800X 8-Core Processor".into(), ..Default::default() };
        assert_eq!(hw.first_level(), None);
        let mut config = Config::default();
        config.set_up_for(&hw);
        let g = &config.graphics;
        assert_eq!((g.preset, (g.width, g.height), g.window_mode, g.gpu.as_str()), (Preset::Auto, (1600, 900), WindowMode::Fullscreen, ""));
        assert!(!config.first_start());
        assert_eq!((config.gameplay.ui_scale, config.launcher.maximized), (1.0, false));
        // The game opens the card: its level, as for any computer.
        assert_eq!(Preset::for_gpu(GpuKind::Discrete, "NVIDIA GeForce RTX 3070"), Preset::High);
        assert_eq!(Preset::for_gpu(GpuKind::Integrated, "Intel(R) UHD Graphics 620 (KBL GT2)"), Preset::Low);
    }

    /// Windows and Macs keep their first start: the level for the card the computer names (an
    /// Intel Mac, whose graphics are not known, Medium as before), and no handheld's settings.
    #[test]
    fn windows_and_macs_keep_their_first_start() {
        let mut windows = pc("NVIDIA GeForce RTX 3050", GpuKind::Discrete, 8_000, (2560, 1440));
        windows.os = "windows";
        let mac = Hardware { gpu: "Apple M5".into(), gpu_kind: Some(GpuKind::Integrated), threads: 10, ram_mb: 24_576, os: "macos", ..Default::default() };
        let intel_mac = Hardware { threads: 12, ram_mb: 16_384, os: "macos", cpu: "Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz".into(), ..Default::default() };
        for (hw, level, size) in [(&windows, Preset::High, (2560, 1440)), (&mac, Preset::Medium, (1600, 900)), (&intel_mac, Preset::Medium, (1600, 900))] {
            let mut config = Config::default();
            config.set_up_for(hw);
            let g = &config.graphics;
            assert_eq!((g.preset, (g.width, g.height), g.window_mode), (level, size, WindowMode::Fullscreen), "{hw:?}");
            assert!(g.set_up && !config.first_start());
            assert_eq!((config.gameplay.ui_scale, config.launcher.maximized), (1.0, false));
            // (Nothing but the graphics.)
            let mut alone = Graphics::default();
            alone.set_up_for(hw);
            assert_eq!(alone, config.graphics);
        }
        assert_eq!(windows.handheld, None);
    }

    /// Windows on a Steam Deck: its graphics' name tells it (Windows gives them a gigabyte of
    /// their own, as a dedicated card has): Medium, full screen, the launcher maximised; the
    /// interface's size is left to Windows, which enlarges a small screen's itself.
    #[test]
    fn windows_on_a_steam_deck() {
        let mut hw = pc("AMD Custom GPU 0405", GpuKind::Discrete, 1_024, (1280, 800));
        hw.os = "windows";
        hw.handheld = Handheld::detect(&hw.gpu);
        assert_eq!(hw.handheld, Some(Handheld::SteamDeck));
        assert_eq!(hw.preset(), Preset::Medium);
        let mut config = Config::default();
        config.set_up_for(&hw);
        let g = &config.graphics;
        assert_eq!((g.preset, (g.width, g.height), g.window_mode, g.vsync), (Preset::Medium, (1280, 800), WindowMode::Fullscreen, true));
        assert!(config.launcher.maximized);
        assert_eq!(config.gameplay.ui_scale, 1.0);
    }

    /// The first start comes once. Settings set up are the player's, and so are files from
    /// before the mark (they have a level of their own); a level put back to Auto in the options
    /// is picked again as the game starts, the window left as the player has it; graphics put
    /// back to how they came (RESET ALL SETTINGS) are set up again.
    #[test]
    fn the_first_start_comes_once() {
        let mut config = Config::default();
        config.set_up_for(&deck());
        config.graphics.window_mode = WindowMode::Windowed;
        config.gameplay.ui_scale = 1.5;
        config.launcher.maximized = false;
        let kept: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        assert!(!kept.first_start(), "the mark is kept in the file");
        assert_eq!((kept.graphics.window_mode, kept.gameplay.ui_scale, kept.launcher.maximized), (WindowMode::Windowed, 1.5, false));
        for old in ["[graphics]\npreset = \"High\"\n", "[graphics]\nmsaa = 8\n"] {
            assert!(!toml::from_str::<Config>(old).unwrap().first_start(), "{old}");
        }
        // AUTO picked again on Windows: the level the computer tells, nothing else touched.
        let mut windows = pc("NVIDIA GeForce RTX 4070", GpuKind::Discrete, 12_000, (2560, 1440));
        windows.os = "windows";
        let mut config = Config::default();
        config.set_up_for(&windows);
        config.graphics.window_mode = WindowMode::Windowed;
        config.graphics.preset = Preset::Auto;
        assert!(!config.first_start());
        config.graphics.pick_level_for(&windows);
        assert_eq!((config.graphics.preset, config.graphics.window_mode), (Preset::Ultra, WindowMode::Windowed));
        // On Linux it waits for the game's card.
        config.graphics.preset = Preset::Auto;
        config.graphics.pick_level_for(&deck());
        assert_eq!(config.graphics.preset, Preset::Auto);
        config.graphics = Graphics::default();
        assert!(config.first_start());
    }
}
