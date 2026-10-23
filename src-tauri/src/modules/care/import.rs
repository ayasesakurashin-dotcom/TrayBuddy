//! Network adapters only run on an explicit calendar import command.
use super::calendar::{CalendarEvent, Dataset, LunarDay};
use chrono::{Datelike, NaiveDate};
use serde_json::Value;

pub async fn download(country: &str, first: i32, last: i32) -> Result<Vec<Dataset>, String> {
    let country = country.trim().to_ascii_uppercase();
    if country.len() != 2
        || !country.bytes().all(|c| c.is_ascii_uppercase())
        || !(1901..=2100).contains(&first)
        || last < first
        || last > 2100
        || last - first > 4
    {
        return Err("请选择两位国家代码及最多连续五年的日历（1901—2100）".into());
    }
    let client = crate::modules::companion::providers::client(false)?;
    let mut sets = Vec::new();
    let mut previous = None;
    let begin = if country == "CN" {
        if first == 1901 {
            return Err("中国农历导入从 1902 年起可核验上一年衔接资料".into());
        }
        first - 1
    } else {
        first
    };
    for year in begin..=last {
        let url = if country == "CN" {
            format!("https://www.hko.gov.hk/en/gts/time/calendar/text/files/T{year}e.txt")
        } else {
            format!("https://nagerholidays.com/api/v4/Holidays/{country}/{year}")
        };
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|_| "日历来源暂时无法连接，未更改本地日历")?;
        if !resp.status().is_success() || resp.content_length().unwrap_or(0) > 1_000_000 {
            return Err("该国家或年份的日历暂不可用，未更改本地日历".into());
        }
        let mut resp = resp;
        let mut bytes = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|_| "日历下载未完成")? {
            if bytes.len() + chunk.len() > 1_000_000 {
                return Err("日历资料过大".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let text = String::from_utf8(bytes).map_err(|_| "日历编码无效")?;
        let mut set = if country == "CN" {
            parse_hko_with_previous(year, &text, previous.as_ref())?
        } else {
            parse_public(&country, year, &text)?
        };
        set.source = url;
        previous = set.lunar_days.last().cloned();
        if year >= first {
            sets.push(set);
        }
    }
    Ok(sets)
}
fn dataset(country: &str, year: i32) -> Dataset {
    Dataset {
        id: format!("{country}-{year}"),
        country: country.into(),
        year,
        source: String::new(),
        imported_at: chrono::Utc::now().to_rfc3339(),
        events: vec![],
        lunar_days: vec![],
    }
}
#[cfg(test)]
pub fn parse_hko(year: i32, text: &str) -> Result<Dataset, String> {
    parse_hko_with_previous(year, text, None)
}
fn parse_hko_with_previous(
    year: i32,
    text: &str,
    previous: Option<&LunarDay>,
) -> Result<Dataset, String> {
    let rows = text
        .lines()
        .filter_map(|line| {
            let cols = line.split_whitespace().collect::<Vec<_>>();
            let date = NaiveDate::parse_from_str(cols.first()?, "%Y/%m/%d").ok()?;
            Some((date, cols))
        })
        .collect::<Vec<_>>();
    let expected = NaiveDate::from_ymd_opt(year + 1, 1, 1)
        .zip(NaiveDate::from_ymd_opt(year, 1, 1))
        .map(|(a, b)| (a - b).num_days() as usize)
        .ok_or("日历年份无效")?;
    if rows.len() != expected
        || rows
            .iter()
            .enumerate()
            .any(|(n, (d, _))| d.year() != year || d.ordinal() as usize != n + 1)
    {
        return Err("公农历对照表不完整，未导入".into());
    }
    let month_re = regex::Regex::new(r"^(\d+)(?:st|nd|rd|th)$").unwrap();
    let first_month = rows
        .iter()
        .find_map(|(_, c)| {
            if c.get(2) == Some(&"Lunar") {
                month_re
                    .captures(c[1])
                    .and_then(|c| c[1].parse::<u32>().ok())
            } else {
                None
            }
        })
        .ok_or("农历月份缺失")?;
    let mut month = if first_month == 1 {
        12
    } else {
        first_month - 1
    };
    if let Some(d) = previous {
        month = d.month;
    }
    let mut leap = previous.is_some_and(|d| d.leap);
    let mut lunar_year = year - 1;
    let terms = [
        ("Minor Cold", "小寒"),
        ("Moderate Cold", "小寒"),
        ("Severe Cold", "大寒"),
        ("Spring Commences", "立春"),
        ("Spring Showers", "雨水"),
        ("Insects Waken", "惊蛰"),
        ("Vernal Equinox", "春分"),
        ("Bright & Clear", "清明"),
        ("Corn Rain", "谷雨"),
        ("Summer Commences", "立夏"),
        ("Corn Forms", "小满"),
        ("Corn on Ear", "芒种"),
        ("Summer Solstice", "夏至"),
        ("Moderate Heat", "小暑"),
        ("Great Heat", "大暑"),
        ("Autumn Commences", "立秋"),
        ("End of Heat", "处暑"),
        ("White Dew", "白露"),
        ("Autumnal Equinox", "秋分"),
        ("Cold Dew", "寒露"),
        ("Frost", "霜降"),
        ("Winter Commences", "立冬"),
        ("Light Snow", "小雪"),
        ("Heavy Snow", "大雪"),
        ("Winter Solstice", "冬至"),
    ];
    let mut set = dataset("CN", year);
    for (date, cols) in rows {
        let day = if cols.get(2) == Some(&"Lunar") {
            let next_month = month_re
                .captures(cols[1])
                .and_then(|c| c[1].parse().ok())
                .ok_or("农历月份无效")?;
            leap =
                next_month == month || cols.iter().any(|c| c.to_ascii_lowercase().contains("leap"));
            month = next_month;
            1
        } else {
            cols.get(1)
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|n| (1..=30).contains(n))
                .ok_or("农历日期无效")?
        };
        if month == 1 && day == 1 && !leap {
            lunar_year = year;
        }
        set.lunar_days.push(LunarDay {
            date,
            year: lunar_year,
            month,
            day,
            leap,
        });
        let name = match (date.month(), date.day()) {
            (1, 1) => Some("元旦"),
            (5, 1) => Some("劳动节"),
            (10, 1) => Some("国庆节"),
            _ => None,
        };
        if let Some(name) = name {
            set.events.push(CalendarEvent {
                date,
                name: name.into(),
                kind: "festival".into(),
            });
        }
        if !leap {
            let festival = match (month, day) {
                (1, 1) => Some("春节"),
                (1, 15) => Some("元宵节"),
                (5, 5) => Some("端午节"),
                (7, 7) => Some("七夕"),
                (8, 15) => Some("中秋节"),
                (9, 9) => Some("重阳节"),
                (12, 8) => Some("腊八节"),
                _ => None,
            };
            if let Some(name) = festival {
                set.events.push(CalendarEvent {
                    date,
                    name: name.into(),
                    kind: "festival".into(),
                });
            }
            if month == 1 && day == 1 {
                if let Some(eve) = date.pred_opt().filter(|d| d.year() == year) {
                    set.events.push(CalendarEvent {
                        date: eve,
                        name: "除夕".into(),
                        kind: "festival".into(),
                    });
                }
            }
        }
        let line = cols.join(" ");
        if let Some((_, term)) = terms.iter().find(|(en, _)| line.ends_with(en)) {
            set.events.push(CalendarEvent {
                date,
                name: (*term).into(),
                kind: if *term == "清明" {
                    "festival"
                } else {
                    "term"
                }
                .into(),
            });
        }
    }
    Ok(set)
}
pub fn parse_public(country: &str, year: i32, text: &str) -> Result<Dataset, String> {
    let rows: Vec<Value> = serde_json::from_str(text).map_err(|_| "公众节日日历格式无效")?;
    if rows.len() > 500 {
        return Err("日历事件过多".into());
    }
    let mut set = dataset(country, year);
    for row in rows {
        if row["countryCode"].as_str() != Some(country) {
            return Err("日历国家不匹配".into());
        }
        // Regional holidays require choosing a region; never apply them to the entire country.
        if row["nationalHoliday"].as_bool() != Some(true)
            || !row["holidayTypes"]
                .as_array()
                .is_some_and(|v| v.iter().any(|t| t == "Public"))
        {
            continue;
        }
        let date = NaiveDate::parse_from_str(row["date"].as_str().unwrap_or(""), "%Y-%m-%d")
            .map_err(|_| "日历日期无效")?;
        let name = row["localName"]
            .as_str()
            .or_else(|| row["name"].as_str())
            .unwrap_or("");
        if date.year() != year || name.is_empty() || name.chars().count() > 200 {
            return Err("日历内容无效".into());
        }
        set.events.push(CalendarEvent {
            date,
            name: name.into(),
            kind: "public".into(),
        });
    }
    if set.events.is_empty() {
        return Err("该来源没有此国家、年份的全国公众节日，请改用其他年份或国家".into());
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imported_years_resolve_real_lunar_festivals_and_all_solar_terms() {
        let a = parse_hko(2026, include_str!("../../../../docs/fixtures/hko-2026.txt")).unwrap();
        let b = parse_hko(2027, include_str!("../../../../docs/fixtures/hko-2027.txt")).unwrap();
        assert!(a
            .events
            .iter()
            .any(|e| e.name == "春节" && e.date.to_string() == "2026-02-17"));
        assert!(b
            .events
            .iter()
            .any(|e| e.name == "春节" && e.date.to_string() == "2027-02-06"));
        assert_eq!(a.lunar_days.len(), 365);
        assert_eq!(a.events.iter().filter(|e| e.kind == "term").count(), 23);
        assert!(a
            .lunar_days
            .iter()
            .any(|d| d.date.to_string() == "2026-01-01" && d.year == 2025));
        let s = super::super::calendar::SpecialDate {
            name: "农历生日".into(),
            calendar: "lunar".into(),
            year: Some(2026),
            month: 1,
            day: 1,
            yearly: false,
            ..Default::default()
        };
        assert!(s.occurs(NaiveDate::from_ymd_opt(2026, 2, 17).unwrap(), &[a, b]));
        let jp = parse_public(
            "JP",
            2026,
            include_str!("../../../../docs/fixtures/nager-jp-2026.json"),
        )
        .unwrap();
        assert!(!jp.events.is_empty());
    }
    #[test]
    fn repeated_lunar_month_header_means_leap_month_and_does_not_repeat_normal_anniversaries() {
        let set = parse_hko(2025, include_str!("../../../../docs/fixtures/hko-2025.txt")).unwrap();
        let ordinary = super::super::calendar::SpecialDate {
            name: "农历生日".into(),
            calendar: "lunar".into(),
            month: 6,
            day: 1,
            ..Default::default()
        };
        assert!(ordinary.occurs(
            NaiveDate::from_ymd_opt(2025, 6, 25).unwrap(),
            &[set.clone()]
        ));
        assert!(!ordinary.occurs(
            NaiveDate::from_ymd_opt(2025, 7, 25).unwrap(),
            &[set.clone()]
        ));
        let leap = super::super::calendar::SpecialDate {
            leap_month: true,
            ..ordinary
        };
        assert!(leap.occurs(NaiveDate::from_ymd_opt(2025, 7, 25).unwrap(), &[set]));
    }
    #[test]
    fn reject_partial_lunar_table_and_wrong_country() {
        assert!(parse_hko(2026, "2026/1/1 13 Thursday").is_err());
        assert!(parse_public("US", 2026, r#"[{"countryCode":"GB"}]"#).is_err());
    }
    #[test]
    fn regional_or_nonpublic_dates_are_not_national_greetings() {
        let text = r#"[{"countryCode":"US","date":"2026-01-01","name":"New Year","nationalHoliday":true,"holidayTypes":["Public"]},{"countryCode":"US","date":"2026-01-02","name":"Local","nationalHoliday":false,"holidayTypes":["Public"]}]"#;
        assert_eq!(parse_public("US", 2026, text).unwrap().events.len(), 1);
    }
}
