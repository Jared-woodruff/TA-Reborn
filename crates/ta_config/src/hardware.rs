//! What the computer is, for the first start: the graphics card and the memory of its own,
//! the processor's threads, the memory, and the main screen. The first start (the
//! launcher's or the game's, whichever comes first) sets the game up from it: the graphics
//! level for the card, the screen's own size, full screen. After that the settings are the
//! player's ([`crate::Graphics::preset`] is no longer Auto).

use crate::{GpuKind, Graphics, Preset, WindowMode};

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
}

impl Hardware {
    /// Looks at this computer (on Windows; on Linux its memory and processor; elsewhere only the
    /// threads are known).
    pub fn detect() -> Hardware {
        #[cfg_attr(not(any(windows, target_os = "linux")), allow(unused_mut))]
        let mut hw = Hardware { threads: std::thread::available_parallelism().map_or(0, |n| n.get()), ..Default::default() };
        #[cfg(windows)]
        win::detect(&mut hw);
        #[cfg(target_os = "linux")]
        linux::detect(&mut hw);
        hw
    }

    /// The graphics level for this computer: the card's kind and name (see
    /// [`Preset::for_gpu`]), on a dedicated card its memory (Ultra from 10 GB, High from
    /// 6 GB, Medium from 3 GB), one level less on a 4K screen without 12 GB, and no more
    /// than Medium with fewer than four threads or 8 GB.
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
        let big_screen = self.screen.is_some_and(|(w, h)| w as u64 * h as u64 >= 3840 * 2160);
        if big_screen && self.vram_mb < 11_500 {
            level = lower(level);
        }
        if (self.threads > 0 && self.threads < 4) || (self.ram_mb > 0 && self.ram_mb < 7_500) {
            level = level.min_level(Preset::Medium);
        }
        level
    }

    /// A line for the launcher: "NVIDIA GeForce RTX 3050 (8 GB) · 8 threads · 32 GB".
    pub fn summary(&self) -> String {
        let gb = |mb: u64| format!("{:.0} GB", mb as f64 / 1024.0);
        let mut parts = Vec::new();
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
    /// The first start: the level for this computer, the screen's own size, full screen.
    pub fn set_up_for(&mut self, hw: &Hardware) {
        let level = hw.preset();
        level.apply(self);
        self.preset = level;
        if let Some((w, h)) = hw.screen.filter(|&(w, h)| w >= 640 && h >= 480) {
            self.width = w;
            self.height = h;
        }
        self.window_mode = WindowMode::Fullscreen;
        if !hw.gpu.is_empty() {
            self.gpu = hw.gpu.clone();
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

    #[test]
    fn the_first_start_sets_up_full_screen() {
        let mut g = Graphics::default();
        assert_eq!(g.preset, Preset::Auto);
        g.set_up_for(&pc("NVIDIA GeForce RTX 3050", GpuKind::Discrete, 8_000, (2560, 1440)));
        assert_eq!((g.preset, g.width, g.height, g.window_mode), (Preset::High, 2560, 1440, WindowMode::Fullscreen));
        assert_eq!(g.msaa, 4);
    }

    #[test]
    fn this_computer_is_detected() {
        let hw = Hardware::detect();
        println!("{hw:?} -> {:?}", hw.preset());
        assert!(hw.threads > 0);
    }
}
