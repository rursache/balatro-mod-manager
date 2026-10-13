use std::fs;
use sysinfo::System;

use crate::links::APP_REPO_URL;

const ISSUE_DESCRIPTION_MAX_CHARS: usize = 1200;
const ISSUE_URL_MAX_LEN: usize = 7000;

struct SystemSummary {
    os: String,
    arch: String,
    cpu: String,
    gpu: String,
    ram: String,
}

fn collect_system_summary() -> SystemSummary {
    let (cpu, ram) = get_cpu_and_ram();
    let os = std::env::consts::OS.to_string();
    let os = match detect_os_version() {
        Some(v) => format!("{os} | {v}"),
        None => os,
    };
    SystemSummary {
        os,
        arch: std::env::consts::ARCH.to_string(),
        cpu,
        gpu: detect_gpu().unwrap_or_else(|| "Unknown".to_string()),
        ram,
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((idx, _)) => format!("{}...", &s[..idx]),
        None => s.to_string(),
    }
}

/// Prefilled GitHub issue form URL, kept under a length GitHub accepts
fn issue_url(title: &str, description: &str, mm_version: &str, sys: &SystemSummary) -> String {
    let build = |desc: &str| {
        let info = format!(
            "{desc}\n\n### System\n- OS: {}\n- Arch: {}\n- CPU: {}\n- GPU: {}\n- RAM: {}\n\nPlease attach the log file (it was copied to your clipboard)",
            sys.os, sys.arch, sys.cpu, sys.gpu, sys.ram
        );
        format!(
            "{APP_REPO_URL}/issues/new?template=bug_report.yml&title={}&version={}&additional-info={}",
            urlencoding::encode(title.trim()),
            urlencoding::encode(mm_version.trim()),
            urlencoding::encode(&info)
        )
    };
    let mut max = ISSUE_DESCRIPTION_MAX_CHARS;
    loop {
        let url = build(&truncate_chars(description.trim(), max));
        if url.len() <= ISSUE_URL_MAX_LEN || max == 0 {
            return url;
        }
        max /= 2;
    }
}

/// Builds the GitHub issue URL the frontend opens for bug reports
#[tauri::command]
pub async fn build_issue_report_url(
    title: String,
    description: String,
    mm_version: String,
) -> Result<String, String> {
    let sys = tauri::async_runtime::spawn_blocking(collect_system_summary)
        .await
        .map_err(|e| e.to_string())?;
    Ok(issue_url(&title, &description, &mm_version, &sys))
}

/// Returns the latest log's filename and text, if any.
#[tauri::command]
pub async fn get_latest_log() -> Result<(String, String), String> {
    latest_log().ok_or_else(|| "No logs found.".to_string())
}

#[tauri::command]
pub async fn get_logs_folder() -> Result<String, String> {
    let dir = dirs::config_dir()
        .ok_or_else(|| "config directory not found".to_string())?
        .join("Balatro")
        .join("logs");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.to_string_lossy().to_string())
}

fn latest_log() -> Option<(String, String)> {
    let dir = dirs::config_dir()?.join("Balatro").join("logs");
    let entries = fs::read_dir(&dir).ok()?;
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let p = e.path();
            p.extension().is_some_and(|ext| ext == "log")
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("bmm_"))
        })
        .collect();

    files.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    let path = files.last()?.path();
    let filename = path.file_name()?.to_string_lossy().to_string();
    let text = fs::read_to_string(&path).ok()?;
    Some((filename, text))
}

fn get_cpu_and_ram() -> (String, String) {
    let mut sys = System::new_all();
    sys.refresh_all();
    let cpu = detect_cpu();

    // Try to produce a sane GB value despite sysinfo changes over versions
    let tm = sys.total_memory();
    let gib_from_bytes = (tm as f64) / (1024.0 * 1024.0 * 1024.0);
    let gib_from_kib = (tm as f64) / (1024.0 * 1024.0);

    let gb = if (1.0..=1024.0).contains(&gib_from_bytes) {
        gib_from_bytes
    } else if (1.0..=1024.0).contains(&gib_from_kib) {
        gib_from_kib
    } else {
        // Fallback: prefer bytes path
        gib_from_bytes
    };
    let ram_str = format!("{}GB", (gb.round() as u64).max(1));
    (cpu, ram_str)
}

/// Returns a human-friendly OS version string, if available.
///
/// Examples:
/// - Windows: "Microsoft Windows 11 Pro 10.0.22631 (Build 3880)"
/// - macOS:   "macOS 14.5 (23F79)"
/// - Linux:   distro + kernel if available
fn detect_os_version() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        // Prefer PowerShell CIM query (wmic deprecated on newer Windows)
        let ps = r#"
            $o = Get-CimInstance Win32_OperatingSystem; 
            Write-Output ($o.Caption + ' ' + $o.Version + ' (Build ' + $o.BuildNumber + ')')
        "#;
        if let Ok(out) = Command::new("powershell")
            .args(["-NoProfile", "-Command", ps])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }

        // Fallback: wmic
        if let Ok(out) = Command::new("wmic")
            .args(["os", "get", "Caption,Version,BuildNumber", "/value"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                let mut caption = String::new();
                let mut version = String::new();
                let mut build = String::new();
                for line in s.lines() {
                    let l = line.trim();
                    if let Some(v) = l.strip_prefix("Caption=") {
                        caption = v.trim().to_string();
                    } else if let Some(v) = l.strip_prefix("Version=") {
                        version = v.trim().to_string();
                    } else if let Some(v) = l.strip_prefix("BuildNumber=") {
                        build = v.trim().to_string();
                    }
                }
                if !caption.is_empty() && !version.is_empty() && !build.is_empty() {
                    return Some(format!("{} {} (Build {})", caption, version, build));
                }
            }
        }

        // Last resort: sysinfo long_os_version + kernel
        let mut sys = System::new_all();
        sys.refresh_all();
        let long = System::long_os_version().or_else(System::os_version);
        let kernel = System::kernel_version();
        if let Some(l) = long {
            if let Some(k) = kernel {
                return Some(format!("{} (Kernel {})", l, k));
            }
            return Some(l);
        }
        None
    }

    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        let name = Command::new("sw_vers").arg("-productName").output().ok();
        let version = Command::new("sw_vers").arg("-productVersion").output().ok();
        let build = Command::new("sw_vers").arg("-buildVersion").output().ok();

        let clean = |o: Option<std::process::Output>| -> Option<String> {
            o.and_then(|out| {
                if out.status.success() {
                    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
                } else {
                    None
                }
            })
        };

        let name = clean(name).unwrap_or_else(|| "macOS".to_string());
        let version = clean(version).unwrap_or_default();
        let build = clean(build);
        if !version.is_empty() {
            return Some(match build {
                Some(b) if !b.is_empty() => format!("{} {} ({})", name, version, b),
                _ => format!("{} {}", name, version),
            });
        }

        // Fallback: sysinfo long_os_version
        let mut sys = System::new_all();
        sys.refresh_all();
        if let Some(l) = System::long_os_version().or_else(System::os_version) {
            return Some(l);
        }
        Some(name)
    }

    #[cfg(target_os = "linux")]
    {
        use std::fs;

        // Try /etc/os-release PRETTY_NAME
        if let Ok(s) = fs::read_to_string("/etc/os-release") {
            let mut pretty = None;
            for line in s.lines() {
                if let Some(v) = line.strip_prefix("PRETTY_NAME=") {
                    let v = v.trim_matches('"').to_string();
                    if !v.is_empty() {
                        pretty = Some(v);
                        break;
                    }
                }
            }
            if let Some(p) = pretty {
                return Some(p);
            }
        }

        // Fallback: sysinfo long_os_version + kernel
        let mut sys = System::new_all();
        sys.refresh_all();
        let long = System::long_os_version().or_else(System::os_version);
        let kernel = System::kernel_version();
        match (long, kernel) {
            (Some(l), Some(k)) => Some(format!("{} (Kernel {})", l, k)),
            (Some(l), None) => Some(l),
            (None, Some(k)) => Some(format!("Linux (Kernel {})", k)),
            _ => None,
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let mut sys = System::new_all();
        sys.refresh_all();
        System::long_os_version()
            .or_else(System::os_version)
            .or_else(|| Some(std::env::consts::OS.to_string()))
    }
}

// Best-effort GPU detection; returns None if not supported
#[cfg(target_os = "macos")]
fn detect_gpu() -> Option<String> {
    use std::process::Command;
    let out = Command::new("system_profiler")
        .arg("SPDisplaysDataType")
        .arg("-detailLevel")
        .arg("mini")
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    for line in s.lines() {
        let l = line.trim();
        if l.starts_with("Chipset Model:") || l.starts_with("Chipset:") {
            return Some(
                l.replace("Chipset Model:", "")
                    .replace("Chipset:", "")
                    .trim()
                    .to_string(),
            );
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn detect_gpu() -> Option<String> {
    // Using wmic is deprecated on newer Windows; keep it best-effort
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let out = Command::new("wmic")
        .args(["path", "win32_VideoController", "get", "name"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let mut lines = s.lines().skip(1).filter(|l| !l.trim().is_empty());
    lines.next().map(|l| l.trim().to_string())
}

#[cfg(target_os = "linux")]
fn detect_gpu() -> Option<String> {
    use std::process::Command;
    let out = Command::new("sh")
        .arg("-c")
        .arg("lspci | grep -Ei 'vga|3d|display' | head -n1")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    Some(s.trim().to_string())
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn detect_gpu() -> Option<String> {
    None
}

// Platform-specific CPU detection (best effort)
fn detect_cpu() -> String {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        if let Ok(out) = Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
            && out.status.success()
        {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        if let Ok(out) = Command::new("wmic")
            .args(["cpu", "get", "name"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                if let Some(name) = s.lines().skip(1).find(|l| !l.trim().is_empty()) {
                    return name.trim().to_string();
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        if let Ok(out) = Command::new("sh")
            .arg("-c")
            .arg("lscpu | grep 'Model name' | cut -d: -f2")
            .output()
            && out.status.success()
        {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
        if let Ok(out) = Command::new("sh")
            .arg("-c")
            .arg("grep -m1 'model name' /proc/cpuinfo | cut -d: -f2")
            .output()
            && out.status.success()
        {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    "Unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sys() -> SystemSummary {
        SystemSummary {
            os: "macos | macOS 26.0".into(),
            arch: "aarch64".into(),
            cpu: "Apple M4".into(),
            gpu: "Apple M4".into(),
            ram: "16GB".into(),
        }
    }

    #[test]
    fn issue_url_encodes_and_includes_template_version_and_caps_length() {
        let url = issue_url("Crash & burn", "It broke\nbadly", "0.4.1", &sys());
        assert!(url.starts_with(
            "https://github.com/rursache/balatro-mod-manager/issues/new?template=bug_report.yml&title=Crash%20%26%20burn&version=0.4.1&additional-info="
        ));
        assert!(url.contains("It%20broke%0Abadly"));
        assert!(url.contains("Apple%20M4"));
        assert!(!url.contains(' '));

        let long = "x".repeat(50_000);
        let capped = issue_url("t", &long, "0.4.1", &sys());
        assert!(capped.len() <= ISSUE_URL_MAX_LEN);
        assert!(capped.contains("xxx..."));

        let wide = "\u{1F600}".repeat(5_000);
        assert!(issue_url("t", &wide, "0.4.1", &sys()).len() <= ISSUE_URL_MAX_LEN);
    }

    #[test]
    fn test_get_cpu_and_ram_returns_valid_format() {
        let (cpu, ram) = get_cpu_and_ram();
        // CPU should be non-empty (might be "Unknown")
        assert!(!cpu.is_empty());
        // RAM should end with "GB"
        assert!(
            ram.ends_with("GB"),
            "RAM format should end with GB: {}",
            ram
        );
    }

    #[test]
    fn test_detect_cpu_returns_string() {
        let cpu = detect_cpu();
        assert!(!cpu.is_empty());
    }

    #[test]
    fn test_detect_os_version_returns_option() {
        // Just verify it doesn't panic
        let _version = detect_os_version();
    }

    #[test]
    fn test_detect_gpu_returns_option() {
        // Just verify it doesn't panic
        let _gpu = detect_gpu();
    }
}
