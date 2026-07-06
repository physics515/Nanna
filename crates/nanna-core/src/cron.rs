//! Cron expression parser
//!
//! Parses standard cron expressions: `minute hour day month weekday`
//!
//! Supports:
//! - Numbers: `0`, `15`, `30`
//! - Wildcards: `*`
//! - Ranges: `1-5`, `10-20`
//! - Steps: `*/5`, `0-30/10`
//! - Lists: `1,15,30`
//! - Special strings: `@hourly`, `@daily`, `@weekly`, `@monthly`

use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};
use std::collections::HashSet;
use std::str::FromStr;

/// A parsed cron expression
#[derive(Debug, Clone)]
pub struct CronExpr {
    minutes: HashSet<u32>,
    hours: HashSet<u32>,
    days: HashSet<u32>,
    months: HashSet<u32>,
    weekdays: HashSet<u32>, // 0 = Sunday, 6 = Saturday
}

/// Cron parsing error
#[derive(Debug, Clone, thiserror::Error)]
pub enum CronError {
    #[error("Invalid cron expression: {0}")]
    Invalid(String),
    #[error("Invalid field '{field}': {message}")]
    InvalidField { field: String, message: String },
    #[error("Value {value} out of range for {field} (expected {min}-{max})")]
    OutOfRange {
        field: String,
        value: u32,
        min: u32,
        max: u32,
    },
}

impl CronExpr {
    /// Parse a cron expression
    ///
    /// Format: `minute hour day month weekday`
    ///
    /// # Examples
    ///
    /// ```
    /// use nanna_core::cron::CronExpr;
    ///
    /// // Every 5 minutes
    /// let expr = CronExpr::parse("*/5 * * * *").unwrap();
    ///
    /// // 8 AM every day
    /// let expr = CronExpr::parse("0 8 * * *").unwrap();
    ///
    /// // 3 PM on weekdays
    /// let expr = CronExpr::parse("0 15 * * 1-5").unwrap();
    ///
    /// // Special strings
    /// let expr = CronExpr::parse("@hourly").unwrap();
    /// ```
    pub fn parse(expr: &str) -> Result<Self, CronError> {
        let expr = expr.trim();

        // Handle special strings
        match expr {
            "@yearly" | "@annually" => return Self::parse("0 0 1 1 *"),
            "@monthly" => return Self::parse("0 0 1 * *"),
            "@weekly" => return Self::parse("0 0 * * 0"),
            "@daily" | "@midnight" => return Self::parse("0 0 * * *"),
            "@hourly" => return Self::parse("0 * * * *"),
            "@every_minute" => return Self::parse("* * * * *"),
            _ => {}
        }

        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(CronError::Invalid(format!(
                "Expected 5 fields, got {}",
                parts.len()
            )));
        }

        Ok(Self {
            minutes: parse_field(parts[0], "minute", 0, 59)?,
            hours: parse_field(parts[1], "hour", 0, 23)?,
            days: parse_field(parts[2], "day", 1, 31)?,
            months: parse_field(parts[3], "month", 1, 12)?,
            weekdays: parse_field(parts[4], "weekday", 0, 6)?,
        })
    }

    /// Check if the expression matches a specific datetime
    pub fn matches<Tz: TimeZone>(&self, dt: &DateTime<Tz>) -> bool {
        let minute = dt.minute();
        let hour = dt.hour();
        let day = dt.day();
        let month = dt.month();
        let weekday = dt.weekday().num_days_from_sunday();

        self.minutes.contains(&minute)
            && self.hours.contains(&hour)
            && self.days.contains(&day)
            && self.months.contains(&month)
            && self.weekdays.contains(&weekday)
    }

    /// Find the next datetime that matches this expression
    ///
    /// Returns None if no match is found within 4 years (to prevent infinite loops)
    pub fn next<Tz: TimeZone>(&self, from: &DateTime<Tz>) -> Option<DateTime<Tz>>
    where
        Tz::Offset: Copy,
    {
        let mut dt = from.clone() + chrono::Duration::minutes(1);
        // Reset seconds
        dt = dt
            .timezone()
            .with_ymd_and_hms(dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute(), 0)
            .single()?;

        let max_iterations = 4 * 365 * 24 * 60; // ~4 years of minutes

        for _ in 0..max_iterations {
            if self.matches(&dt) {
                return Some(dt);
            }

            // Advance by 1 minute
            dt = dt + chrono::Duration::minutes(1);
        }

        None
    }

    /// Find the next datetime that matches, starting from now (UTC)
    pub fn next_from_now(&self) -> Option<DateTime<Utc>> {
        self.next(&Utc::now())
    }

    /// Get human-readable description
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();

        // Minutes
        if self.minutes.len() == 60 {
            parts.push("every minute".to_string());
        } else if self.minutes.len() == 1 {
            let m = *self.minutes.iter().next().unwrap();
            if m == 0 {
                // Don't mention "at minute 0"
            } else {
                parts.push(format!("at minute {m}"));
            }
        } else {
            // `minutes` is a HashSet (unordered) — sort before differencing so the
            // reported step is deterministic and actually the smallest interval.
            let mut mins: Vec<_> = self.minutes.iter().copied().collect();
            mins.sort_unstable();
            if is_step(&mins) {
                parts.push(format!("every {} minutes", mins[1] - mins[0]));
            }
        }

        // Hours
        if self.hours.len() == 24 {
            if self.minutes.len() != 60 {
                parts.push("every hour".to_string());
            }
        } else if self.hours.len() == 1 {
            let h = *self.hours.iter().next().unwrap();
            let period = if h < 12 { "AM" } else { "PM" };
            let h12 = if h == 0 {
                12
            } else if h > 12 {
                h - 12
            } else {
                h
            };
            parts.push(format!("at {h12} {period}"));
        }

        // Weekdays
        if self.weekdays.len() < 7 {
            let names: Vec<_> = self
                .weekdays
                .iter()
                .map(|&d| match d {
                    0 => "Sun",
                    1 => "Mon",
                    2 => "Tue",
                    3 => "Wed",
                    4 => "Thu",
                    5 => "Fri",
                    6 => "Sat",
                    _ => "?",
                })
                .collect();
            parts.push(format!("on {}", names.join(", ")));
        }

        if parts.is_empty() {
            "every minute".to_string()
        } else {
            parts.join(" ")
        }
    }
}

impl FromStr for CronExpr {
    type Err = CronError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Parse a single cron field
fn parse_field(field: &str, name: &str, min: u32, max: u32) -> Result<HashSet<u32>, CronError> {
    let mut values = HashSet::new();

    for part in field.split(',') {
        let part = part.trim();

        if part == "*" {
            // Wildcard - all values
            for v in min..=max {
                values.insert(v);
            }
        } else if let Some(step_part) = part.strip_prefix("*/") {
            // Step from start: */5 means 0, 5, 10, ...
            let step: u32 = step_part.parse().map_err(|_| CronError::InvalidField {
                field: name.to_string(),
                message: format!("Invalid step: {step_part}"),
            })?;

            if step == 0 {
                return Err(CronError::InvalidField {
                    field: name.to_string(),
                    message: "Step cannot be 0".to_string(),
                });
            }

            let mut v = min;
            while v <= max {
                values.insert(v);
                v += step;
            }
        } else if part.contains('/') {
            // Range with step: 0-30/10
            let parts: Vec<&str> = part.split('/').collect();
            if parts.len() != 2 {
                return Err(CronError::InvalidField {
                    field: name.to_string(),
                    message: format!("Invalid step expression: {part}"),
                });
            }

            let range = parts[0];
            let step: u32 = parts[1].parse().map_err(|_| CronError::InvalidField {
                field: name.to_string(),
                message: format!("Invalid step: {}", parts[1]),
            })?;

            let (start, end) = parse_range(range, name, min, max)?;

            let mut v = start;
            while v <= end {
                values.insert(v);
                v += step;
            }
        } else if part.contains('-') {
            // Range: 1-5
            let (start, end) = parse_range(part, name, min, max)?;
            for v in start..=end {
                values.insert(v);
            }
        } else {
            // Single value
            let v: u32 = part.parse().map_err(|_| CronError::InvalidField {
                field: name.to_string(),
                message: format!("Invalid value: {part}"),
            })?;

            if v < min || v > max {
                return Err(CronError::OutOfRange {
                    field: name.to_string(),
                    value: v,
                    min,
                    max,
                });
            }

            values.insert(v);
        }
    }

    Ok(values)
}

/// Parse a range like "1-5" or "10-20"
fn parse_range(range: &str, name: &str, min: u32, max: u32) -> Result<(u32, u32), CronError> {
    let parts: Vec<&str> = range.split('-').collect();
    if parts.len() != 2 {
        return Err(CronError::InvalidField {
            field: name.to_string(),
            message: format!("Invalid range: {range}"),
        });
    }

    let start: u32 = parts[0].parse().map_err(|_| CronError::InvalidField {
        field: name.to_string(),
        message: format!("Invalid range start: {}", parts[0]),
    })?;

    let end: u32 = parts[1].parse().map_err(|_| CronError::InvalidField {
        field: name.to_string(),
        message: format!("Invalid range end: {}", parts[1]),
    })?;

    if start < min || start > max {
        return Err(CronError::OutOfRange {
            field: name.to_string(),
            value: start,
            min,
            max,
        });
    }

    if end < min || end > max {
        return Err(CronError::OutOfRange {
            field: name.to_string(),
            value: end,
            min,
            max,
        });
    }

    if start > end {
        return Err(CronError::InvalidField {
            field: name.to_string(),
            message: format!("Range start ({start}) greater than end ({end})"),
        });
    }

    Ok((start, end))
}

/// Check if values form a regular step pattern
fn is_step(values: &[u32]) -> bool {
    if values.len() < 2 {
        return false;
    }

    let mut sorted: Vec<_> = values.to_vec();
    sorted.sort_unstable();

    let step = sorted[1] - sorted[0];
    if step == 0 {
        return false;
    }

    for i in 2..sorted.len() {
        if sorted[i] - sorted[i - 1] != step {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_parse_simple() {
        let expr = CronExpr::parse("0 8 * * *").unwrap();
        assert!(expr.minutes.contains(&0));
        assert!(expr.hours.contains(&8));
        assert_eq!(expr.days.len(), 31);
        assert_eq!(expr.months.len(), 12);
        assert_eq!(expr.weekdays.len(), 7);
    }

    #[test]
    fn test_parse_step() {
        let expr = CronExpr::parse("*/15 * * * *").unwrap();
        assert!(expr.minutes.contains(&0));
        assert!(expr.minutes.contains(&15));
        assert!(expr.minutes.contains(&30));
        assert!(expr.minutes.contains(&45));
        assert_eq!(expr.minutes.len(), 4);
    }

    #[test]
    fn test_parse_range() {
        let expr = CronExpr::parse("0 9-17 * * 1-5").unwrap();
        assert!(expr.hours.contains(&9));
        assert!(expr.hours.contains(&17));
        assert!(!expr.hours.contains(&8));
        assert!(expr.weekdays.contains(&1)); // Monday
        assert!(expr.weekdays.contains(&5)); // Friday
        assert!(!expr.weekdays.contains(&0)); // Not Sunday
    }

    #[test]
    fn test_parse_list() {
        let expr = CronExpr::parse("0,15,30,45 * * * *").unwrap();
        assert_eq!(expr.minutes.len(), 4);
        assert!(expr.minutes.contains(&0));
        assert!(expr.minutes.contains(&15));
    }

    #[test]
    fn test_parse_special() {
        let hourly = CronExpr::parse("@hourly").unwrap();
        assert!(hourly.minutes.contains(&0));
        assert_eq!(hourly.minutes.len(), 1);
        assert_eq!(hourly.hours.len(), 24);

        let daily = CronExpr::parse("@daily").unwrap();
        assert!(daily.minutes.contains(&0));
        assert!(daily.hours.contains(&0));
        assert_eq!(daily.hours.len(), 1);
    }

    #[test]
    fn test_matches() {
        let expr = CronExpr::parse("30 14 * * *").unwrap();
        let dt = Utc.with_ymd_and_hms(2024, 6, 15, 14, 30, 0).unwrap();
        assert!(expr.matches(&dt));

        let dt2 = Utc.with_ymd_and_hms(2024, 6, 15, 14, 31, 0).unwrap();
        assert!(!expr.matches(&dt2));
    }

    #[test]
    fn test_next() {
        let expr = CronExpr::parse("0 * * * *").unwrap(); // Every hour on the hour
        let from = Utc.with_ymd_and_hms(2024, 6, 15, 14, 30, 0).unwrap();
        let next = expr.next(&from).unwrap();
        assert_eq!(next.hour(), 15);
        assert_eq!(next.minute(), 0);
    }

    #[test]
    fn test_describe() {
        let expr = CronExpr::parse("0 8 * * *").unwrap();
        assert!(expr.describe().contains("8 AM"));

        let expr2 = CronExpr::parse("*/5 * * * *").unwrap();
        assert!(expr2.describe().contains("5 minutes"));
    }
}
