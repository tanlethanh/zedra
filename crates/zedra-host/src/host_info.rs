use std::time::{SystemTime, UNIX_EPOCH};

use sysinfo::System;
use zedra_rpc::proto::{HostBatteryInfo, HostBatteryState, HostInfoSnapshot};

pub const HOST_INFO_SAMPLE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

pub fn collect_host_info_snapshot(system: &mut System) -> HostInfoSnapshot {
    system.refresh_memory();
    system.refresh_cpu_usage();

    HostInfoSnapshot {
        captured_at_ms: now_ms(),
        cpu_usage_percent: system.global_cpu_usage(),
        cpu_count: system.cpus().len() as u32,
        memory_used_bytes: system.used_memory(),
        memory_total_bytes: system.total_memory(),
        swap_used_bytes: system.used_swap(),
        swap_total_bytes: system.total_swap(),
        system_uptime_secs: System::uptime(),
        batteries: collect_batteries(),
    }
}

pub fn new_system_sampler() -> System {
    System::new_all()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn collect_batteries() -> Vec<HostBatteryInfo> {
    let output = std::process::Command::new("pmset")
        .args(["-g", "batt"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let Ok(stdout) = String::from_utf8(output.stdout) else {
        return Vec::new();
    };

    stdout
        .lines()
        .filter(|line| line.contains("%;"))
        .enumerate()
        .filter_map(|(index, line)| parse_pmset_battery_line(index as u32, line))
        .collect()
}

#[cfg(target_os = "macos")]
fn parse_pmset_battery_line(index: u32, line: &str) -> Option<HostBatteryInfo> {
    Some(HostBatteryInfo {
        index,
        charge_percent: parse_charge_percent(line),
        state: parse_battery_state(line),
        time_remaining_secs: parse_pmset_time_remaining(line),
    })
}

#[cfg(target_os = "macos")]
fn parse_pmset_time_remaining(line: &str) -> Option<u64> {
    let time_token = line
        .split(';')
        .find(|part| part.contains("remaining"))?
        .split_whitespace()
        .find(|token| token.contains(':'))?;
    let (hours, minutes) = time_token.split_once(':')?;
    let hours = hours.parse::<u64>().ok()?;
    let minutes = minutes.parse::<u64>().ok()?;
    Some((hours * 60 + minutes) * 60)
}

#[cfg(target_os = "linux")]
fn collect_batteries() -> Vec<HostBatteryInfo> {
    let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let power_type = read_trimmed(path.join("type"));
            if power_type.as_deref() != Some("Battery") && !name.starts_with("BAT") {
                return None;
            }

            Some((path, name))
        })
        .enumerate()
        .map(|(index, (path, _name))| {
            let status = read_trimmed(path.join("status")).unwrap_or_default();
            let charge_percent = read_trimmed(path.join("capacity"))
                .and_then(|v| v.parse::<u8>().ok())
                .map(|v| v.min(100));

            HostBatteryInfo {
                index: index as u32,
                charge_percent,
                state: parse_battery_state(&status),
                time_remaining_secs: None,
            }
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn read_trimmed(path: impl AsRef<std::path::Path>) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn collect_batteries() -> Vec<HostBatteryInfo> {
    Vec::new()
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn parse_charge_percent(value: &str) -> Option<u8> {
    let percent_index = value.find('%')?;
    let prefix = &value[..percent_index];
    let start = prefix
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_ascii_digit())
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0);
    prefix[start..].parse::<u8>().ok().map(|v| v.min(100))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn parse_battery_state(value: &str) -> HostBatteryState {
    let value = value.to_ascii_lowercase();
    if value.contains("discharging") {
        HostBatteryState::Discharging
    } else if value.contains("not charging") {
        HostBatteryState::NotCharging
    } else if value.contains("charged") || value.contains("full") {
        HostBatteryState::Full
    } else if value.contains("charging") {
        HostBatteryState::Charging
    } else if value.contains("empty") {
        HostBatteryState::Empty
    } else {
        HostBatteryState::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn parses_pmset_battery_line() {
        let line =
            " -InternalBattery-0 (id=1234567)\t87%; discharging; 2:14 remaining present: true";
        let battery = parse_pmset_battery_line(0, line).unwrap();

        assert_eq!(battery.index, 0);
        assert_eq!(battery.charge_percent, Some(87));
        assert_eq!(battery.state, HostBatteryState::Discharging);
        assert_eq!(battery.time_remaining_secs, Some((2 * 60 + 14) * 60));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn parses_battery_state_without_matching_discharging_as_charging() {
        assert_eq!(
            parse_battery_state("discharging"),
            HostBatteryState::Discharging
        );
        assert_eq!(parse_battery_state("charging"), HostBatteryState::Charging);
        assert_eq!(
            parse_battery_state("not charging"),
            HostBatteryState::NotCharging
        );
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn parses_charge_percent_from_status_line() {
        assert_eq!(parse_charge_percent("BAT0\t100%; charged"), Some(100));
        assert_eq!(parse_charge_percent("capacity: 7%"), Some(7));
        assert_eq!(parse_charge_percent("no percent"), None);
    }
}
