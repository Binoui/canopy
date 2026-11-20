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
        date: format!("{:04}-{:02}-{:02}", t.year(), t.month(), t.day_of_month()),
        ordinal: DateTime::from_utc(t.year(), t.month(), t.day_of_month(), 12, 0, 0.)
            .unwrap()
            .to_unix()
            .div_euclid(86400),
        weekday: t.day_of_week(),
        month: t.month(),
        year: t.year(),
        day: t.day_of_month(),
        clock: format!("{:02}:{:02}", t.hour(), t.minute()),
    }
}
#[derive(Clone, Debug)]
pub(super) struct Civil {
    pub date: String,
    ordinal: i64,
    weekday: i32,
    month: i32,
    year: i32,
    day: i32,
    pub clock: String,
}
impl Civil {
    pub fn label(&self, today: &Self) -> String {
        if self.date >= today.date {
            return "Today".into();
        }
        if self.ordinal == today.ordinal - 1 {
            return "Yesterday".into();
        }
        format!(
            "{} {}",
            WEEKDAYS[(self.weekday - 1) as usize],
            self.short_date(today)
        )
    }
    fn short_date(&self, today: &Self) -> String {
        let month = MONTHS[(self.month - 1) as usize];
        if self.year == today.year {
            format!("{} {month}", self.day)
        } else {
            format!("{} {month} {}", self.day, self.year)
        }
    }
    pub fn timeline(&self, today: &Self) -> (String, String) {
        let (bucket, heading) = self.bucket(today);
        let when = match bucket.as_str() {
            "today" | "yesterday" => self.clock.clone(),
            _ => self.short_date(today),
        };
        (heading, when)
    }
    pub fn bucket(&self, today: &Self) -> (String, String) {
        let monday = today.ordinal - i64::from(today.weekday - 1);
        let (key, label) = if self.ordinal >= today.ordinal {
            ("today", "Today")
        } else if self.ordinal == today.ordinal - 1 {
            ("yesterday", "Yesterday")
        } else if self.ordinal >= monday {
            ("week", "Earlier this week")
        } else if self.ordinal >= monday - 7 {
            ("last-week", "Last week")
        } else {
            const MONTHS: [&str; 12] = [
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ];
            return (
                format!("month-{:04}-{:02}", self.year, self.month),
                format!(
                    "{}{}",
                    MONTHS[(self.month - 1) as usize],
                    if self.year == today.year {
                        String::new()
                    } else {
                        format!(" {}", self.year)
                    }
                ),
            );
        };
        (key.into(), label.into())
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

#[cfg(test)]
mod bucket_tests {
    use super::*;
    fn at(s: &str) -> Civil {
        civil(DateTime::from_iso8601(s, None).unwrap())
    }
    #[test]
    fn monday_month_end_and_clock_changes_partition_days() {
        let monday = at("2026-11-02T00:05:00+01:00");
        assert_eq!(
            at("2026-11-01T23:55:00+01:00").bucket(&monday).1,
            "Yesterday"
        );
        assert_eq!(
            at("2026-10-31T12:00:00+01:00").bucket(&monday).1,
            "Last week"
        );
        assert_eq!(at("2026-10-25T02:30:00+02:00").bucket(&monday).1, "October");
        let wed = at("2026-11-04T08:00:00+01:00");
        assert_eq!(monday.bucket(&wed).1, "Earlier this week");
        assert_eq!(at("2026-11-03T08:00:00+01:00").bucket(&wed).1, "Yesterday");
        let spring = at("2026-03-30T00:05:00+02:00");
        assert_eq!(
            at("2026-03-29T00:10:00+01:00").bucket(&spring).1,
            "Yesterday"
        );
        assert_eq!(at("2025-11-18T08:05:00Z").bucket(&wed).1, "November 2025");
    }
}
