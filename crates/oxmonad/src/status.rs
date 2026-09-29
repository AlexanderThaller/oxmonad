//! The bar's clock and battery modules (xmobar's `Run Date` and
//! `Run Battery`).

use std::{
    fs,
    path::{
        Path,
        PathBuf,
    },
    time::Duration,
};

use chrono::{
    DateTime,
    Local,
    Timelike,
};
use oxmonad_ui::{
    BatteryState,
    BatteryStatus,
};

/// xmobar's `Run Date "%Y.%m.%d-%u %H:%M"`.
pub const DATE_FORMAT: &str = "%Y.%m.%d-%u %H:%M";

/// How often the battery is polled.
pub const BATTERY_INTERVAL: Duration = Duration::from_secs(30);

/// The formatted current local time.
#[must_use]
pub fn clock_text(now: &DateTime<Local>) -> String {
    now.format(DATE_FORMAT).to_string()
}

/// Time until the next full minute, when the clock text changes.
#[must_use]
pub fn until_next_minute(now: &DateTime<Local>) -> Duration {
    let into_minute = Duration::new(u64::from(now.second()), now.nanosecond() % 1_000_000_000);
    Duration::from_secs(60).saturating_sub(into_minute) + Duration::from_millis(5)
}

/// The batteries (`BAT*`) under a power-supply sysfs directory.
fn batteries(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("BAT"))
        .map(|entry| entry.path())
        .collect();
    found.sort();
    found
}

/// Reads the first battery under `root` (normally
/// `/sys/class/power_supply`); `None` without a battery.
#[must_use]
pub fn read_battery(root: &Path) -> Option<BatteryState> {
    let dir = batteries(root).into_iter().next()?;
    let read = |name: &str| fs::read_to_string(dir.join(name)).ok();
    parse_battery(&|name| read(name))
}

/// Parses a battery from its sysfs attributes (`read(name)` returns the
/// content of `name`). Energy (µWh/µW) and charge (µAh/µA) attributes are
/// both understood; `capacity` is used when the totals are missing.
#[must_use]
pub fn parse_battery(read: &dyn Fn(&str) -> Option<String>) -> Option<BatteryState> {
    let number = |name: &str| -> Option<f64> { read(name)?.trim().parse().ok() };
    let status = match read("status").as_deref().map(str::trim) {
        Some("Charging") => BatteryStatus::Charging,
        Some("Discharging") => BatteryStatus::Discharging,
        Some("Full" | "Not charging") => BatteryStatus::Full,
        _ => BatteryStatus::Unknown,
    };
    let (now, full, rate) = match (number("energy_now"), number("energy_full")) {
        (Some(now), Some(full)) => (Some(now), Some(full), number("power_now")),
        _ => (
            number("charge_now"),
            number("charge_full"),
            number("current_now"),
        ),
    };
    let percent = match (now, full) {
        (Some(now), Some(full)) if full > 0.0 => now / full * 100.0,
        _ => number("capacity")?,
    };
    let time_left = match (status, now, rate) {
        (BatteryStatus::Discharging, Some(now), Some(rate)) if rate > 0.0 => {
            Some(Duration::from_secs_f64(now / rate * 3600.0))
        }
        _ => None,
    };
    Some(BatteryState {
        percent: to_percent(percent),
        status,
        time_left,
    })
}

/// Rounds a percentage into `0..=100`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is rounded and clamped into 0..=100"
)]
fn to_percent(value: f64) -> u8 {
    value.round().clamp(0.0, 100.0) as u8
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::TimeZone;

    use super::*;

    fn battery(attrs: &[(&str, &str)]) -> Option<BatteryState> {
        let map: HashMap<String, String> = attrs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), format!("{v}\n")))
            .collect();
        parse_battery(&|name| map.get(name).cloned())
    }

    #[test]
    fn energy_battery_discharging() {
        let state = battery(&[
            ("status", "Discharging"),
            ("energy_now", "25000000"),
            ("energy_full", "50000000"),
            ("power_now", "10000000"),
        ])
        .expect("battery");
        assert_eq!(state.percent, 50);
        assert_eq!(state.status, BatteryStatus::Discharging);
        assert_eq!(state.time_left, Some(Duration::from_mins(150)));
    }

    #[test]
    fn charge_battery_charging() {
        let state = battery(&[
            ("status", "Charging"),
            ("charge_now", "3000000"),
            ("charge_full", "4000000"),
            ("current_now", "1000000"),
        ])
        .expect("battery");
        assert_eq!(state.percent, 75);
        assert_eq!(state.status, BatteryStatus::Charging);
        assert_eq!(state.time_left, None);
    }

    #[test]
    fn capacity_fallback_and_full() {
        let state = battery(&[("status", "Full"), ("capacity", "100")]).expect("battery");
        assert_eq!(state.percent, 100);
        assert_eq!(state.status, BatteryStatus::Full);
        let state = battery(&[("status", "Not charging"), ("capacity", "87")]).expect("battery");
        assert_eq!(state.status, BatteryStatus::Full);
        assert_eq!(state.percent, 87);
    }

    #[test]
    fn no_rate_means_no_time_and_garbage_is_none() {
        let state = battery(&[
            ("status", "Discharging"),
            ("energy_now", "1000"),
            ("energy_full", "2000"),
            ("power_now", "0"),
        ])
        .expect("battery");
        assert_eq!(state.time_left, None);
        assert_eq!(battery(&[("status", "Unknown")]), None);
        assert_eq!(
            battery(&[("status", "weird"), ("capacity", "12")]).map(|s| s.status),
            Some(BatteryStatus::Unknown)
        );
    }

    #[test]
    fn reads_first_battery_from_sysfs_dir() {
        let root = std::env::temp_dir().join(format!("oxmonad-bat-{}", std::process::id()));
        let bat = root.join("BAT1");
        fs::create_dir_all(&bat).expect("mkdir");
        fs::create_dir_all(root.join("AC")).expect("mkdir");
        fs::write(bat.join("status"), "Charging\n").expect("write");
        fs::write(bat.join("capacity"), "42\n").expect("write");
        let state = read_battery(&root);
        fs::remove_dir_all(&root).expect("cleanup");
        assert_eq!(state.map(|s| s.percent), Some(42));
        assert_eq!(read_battery(Path::new("/nonexistent/oxmonad")), None);
    }

    #[test]
    fn clock() {
        let time = Local
            .with_ymd_and_hms(2026, 9, 29, 13, 5, 30)
            .single()
            .expect("valid time");
        assert_eq!(clock_text(&time), "2026.09.29-2 13:05");
        let wait = until_next_minute(&time);
        assert!(wait > Duration::from_secs(29) && wait < Duration::from_secs(31));
    }
}
