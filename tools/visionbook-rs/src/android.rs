// Android/ADB utilities for visionbook
// Provides screenshot, tap, swipe, and app launch capabilities

use std::process::{Command, Output};
use std::path::Path;
use std::io::{self, Write};

/// Find ADB executable path
pub fn find_adb() -> String {
    let candidates = [
        std::env::var("ANDROID_HOME").ok().map(|h| format!("{}/platform-tools/adb.exe", h)),
        std::env::var("ANDROID_SDK_ROOT").ok().map(|h| format!("{}/platform-tools/adb.exe", h)),
        std::env::var("ADB_PATH").ok().or_else(|| {
        // Try common Android SDK locations
        let paths = [
            dirs::home_dir().map(|h| h.join("AppData/Local/Android/Sdk/platform-tools/adb.exe")),
            Some(std::path::PathBuf::from("adb")),  // System PATH
        ];
        paths.into_iter().flatten().find(|p| p.exists()).map(|p| p.to_string_lossy().to_string())
    }),
        Some("adb".to_string()),
    ];

    for candidate in candidates.into_iter().flatten() {
        if Path::new(&candidate).exists() || candidate == "adb" {
            return candidate;
        }
    }
    "adb".to_string()
}

/// Run an ADB command, optionally targeting a specific device
pub fn run_adb(args: &[&str], device: Option<&str>) -> io::Result<Output> {
    let adb = find_adb();
    let mut cmd = Command::new(&adb);

    if let Some(serial) = device {
        cmd.args(["-s", serial]);
    }

    cmd.args(args).output()
}

/// List connected Android devices
pub fn list_devices() -> io::Result<Vec<(String, String)>> {
    let output = run_adb(&["devices", "-l"], None)?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut devices = Vec::new();
    for line in stdout.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 && parts[1] == "device" {
            let serial = parts[0].to_string();
            let model = parts.iter()
                .find(|p| p.starts_with("model:"))
                .map(|m| m.trim_start_matches("model:").to_string())
                .unwrap_or_else(|| "unknown".to_string());
            devices.push((serial, model));
        }
    }

    Ok(devices)
}

/// Capture screenshot from Android device
pub fn screenshot(output_path: &Path, device: Option<&str>) -> io::Result<()> {
    let output = run_adb(&["exec-out", "screencap", "-p"], device)?;

    if output.status.success() {
        let mut file = std::fs::File::create(output_path)?;
        file.write_all(&output.stdout)?;
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            format!("ADB screenshot failed: {}", String::from_utf8_lossy(&output.stderr))
        ))
    }
}

/// Tap at coordinates on Android device
pub fn tap(x: i32, y: i32, device: Option<&str>) -> io::Result<()> {
    let output = run_adb(
        &["shell", "input", "tap", &x.to_string(), &y.to_string()],
        device
    )?;

    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            format!("ADB tap failed: {}", String::from_utf8_lossy(&output.stderr))
        ))
    }
}

/// Swipe on Android device
pub fn swipe(x1: i32, y1: i32, x2: i32, y2: i32, duration_ms: u32, device: Option<&str>) -> io::Result<()> {
    let output = run_adb(
        &[
            "shell", "input", "swipe",
            &x1.to_string(), &y1.to_string(),
            &x2.to_string(), &y2.to_string(),
            &duration_ms.to_string()
        ],
        device
    )?;

    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            format!("ADB swipe failed: {}", String::from_utf8_lossy(&output.stderr))
        ))
    }
}

/// Launch an Android app/activity
pub fn launch(component: &str, device: Option<&str>) -> io::Result<()> {
    let output = run_adb(
        &["shell", "am", "start", "-n", component],
        device
    )?;

    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            format!("ADB launch failed: {}", String::from_utf8_lossy(&output.stderr))
        ))
    }
}

/// Get screen size of Android device
pub fn screen_size(device: Option<&str>) -> io::Result<(u32, u32)> {
    let output = run_adb(&["shell", "wm", "size"], device)?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse "Physical size: 1080x2408"
    for line in stdout.lines() {
        if line.contains("Physical size:") {
            if let Some(size_str) = line.split(':').nth(1) {
                let parts: Vec<&str> = size_str.trim().split('x').collect();
                if parts.len() == 2 {
                    if let (Ok(w), Ok(h)) = (parts[0].parse(), parts[1].parse()) {
                        return Ok((w, h));
                    }
                }
            }
        }
    }

    Err(io::Error::new(io::ErrorKind::Other, "Could not parse screen size"))
}

/// Press back button
pub fn back(device: Option<&str>) -> io::Result<()> {
    let output = run_adb(&["shell", "input", "keyevent", "KEYCODE_BACK"], device)?;

    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            format!("ADB back failed: {}", String::from_utf8_lossy(&output.stderr))
        ))
    }
}

/// Press home button
pub fn home(device: Option<&str>) -> io::Result<()> {
    let output = run_adb(&["shell", "input", "keyevent", "KEYCODE_HOME"], device)?;

    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            format!("ADB home failed: {}", String::from_utf8_lossy(&output.stderr))
        ))
    }
}
