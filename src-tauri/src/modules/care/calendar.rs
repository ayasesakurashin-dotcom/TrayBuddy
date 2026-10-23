//! Imported calendar facts and personal dates. No persona, model or scheduler policy.
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub date: NaiveDate,
    pub name: String,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LunarDay {
    pub date: NaiveDate,
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub leap: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dataset {
    pub id: String,
    pub country: String,
    pub year: i32,
    pub source: String,
    pub imported_at: String,
    pub events: Vec<CalendarEvent>,
    pub lunar_days: Vec<LunarDay>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SpecialDate {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub calendar: String,
    pub year: Option<i32>,
    pub month: u32,
    pub day: u32,
    pub yearly: bool,
    pub leap_month: bool,
    pub enabled: bool,
    pub source: String,
}
impl Default for SpecialDate {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: "special".into(),
            calendar: "gregorian".into(),
            year: None,
            month: 1,
            day: 1,
            yearly: true,
            leap_month: false,
            enabled: true,
            source: "manual".into(),
        }
    }
}
impl SpecialDate {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty()
            || self.name.chars().count() > 80
            || !matches!(self.calendar.as_str(), "gregorian" | "lunar")
            || !matches!(
                self.kind.as_str(),
                "birthday" | "anniversary" | "vacation" | "special"
            )
            || self.year.is_some_and(|y| !(1901..=2100).contains(&y))
            || (!self.yearly && self.year.is_none())
            || !(1..=12).contains(&self.month)
            || !(1..=31).contains(&self.day)
            || (self.calendar == "lunar" && self.day > 30)
            || (self.calendar == "gregorian"
                && (self.leap_month
                    || NaiveDate::from_ymd_opt(
                        if self.yearly {
                            2000
                        } else {
                            self.year.unwrap_or(2000)
                        },
                        self.month,
                        self.day,
                    )
                    .is_none()))
        {
            return Err("特殊日期无效，请检查日期、历法与重复方式".into());
        }
        Ok(())
    }
    pub fn occurs(&self, date: NaiveDate, datasets: &[Dataset]) -> bool {
        if !self.enabled {
            return false;
        }
        if self.calendar == "gregorian" {
            self.month == date.month()
                && self.day == date.day()
                && (self.yearly || self.year == Some(date.year()))
        } else {
            // Lunar years can cross Gregorian years; only imported day mappings are authoritative.
            datasets.iter().flat_map(|d| &d.lunar_days).any(|d| {
                d.date == date
                    && (self.yearly || self.year == Some(d.year))
                    && d.month == self.month
                    && d.day == self.day
                    && d.leap == self.leap_month
            })
        }
    }
    pub fn description(&self) -> String {
        format!(
            "{}：{}{}月{}日，{}{}",
            self.name,
            if self.calendar == "lunar" {
                "农历"
            } else {
                "公历"
            },
            self.month,
            self.day,
            if self.yearly {
                "每年重复".into()
            } else {
                format!("{} 年一次", self.year.unwrap_or_default())
            },
            if self.leap_month { "（闰月）" } else { "" }
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Data {
    pub schema_version: u32,
    pub revision: u64,
    pub datasets: Vec<Dataset>,
    pub special_dates: Vec<SpecialDate>,
    pub pending: Option<serde_json::Value>,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 1,
            datasets: vec![],
            special_dates: vec![],
            pending: None,
        }
    }
}
pub struct Book {
    pub data: Data,
    pub error: Option<String>,
    path: PathBuf,
}
impl Book {
    pub fn new(path: PathBuf) -> Self {
        let loaded = (|| {
            if !path.exists() {
                return Ok(Data::default());
            }
            if std::fs::metadata(&path)
                .map_err(|_| "日历文件无法读取")?
                .len()
                > 16 * 1024 * 1024
            {
                return Err("日历文件过大，原文件已保留".to_string());
            }
            let bytes = std::fs::read(&path).map_err(|_| "日历文件无法读取")?;
            serde_json::from_slice::<Data>(&bytes)
                .map_err(|_| "日历文件损坏，原文件已保留".to_string())
        })()
        .and_then(|d| {
            Self::validate(&d)?;
            Ok(d)
        });
        Self {
            error: loaded.as_ref().err().cloned(),
            data: loaded.unwrap_or_default(),
            path,
        }
    }
    fn validate(d: &Data) -> Result<(), String> {
        if d.schema_version != 1 || d.datasets.len() > 100 || d.special_dates.len() > 500 {
            return Err("日历数据格式无效，原文件已保留".into());
        }
        for s in &d.special_dates {
            s.validate()?;
        }
        for set in &d.datasets {
            if set.events.len() > 500
                || set.lunar_days.len() > 366
                || set.events.iter().any(|e| {
                    e.date.year() != set.year
                        || e.name.is_empty()
                        || e.name.chars().count() > 200
                        || !matches!(e.kind.as_str(), "festival" | "public" | "term")
                })
                || set.lunar_days.iter().any(|d| {
                    d.date.year() != set.year
                        || !(1..=12).contains(&d.month)
                        || !(1..=30).contains(&d.day)
                })
            {
                return Err("日历数据格式无效，原文件已保留".into());
            }
        }
        Ok(())
    }
    pub fn edit(
        &mut self,
        update: impl FnOnce(&mut Data) -> Result<(), String>,
    ) -> Result<(), String> {
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        let mut next = self.data.clone();
        update(&mut next)?;
        next.revision = next.revision.saturating_add(1);
        Self::validate(&next)?;
        if serde_json::to_vec_pretty(&next)
            .map_err(|_| "日历无法编码")?
            .len()
            > 16 * 1024 * 1024
        {
            return Err("日历数据过大，请先移除不用的年度或日期".into());
        }
        super::files::save(&self.path, &next)?;
        self.data = next;
        Ok(())
    }
    pub fn save_date(&mut self, mut value: SpecialDate) -> Result<SpecialDate, String> {
        value.validate()?;
        let creating = value.id.is_empty();
        if value.id.is_empty() {
            value.id = crate::modules::companion::settings::random_id()?;
        }
        if value.id.len() > 100 {
            return Err("日期标识无效".into());
        }
        let saved_name = value.name.clone();
        let saved_kind = value.kind.clone();
        self.edit(|d| {
            if !creating
                && d.special_dates
                    .iter()
                    .any(|s| s.id != value.id && s.name == value.name && s.kind == value.kind)
            {
                return Err("已存在同名、同类的日期，请直接修改原条目".into());
            }
            if let Some(old) = d.special_dates.iter_mut().find(|s| {
                s.id == value.id || (creating && s.name == value.name && s.kind == value.kind)
            }) {
                value.id = old.id.clone();
                *old = value;
            } else {
                d.special_dates.push(value);
            }
            d.pending = None;
            Ok(())
        })?;
        self.data
            .special_dates
            .iter()
            .find(|d| d.name == saved_name && d.kind == saved_kind)
            .cloned()
            .ok_or("日期保存未完成".into())
    }
    pub fn view(
        &self,
        date: NaiveDate,
    ) -> crate::modules::environment_service::types::CalendarView {
        let sets = self
            .data
            .datasets
            .iter()
            .filter(|s| s.year == date.year())
            .collect::<Vec<_>>();
        crate::modules::environment_service::types::CalendarView {
            local_date: date.to_string(),
            region: sets
                .iter()
                .map(|s| s.country.clone())
                .collect::<Vec<_>>()
                .join(","),
            dataset_version: format!("imported-{}", self.data.revision),
            coverage: if self.error.is_some() {
                "unavailable"
            } else if sets.is_empty() {
                "not_imported"
            } else {
                "covered"
            }
            .into(),
            holidays: sets
                .iter()
                .flat_map(|s| &s.events)
                .filter(|e| e.date == date && e.kind != "term")
                .map(|e| e.name.clone())
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_calendar_means_no_holidays_and_personal_dates_still_work() {
        let t = tempfile::tempdir().unwrap();
        let mut b = Book::new(t.path().join("calendar.json"));
        let day = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        assert!(b.view(day).holidays.is_empty());
        let s = SpecialDate {
            name: "生日".into(),
            kind: "birthday".into(),
            month: 10,
            day: 25,
            ..Default::default()
        };
        assert!(s.occurs(day, &[]));
        b.save_date(s).unwrap();
        assert_eq!(
            Book::new(t.path().join("calendar.json"))
                .data
                .special_dates
                .len(),
            1
        );
    }
    #[test]
    fn multiple_years_survive_restart_without_the_small_settings_size_limit() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("calendar.json");
        let mut book = Book::new(path.clone());
        let mut sets = Vec::new();
        for year in 2026..2031 {
            let mut set = super::super::import::parse_hko(
                2026,
                include_str!("../../../../docs/fixtures/hko-2026.txt"),
            )
            .unwrap();
            set.year = year;
            set.id = format!("CN-{year}");
            for row in &mut set.events {
                row.date = row.date.with_year(year).unwrap();
            }
            for row in &mut set.lunar_days {
                row.date = row.date.with_year(year).unwrap();
                row.year = year;
            }
            sets.push(set);
        }
        book.edit(|d| {
            d.datasets = sets;
            Ok(())
        })
        .unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 128 * 1024);
        let reloaded = Book::new(path);
        assert!(reloaded.error.is_none());
        assert_eq!(reloaded.data.datasets.len(), 5);
    }
    #[test]
    fn leap_day_and_missing_lunar_coverage_never_guess() {
        let mut s = SpecialDate {
            name: "生日".into(),
            month: 2,
            day: 29,
            ..Default::default()
        };
        assert!(s.validate().is_ok());
        assert!(!s.occurs(NaiveDate::from_ymd_opt(2027, 2, 28).unwrap(), &[]));
        s.calendar = "lunar".into();
        assert!(!s.occurs(NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(), &[]));
    }
}
