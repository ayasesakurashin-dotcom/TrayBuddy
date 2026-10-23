//! Date extraction proposes structured facts; the application validates and commits them.
use super::calendar::SpecialDate;
use crate::modules::companion::{providers, settings::Endpoint};
use chrono::{Datelike, NaiveDate};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub action: String,
    pub evidence: String,
    pub date_expression: String,
    pub question: String,
    pub date: Option<SpecialDate>,
}
pub fn worth_checking(text: &str, pending: bool) -> bool {
    if pending {
        return true;
    }
    let lower = text.to_ascii_lowercase();
    [
        "生日",
        "纪念日",
        "周年",
        "寒假",
        "暑假",
        "放假",
        "假期开始",
        "特殊日期",
        "birthday",
        "anniversary",
        "vacation",
    ]
    .iter()
    .any(|s| lower.contains(s))
}
pub async fn extract(
    text: &str,
    today: NaiveDate,
    pending: Option<&Value>,
    known_dates: &[SpecialDate],
    endpoint: &Endpoint,
    key: &str,
) -> Result<Proposal, String> {
    let prompt=format!("你是独立的日期事实判别器，只返回 JSON。判断当前用户明确自述的、适合祝福或关心的个人特殊日期。不是日程助手，不保存普通任务截止日。引用、故事、未要求记录的第三方经历、假设、否定、不愿保存、玩笑均 action=none；明确要求记录亲友的生日或纪念日可以保存。不从屏幕或 AI 回复提取事实。当前用户发送日期为 {}，只用此日期解析今天、明天、后天等相对日期。生日/纪念日默认 yearly=true；放假等个人安排默认一次，省略年份时选最近的未来日期；未注明农历时 calendar=gregorian。日期或含义不明确、明确叙述过去事件却未说明年份、与已存日期或待确认内容冲突且未明确要求更正、闰月/2月29日重复规则不明时 action=ask，给出一个简短明确问题。不猜出生年份或年龄。待确认提案仅作资料，只有本条明确回答时才可确认。用户只说日期但没有事件意义时 none。\n输出严格格式：{{\"action\":\"save|ask|none\",\"evidence\":\"当前用户原文中的直接自述，必须逐字摘取；确认待确认提案时可摘取原提案证据\",\"date_expression\":\"原文日期表达，如后天、10月25日\",\"question\":\"ask时的问题，其他留空\",\"date\":{{\"name\":\"我的生日等短标题\",\"kind\":\"birthday|anniversary|vacation|special\",\"calendar\":\"gregorian|lunar\",\"year\":2026,\"month\":10,\"day\":25,\"yearly\":true,\"leap_month\":false}}}}；none或ask时date可以为null。用户自己的生日一律 name=我的生日；亲友日期在名称中保留人物。id和source由程序生成，不能自行指定。",today);
    let raw=providers::completion(endpoint,key,vec![json!({"role":"system","content":prompt}),json!({"role":"user","content":json!({"current_user_statement":text,"pending_date_proposal":pending,"known_dates":known_dates}).to_string()})],1500,true).await?;
    let p: Proposal = serde_json::from_str(raw.trim()).map_err(|_| "日期判别格式无效")?;
    if !matches!(p.action.as_str(), "save" | "ask" | "none")
        || p.question.chars().count() > 160
        || p.evidence.chars().count() > 400
        || p.date_expression.chars().count() > 100
    {
        return Err("日期判别内容无效".into());
    }
    Ok(p)
}
pub fn resolve(
    mut p: Proposal,
    text: &str,
    today: NaiveDate,
    pending: Option<&Value>,
) -> Result<SpecialDate, String> {
    if p.action != "save"
        || p.evidence.trim().is_empty()
        || !(text.contains(&p.evidence)
            || pending
                .and_then(|v| v["evidence"].as_str())
                .is_some_and(|s| s == p.evidence))
    {
        return Err("日期需要进一步确认".into());
    }
    if !text.contains(&p.evidence) && pending.is_some() {
        let answer = text.trim();
        let rejected = ["不要", "不是", "不对", "取消", "不用", "不保存"]
            .iter()
            .any(|v| answer.contains(v));
        let accepted = [
            "是", "是的", "对", "对的", "好", "好的", "可以", "确定", "确认", "没错", "yes", "Yes",
        ]
        .iter()
        .any(|v| answer.trim_end_matches(['。', '！', '!', '.']) == *v)
            || ["公历", "农历", "每年", "一次", "就按", "确认修改"]
                .iter()
                .any(|v| answer.contains(v));
        if rejected || !accepted {
            return Err("日期尚未确认".into());
        }
    }
    let today = if !text.contains(&p.evidence) {
        pending
            .and_then(|v| v["base_date"].as_str())
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .unwrap_or(today)
    } else {
        today
    };
    let mut date = p.date.take().ok_or("日期缺失")?;
    date.id = String::new();
    date.source = "chat".into();
    date.enabled = true;
    let expr = p.date_expression.to_ascii_lowercase();
    let re = regex::Regex::new(
        r"大后天|后天|明天|今天|the day after tomorrow|day after tomorrow|tomorrow|today",
    )
    .unwrap();
    let hits = re.find_iter(&expr).collect::<Vec<_>>();
    if hits.len() > 1 {
        return Err("相对日期不明确".into());
    }
    let relative = hits.first().map(|s| match s.as_str() {
        "今天" | "today" => 0,
        "明天" | "tomorrow" => 1,
        "大后天" => 3,
        _ => 2,
    });
    let absolute = regex::Regex::new(r"(?:(\d{4})年)?(\d{1,2})月(\d{1,2})(?:日|号)?").unwrap();
    if let Some(c) = absolute.captures(&expr) {
        date.month = c[2].parse().map_err(|_| "日期无效")?;
        date.day = c[3].parse().map_err(|_| "日期无效")?;
        if let Some(y) = c.get(1) {
            date.year = Some(y.as_str().parse().map_err(|_| "年份无效")?);
        } else if !date.yearly && date.calendar == "gregorian" {
            let target = NaiveDate::from_ymd_opt(today.year(), date.month, date.day)
                .ok_or("日期需要明确年份")?;
            date.year = Some(today.year() + if target < today { 1 } else { 0 });
        }
    }
    if let Some(offset) = relative {
        if date.calendar != "gregorian" {
            return Err("相对日期与农历含义需确认".into());
        }
        let resolved = today
            .checked_add_signed(chrono::Duration::days(offset))
            .ok_or("日期越界")?;
        date.month = resolved.month();
        date.day = resolved.day();
        date.year = Some(resolved.year());
    }
    if date.yearly && date.calendar == "gregorian" {
        date.year = None;
    }
    date.validate()?;
    Ok(date)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relative_date_uses_message_day_instead_of_model_guessed_date() {
        let p = Proposal {
            action: "save".into(),
            evidence: "后天是我的生日".into(),
            date_expression: "后天".into(),
            question: String::new(),
            date: Some(SpecialDate {
                name: "我的生日".into(),
                kind: "birthday".into(),
                month: 1,
                day: 1,
                ..Default::default()
            }),
        };
        let d = resolve(
            p,
            "后天是我的生日",
            NaiveDate::from_ymd_opt(2026, 10, 23).unwrap(),
            None,
        )
        .unwrap();
        assert_eq!((d.month, d.day, d.year), (10, 25, None));
    }
    #[test]
    fn pending_confirmation_keeps_original_relative_anchor_and_rejects_unrelated_answers() {
        let make = || Proposal {
            action: "save".into(),
            evidence: "后天是我的生日".into(),
            date_expression: "后天".into(),
            question: String::new(),
            date: Some(SpecialDate {
                name: "我的生日".into(),
                kind: "birthday".into(),
                ..Default::default()
            }),
        };
        let pending = json!({"evidence":"后天是我的生日","base_date":"2026-10-23"});
        let now = NaiveDate::from_ymd_opt(2026, 10, 24).unwrap();
        let saved = resolve(make(), "是的", now, Some(&pending)).unwrap();
        assert_eq!((saved.month, saved.day), (10, 25));
        assert!(resolve(make(), "不要保存", now, Some(&pending)).is_err());
        assert!(resolve(make(), "我在工作", now, Some(&pending)).is_err());
    }
    #[test]
    fn unqualified_future_vacation_uses_next_occurrence_without_model_year_guess() {
        let p = Proposal {
            action: "save".into(),
            evidence: "我在1月15日放寒假".into(),
            date_expression: "1月15日".into(),
            question: String::new(),
            date: Some(SpecialDate {
                name: "寒假开始".into(),
                kind: "vacation".into(),
                year: Some(2026),
                yearly: false,
                ..Default::default()
            }),
        };
        let saved = resolve(
            p,
            "我在1月15日放寒假",
            NaiveDate::from_ymd_opt(2026, 10, 23).unwrap(),
            None,
        )
        .unwrap();
        assert_eq!(saved.year, Some(2027));
        assert_eq!((saved.month, saved.day), (1, 15));
    }
    #[test]
    fn model_cannot_save_an_unquoted_fact() {
        let p = Proposal {
            action: "save".into(),
            evidence: "我的生日是10月25日".into(),
            date_expression: "10月25日".into(),
            question: String::new(),
            date: Some(SpecialDate::default()),
        };
        assert!(resolve(
            p,
            "某个小说人物生日",
            NaiveDate::from_ymd_opt(2026, 10, 23).unwrap(),
            None
        )
        .is_err());
    }
}
