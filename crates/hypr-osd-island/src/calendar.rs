//! The calendar month the island popup draws.
//!
//! No date library: `glib` already has one, and the two things a month grid
//! needs - which weekday the first is, and how many days the month has - are
//! both answers `glib::DateTime` gives. Splitting the arithmetic out into a plain
//! struct is what makes it testable without a display.

use gtk::glib;

/// One month, by year and number (1-12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Month {
    pub year: i32,
    pub month: i32,
}

impl Month {
    /// The month we are in, on this machine's clock.
    pub fn today() -> Month {
        let now = glib::DateTime::now_local().unwrap_or_else(|_| fallback_today());
        Month {
            year: now.year(),
            month: now.month(),
        }
    }

    /// `None` for anything that is not a real month, so the constructor cannot
    /// produce a grid with no days in it.
    pub fn new(year: i32, month: i32) -> Option<Month> {
        (1..=12).contains(&month).then_some(Month { year, month })
    }

    /// A month `delta` away, rolling over the year.
    pub fn shift(self, delta: i32) -> Month {
        let zero_based = self.month - 1 + delta;
        // Through `new` so the "is this a month" guard is the only way a `Month`
        // is ever built; the arithmetic cannot land outside 1-12, so the fallback
        // is unreachable - and keeping the old month beats a panic if it is not.
        Month::new(
            self.year + zero_based.div_euclid(12),
            zero_based.rem_euclid(12) + 1,
        )
        .unwrap_or(self)
    }

    /// The heading: `September 2026`, in the system's own language.
    pub fn title(&self) -> String {
        first_of_month(*self)
            .and_then(|date| date.format("%B %Y").ok())
            .map(|text| text.to_string())
            .unwrap_or_else(|| format!("{}-{:02}", self.year, self.month))
    }

    pub fn days(&self) -> i32 {
        days_in_month(self.year, self.month)
    }

    /// The month as whole weeks, Monday first, with `0` for the days before and
    /// after it - every row is seven cells wide.
    ///
    /// Always six rows: a month needs at most six, and a panel that changes
    /// height when you page through it is a panel that jumps under the pointer.
    pub fn weeks(&self) -> Vec<[i32; 7]> {
        let days = self.days();
        // Monday first, so Monday is column 0: glib counts 1 = Monday.
        let offset = first_of_month(*self)
            .map(|date| date.day_of_week() - 1)
            .unwrap_or(0)
            .clamp(0, 6);
        let mut weeks = Vec::with_capacity(6);
        let mut week = [0; 7];
        let mut column = offset as usize;
        for day in 1..=days {
            week[column] = day;
            column += 1;
            if column == 7 {
                weeks.push(week);
                week = [0; 7];
                column = 0;
            }
        }
        if column != 0 {
            weeks.push(week);
        }
        while weeks.len() < 6 {
            weeks.push([0; 7]);
        }
        weeks
    }
}

/// The first day of a month, as a `glib::DateTime`.
fn first_of_month(month: Month) -> Option<glib::DateTime> {
    glib::DateTime::from_local(month.year, month.month, 1, 0, 0, 0.0).ok()
}

/// How many days a month has, by asking for the day before the next one starts.
fn days_in_month(year: i32, month: i32) -> i32 {
    let next = Month { year, month }.shift(1);
    first_of_month(next)
        .and_then(|date| date.add_days(-1).ok())
        .map(|date| date.day_of_month())
        .unwrap_or(30)
}

/// 1970 if even the system clock cannot be asked (which would mean GLib could not
/// be initialised at all - but a calendar that panics is worse than an old one).
fn fallback_today() -> glib::DateTime {
    glib::DateTime::from_local(1970, 1, 1, 0, 0, 0.0).expect("1970 is a valid date for GLib")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_month_rolls_over_the_year() {
        assert_eq!(
            Month::new(2026, 12).unwrap().shift(1),
            Month {
                year: 2027,
                month: 1
            }
        );
        assert_eq!(
            Month::new(2026, 1).unwrap().shift(-1),
            Month {
                year: 2025,
                month: 12
            }
        );
        assert_eq!(
            Month::new(2026, 5).unwrap().shift(12),
            Month {
                year: 2027,
                month: 5
            }
        );
        assert_eq!(
            Month::new(2026, 5).unwrap().shift(-12),
            Month {
                year: 2025,
                month: 5
            }
        );
    }

    #[test]
    fn a_month_that_is_not_a_month_is_refused() {
        assert!(Month::new(2026, 0).is_none());
        assert!(Month::new(2026, 13).is_none());
        assert!(Month::new(2026, 12).is_some());
    }

    #[test]
    fn the_month_lengths_are_right() {
        assert_eq!(days_in_month(2026, 1), 31);
        assert_eq!(days_in_month(2026, 2), 28);
        // 2024 is a leap year, and the day before the 1st of March is the 29th.
        assert_eq!(days_in_month(2024, 2), 29);
        // The 2000-style rules: 1900 is not a leap year, 2000 is.
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2026, 12), 31);
    }

    /// February 2024 started on a Thursday, so a Monday-first grid has three
    /// empty cells in front of it and 29 days to place.
    #[test]
    fn the_grid_starts_on_the_right_weekday() {
        let weeks = Month::new(2024, 2).unwrap().weeks();
        assert_eq!(weeks.len(), 6);
        assert_eq!(weeks[0], [0, 0, 0, 1, 2, 3, 4]);
        assert_eq!(weeks[1][0], 5);
        // 29 days: the last row holds 26, 27, 28, 29 and then nothing.
        assert_eq!(weeks[4], [26, 27, 28, 29, 0, 0, 0]);
        assert_eq!(weeks[5], [0; 7]);
    }

    /// A month that starts on a Monday starts in the first cell - the case a
    /// naive `(weekday - 1) % 7` or a Sunday-first count gets wrong.
    #[test]
    fn a_month_that_starts_on_monday_fills_the_first_cell() {
        // 2026-06-01 is a Monday.
        let weeks = Month::new(2026, 6).unwrap().weeks();
        assert_eq!(weeks[0][0], 1);
        assert_eq!(weeks[0][6], 7);
    }

    #[test]
    fn every_month_of_a_year_is_six_rows_of_seven() {
        for month in 1..=12 {
            let weeks = Month::new(2026, month).unwrap().weeks();
            assert_eq!(weeks.len(), 6, "month {month}");
            for week in &weeks {
                assert_eq!(week.len(), 7);
            }
            let days: i32 = weeks.iter().flatten().filter(|day| **day > 0).sum();
            // The sum of 1..=n is n(n+1)/2, so this checks the whole month is
            // there exactly once - no day dropped, none doubled.
            let expected = days_in_month(2026, month);
            assert_eq!(days, expected * (expected + 1) / 2, "month {month}");
        }
    }
}
