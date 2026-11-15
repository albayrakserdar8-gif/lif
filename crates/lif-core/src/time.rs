//! Wall-clock time supplied by the caller, and the small strftime subset
//! formats need. The crate never reads a clock itself.

/// A point in wall-clock time as the server sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WallClock {
    /// Seconds since the Unix epoch (UTC).
    pub unix_secs: i64,
    /// Offset of local time from UTC in seconds (e.g. 10800 for UTC+3), as
    /// returned by `localtime_r`'s `tm_gmtoff`. Used for everything shown
    /// to the user (status clock, `#{t:...}`).
    pub utc_offset_secs: i32,
}

impl WallClock {
    /// Construct from epoch seconds and a UTC offset.
    pub const fn new(unix_secs: i64, utc_offset_secs: i32) -> Self {
        Self { unix_secs, utc_offset_secs }
    }

    /// The same offset, a different instant (used for stored epoch times).
    pub(crate) const fn at(self, unix_secs: i64) -> Self {
        Self { unix_secs, utc_offset_secs: self.utc_offset_secs }
    }
}

/// Broken-down local time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Tm {
    pub year: i64,
    /// 1..=12
    pub month: u32,
    /// 1..=31
    pub day: u32,
    pub hour: u32,
    pub min: u32,
    pub sec: u32,
    /// 0 = Sunday
    pub wday: u32,
    /// 0..=365
    pub yday: u32,
}

const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
    "December",
];

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

impl Tm {
    pub fn from_clock(c: WallClock) -> Tm {
        // Saturating: `#{t:...}` takes any i64 from the user, and an
        // overflow here would take the server down. Far-off instants just
        // print an absurd (but well-formed) year.
        let t = c.unix_secs.saturating_add(c.utc_offset_secs as i64);
        let days = t.div_euclid(86400);
        let secs = t.rem_euclid(86400) as u32;
        // Howard Hinnant's civil_from_days.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = if m <= 2 { y + 1 } else { y };
        let wday = (days + 4).rem_euclid(7) as u32;
        let cum = [0u32, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
        let mut yday = cum[(m - 1) as usize] + d - 1;
        if m > 2 && is_leap(year) {
            yday += 1;
        }
        Tm { year, month: m, day: d, hour: secs / 3600, min: secs / 60 % 60, sec: secs % 60, wday, yday }
    }
}

/// Expand strftime conversions in `fmt`. Unknown conversions are kept
/// literally (tmux passes them to strftime, which does the same on macOS).
pub(crate) fn strftime(fmt: &str, tm: &Tm) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(fmt.len() + 16);
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(&spec) = chars.peek() else {
            out.push('%');
            break;
        };
        chars.next();
        let h12 = if tm.hour.is_multiple_of(12) { 12 } else { tm.hour % 12 };
        let _ = match spec {
            '%' => write!(out, "%"),
            'a' => write!(out, "{}", &DAYS[tm.wday as usize][..3]),
            'A' => write!(out, "{}", DAYS[tm.wday as usize]),
            'b' | 'h' => write!(out, "{}", &MONTHS[tm.month as usize - 1][..3]),
            'B' => write!(out, "{}", MONTHS[tm.month as usize - 1]),
            'd' => write!(out, "{:02}", tm.day),
            'e' => write!(out, "{:2}", tm.day),
            'H' => write!(out, "{:02}", tm.hour),
            'I' => write!(out, "{:02}", h12),
            'k' => write!(out, "{:2}", tm.hour),
            'l' => write!(out, "{:2}", h12),
            'j' => write!(out, "{:03}", tm.yday + 1),
            'm' => write!(out, "{:02}", tm.month),
            'M' => write!(out, "{:02}", tm.min),
            'p' => write!(out, "{}", if tm.hour < 12 { "AM" } else { "PM" }),
            'S' => write!(out, "{:02}", tm.sec),
            'u' => write!(out, "{}", if tm.wday == 0 { 7 } else { tm.wday }),
            'w' => write!(out, "{}", tm.wday),
            'y' => write!(out, "{:02}", tm.year.rem_euclid(100)),
            'Y' => write!(out, "{}", tm.year),
            'C' => write!(out, "{:02}", tm.year.div_euclid(100)),
            'D' => write!(out, "{:02}/{:02}/{:02}", tm.month, tm.day, tm.year.rem_euclid(100)),
            'F' => write!(out, "{}-{:02}-{:02}", tm.year, tm.month, tm.day),
            'R' => write!(out, "{:02}:{:02}", tm.hour, tm.min),
            'T' => write!(out, "{:02}:{:02}:{:02}", tm.hour, tm.min, tm.sec),
            'n' => {
                out.push('\n');
                Ok(())
            }
            't' => {
                out.push('\t');
                Ok(())
            }
            other => write!(out, "%{other}"),
        };
    }
    out
}

/// `ctime`-style text without the newline: `Sat Nov 14 21:51:54 2026`.
pub(crate) fn ctime(tm: &Tm) -> String {
    strftime("%a %b %e %H:%M:%S %Y", tm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        // Verified against tmux 3.8 on this machine (UTC+3):
        // #{session_created}=1794682220 printed as "Sat Nov 14 21:50:20 2026".
        let tm = Tm::from_clock(WallClock::new(1_794_682_220, 3 * 3600));
        assert_eq!(ctime(&tm), "Sat Nov 14 21:50:20 2026");
        assert_eq!(strftime("%H:%M %d-%b-%y", &tm), "21:50 14-Nov-26");
        let epoch = Tm::from_clock(WallClock::new(0, 0));
        assert_eq!(ctime(&epoch), "Thu Jan  1 00:00:00 1970");
        let leap = Tm::from_clock(WallClock::new(951_825_600, 0)); // 2000-02-29 12:00
        assert_eq!(strftime("%F %T %j %A %B %I%p", &leap), "2000-02-29 12:00:00 060 Tuesday February 12PM");
        let neg = Tm::from_clock(WallClock::new(-1, 0));
        assert_eq!(strftime("%F %T", &neg), "1969-12-31 23:59:59");
        assert_eq!(strftime("100%% %q", &leap), "100% %q");
    }
}
