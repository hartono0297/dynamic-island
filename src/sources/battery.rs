use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

use crate::model::{Notice, NoticeKind};

/// `BatteryLifePercent` value meaning the charge level is unknown.
const PERCENT_UNKNOWN: u8 = 255;
/// `BatteryFlag` bit set when the machine has no system battery.
const FLAG_NO_BATTERY: u8 = 128;
/// `ACLineStatus` value meaning the charger is connected.
const AC_ONLINE: u8 = 1;

/// Decides when to raise the low-battery notice: once per discharge below the
/// threshold, re-armed by charging or by rising back above the threshold.
pub struct BatteryMonitor {
    threshold: u8,
    alerted: bool,
}

impl BatteryMonitor {
    pub fn new(threshold: u8) -> Self {
        Self {
            threshold,
            alerted: false,
        }
    }

    /// True when the alert should be raised now.
    pub fn update(&mut self, percent: u8, charging: bool) -> bool {
        if charging || percent > self.threshold {
            self.alerted = false;
            return false;
        }
        if self.alerted {
            return false;
        }
        self.alerted = true;
        true
    }
}

/// Current charge percentage and whether the charger is connected, or `None`
/// when there is no battery or its level is unknown.
pub fn read_battery() -> Option<(u8, bool)> {
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut status) }.ok()?;
    if status.BatteryLifePercent == PERCENT_UNKNOWN || status.BatteryFlag & FLAG_NO_BATTERY != 0 {
        return None;
    }
    Some((status.BatteryLifePercent, status.ACLineStatus == AC_ONLINE))
}

pub fn battery_notice(percent: u8) -> Notice {
    Notice {
        key: "battery".to_string(),
        source: "Baterai".to_string(),
        title: "Baterai lemah".to_string(),
        body: format!("Sisa {percent}%. Colokin charger."),
        kind: NoticeKind::BatteryLow,
    }
}

/// Diagnostic mode for `--probe-system`: prints the battery and fullscreen
/// state every second until Ctrl+C.
#[cfg(debug_assertions)]
pub fn probe() {
    println!("Polling battery and fullscreen state; press Ctrl+C to stop.");
    loop {
        println!(
            "battery={:?} fullscreen={}",
            read_battery(),
            super::fullscreen::is_fullscreen()
        );
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alerts_once_when_crossing() {
        let mut b = BatteryMonitor::new(20);
        assert!(!b.update(25, false));
        assert!(b.update(20, false));
        assert!(!b.update(19, false));
    }

    #[test]
    fn charging_never_alerts_and_rearms() {
        let mut b = BatteryMonitor::new(20);
        assert!(b.update(18, false));
        assert!(!b.update(18, true));
        assert!(b.update(18, false));
    }

    #[test]
    fn rearms_after_rising_above_threshold() {
        let mut b = BatteryMonitor::new(20);
        assert!(b.update(19, false));
        assert!(!b.update(30, false));
        assert!(b.update(20, false));
    }

    #[test]
    fn notice_text() {
        let n = battery_notice(15);
        assert_eq!(n.key, "battery");
        assert_eq!(n.source, "Baterai");
        assert_eq!(n.title, "Baterai lemah");
        assert_eq!(n.body, "Sisa 15%. Colokin charger.");
        assert_eq!(n.kind, NoticeKind::BatteryLow);
    }
}
