use chrono::{DateTime, Local, NaiveDate};

use crate::svn::LogEntry;

#[derive(Debug, Clone, Default)]
pub struct FilterSpec {
    pub keyword: String,
    pub author: String,
    pub rev_min: Option<i64>,
    pub rev_max: Option<i64>,
    pub date_min: Option<NaiveDate>,
    pub date_max: Option<NaiveDate>,
    pub limit: usize,
}

impl FilterSpec {
    pub fn matches(&self, e: &LogEntry) -> bool {
        if let Some(mn) = self.rev_min {
            if e.revision < mn {
                return false;
            }
        }
        if let Some(mx) = self.rev_max {
            if e.revision > mx {
                return false;
            }
        }
        if !self.author.is_empty() && !e.author.contains(&self.author) {
            return false;
        }
        if let Some(d) = naive_date(&e.rfc3339) {
            if let Some(mn) = self.date_min {
                if d < mn {
                    return false;
                }
            }
            if let Some(mx) = self.date_max {
                if d > mx {
                    return false;
                }
            }
        }
        if !self.keyword.is_empty() {
            let kw = self.keyword.as_str();
            let hit = e.msg.contains(kw) || e.changes.iter().any(|c| c.path.contains(kw));
            if !hit {
                return false;
            }
        }
        true
    }
}

pub fn parse_rev(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        t.parse().ok()
    }
}

pub fn parse_date(s: &str) -> Option<NaiveDate> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        NaiveDate::parse_from_str(t, "%Y-%m-%d").ok()
    }
}

fn naive_date(rfc3339: &str) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(rfc3339)
        .ok()
        .map(|d| d.with_timezone(&Local).date_naive())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::svn::{Change, LogEntry};

    fn entry(rev: i64, author: &str, rfc: &str, msg: &str, paths: &[&str]) -> LogEntry {
        LogEntry {
            revision: rev,
            author: author.into(),
            date: String::new(),
            rfc3339: rfc.into(),
            msg: msg.into(),
            changes: paths
                .iter()
                .map(|p| Change {
                    action: 'M',
                    kind: "file".into(),
                    path: p.to_string(),
                })
                .collect(),
        }
    }

    fn spec() -> FilterSpec {
        FilterSpec::default()
    }

    #[test]
    fn default_matches_everything() {
        let e = entry(10, "a", "2024-01-01T00:00:00Z", "x", &["/t/f.rs"]);
        assert!(spec().matches(&e));
    }

    #[test]
    fn revision_range() {
        let e = entry(10, "a", "2024-01-01T00:00:00Z", "x", &[]);
        let mut s = spec();
        s.rev_min = Some(11);
        assert!(!s.matches(&e));
        s.rev_min = Some(10);
        s.rev_max = Some(10);
        assert!(s.matches(&e));
        s.rev_max = Some(9);
        assert!(!s.matches(&e));
    }

    #[test]
    fn author_substring() {
        let e = entry(1, "alice", "2024-01-01T00:00:00Z", "x", &[]);
        let mut s = spec();
        s.author = "lic".into();
        assert!(s.matches(&e));
        s.author = "nope".into();
        assert!(!s.matches(&e));
    }

    #[test]
    fn date_range() {
        let e = entry(1, "a", "2024-06-01T00:00:00Z", "x", &[]);
        let mut s = spec();
        s.date_min = Some(NaiveDate::from_ymd_opt(2024, 6, 2).unwrap());
        assert!(!s.matches(&e));
        s.date_min = Some(NaiveDate::from_ymd_opt(2024, 6, 1).unwrap());
        s.date_max = Some(NaiveDate::from_ymd_opt(2024, 6, 1).unwrap());
        assert!(s.matches(&e));
    }

    #[test]
    fn keyword_matches_msg_or_path() {
        let e = entry(1, "a", "2024-01-01T00:00:00Z", "fix crash", &["/trunk/src/app.rs"]);
        let mut s = spec();
        s.keyword = "crash".into();
        assert!(s.matches(&e));
        s.keyword = "app.rs".into();
        assert!(s.matches(&e));
        s.keyword = "zzz".into();
        assert!(!s.matches(&e));
    }

    #[test]
    fn parse_rev_helpers() {
        assert_eq!(parse_rev(""), None);
        assert_eq!(parse_rev("  42 "), Some(42));
        assert_eq!(parse_rev("abc"), None);
    }

    #[test]
    fn parse_date_helpers() {
        assert_eq!(parse_date(""), None);
        assert_eq!(parse_date("2024-03-01"), Some(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap()));
        assert_eq!(parse_date("2024/03/01"), None);
        assert_eq!(parse_date("not-a-date"), None);
    }

    #[test]
    fn unparsable_date_skips_filter() {
        let e = entry(1, "a", "garbage", "x", &[]);
        let mut s = spec();
        s.date_min = Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
        // 日期无法解析时不应误伤
        assert!(s.matches(&e));
    }
}