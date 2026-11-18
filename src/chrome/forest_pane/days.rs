use glib::DateTime;

pub(super) trait Calendar {
    fn now(&self) -> Civil;
    fn civil(&self, ms: i64) -> Civil;
}
pub(super) struct LocalCalendar;
impl Calendar for LocalCalendar {
    fn now(&self) -> Civil {
        civil(DateTime::now_local().unwrap())
    }
    fn civil(&self, ms: i64) -> Civil {
        civil(
            DateTime::from_unix_local(ms.div_euclid(1000))
                .unwrap_or_else(|_| DateTime::from_unix_utc(0).unwrap()),
        )
    }
}
// The chrome is in English. GLib's own names follow LC_TIME once GTK has set the locale.
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
fn civil(t: DateTime) -> Civil {
    Civil {
        date: t.format("%Y-%m-%d").unwrap().to_string(),
        year: t.year(),
        day: t.day_of_year(),
        label: format!(
            "{} {} {}",
            WEEKDAYS[(t.day_of_week() - 1) as usize],
            t.day_of_month(),
            MONTHS[(t.month() - 1) as usize]
        ),
        clock: t.format("%H:%M").unwrap().to_string(),
    }
}
#[derive(Clone, Debug)]
pub(super) struct Civil {
    pub date: String,
    year: i32,
    day: i32,
    label: String,
    pub clock: String,
}
impl Civil {
    pub fn label(&self, today: &Self) -> String {
        if self.date >= today.date {
            return "Today".into();
        }
        let yesterday = DateTime::from_iso8601(&format!("{}T12:00:00Z", today.date), None)
            .unwrap()
            .add_days(-1)
            .unwrap();
        if self.year == yesterday.year() && self.day == yesterday.day_of_year() {
            return "Yesterday".into();
        }
        format!(
            "{}{}",
            self.label,
            if self.year == today.year {
                String::new()
            } else {
                format!(" {}", self.year)
            }
        )
    }
    pub fn when(&self, today: &Self) -> String {
        let label = self.label(today);
        format!(
            "{} at {}",
            if ["Today", "Yesterday"].contains(&label.as_str()) {
                label.to_lowercase()
            } else {
                label
            },
            self.clock
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_day_labels_cross_year_and_dst_without_duration_arithmetic() {
        let at = |s| civil(DateTime::from_iso8601(s, None).unwrap());
        let today = at("2026-01-01T01:00:00+01:00");
        assert_eq!(at("2025-12-31T23:30:00+01:00").label(&today), "Yesterday");
        assert_eq!(
            at("2025-12-30T23:30:00+01:00").label(&today),
            "Tue 30 Dec 2025"
        );
        assert_eq!(
            at("2026-03-29T00:30:00+01:00").label(&at("2026-03-30T00:15:00+02:00")),
            "Yesterday"
        );
        assert_eq!(today.label(&today), "Today");
        assert_eq!(
            at("2026-11-16T08:05:00Z").when(&at("2026-11-18T10:00:00Z")),
            "Mon 16 Nov at 08:05"
        );
    }
}
