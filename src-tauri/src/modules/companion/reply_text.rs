//! Internal context labels are bookkeeping, never spoken or displayed reply text.
use std::sync::OnceLock;

pub fn without_internal_labels(text: &str, done: bool) -> String {
    static LABEL: OnceLock<regex::Regex> = OnceLock::new();
    let pattern=LABEL.get_or_init(||regex::Regex::new(r"(?i)\[(?:(?:实际发言|历史原文)\s*ID|源\s*ID)\s*=\s*[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}[^\]\r\n]*\](?:\r?\n)?").unwrap());
    let mut clean = pattern.replace_all(text, "").into_owned();
    // The marker itself can be split at any UTF-8 boundary across SSE packets.
    // Withhold a possible label until it is disambiguated or its closing bracket
    // arrives, so removing the complete label never retracts displayed text.
    if let Some(open) = clean.rfind('[') {
        let suffix = &clean[open + 1..];
        if !suffix.contains(']') {
            let compact = suffix
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .to_ascii_lowercase();
            if ["实际发言id=", "历史原文id=", "源id="]
                .iter()
                .any(|prefix| {
                    compact.starts_with(prefix) || (!done && prefix.starts_with(compact.as_str()))
                })
            {
                clean.truncate(open);
            }
        }
    }
    clean
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "f5ad2564-42a8-4858-befe-56d3060ce1e4";
    #[test]
    fn drops_only_internal_uuid_labels_and_preserves_answer_and_user_identifiers() {
        let text = format!(
            "[实际发言 ID={ID}]\n先给你一个结论。\n[历史原文 ID={ID}，2026-10-23]\n然后说明依据。"
        );
        assert_eq!(
            without_internal_labels(&text, true),
            "先给你一个结论。\n然后说明依据。"
        );
        let ordinary = format!(
            "请求 ID={ID}；[文档](https://example.com)，[1, 2]；实际发言的意思是已说出的内容。"
        );
        assert_eq!(without_internal_labels(&ordinary, true), ordinary);
        assert_eq!(
            without_internal_labels(&format!("正文[实际发言 ID={ID}"), true),
            "正文"
        );
    }
    #[test]
    fn split_internal_labels_never_flash_or_retract_the_visible_prefix() {
        let raw=format!("[实际发言 ID={ID}；按用户字符上限结束]\n查询资料显示，尚无明确日期。[历史原文 ID={ID}]后续需以官方公告为准。");
        for raw in [
            raw.clone(),
            raw.replace(" ID=", " id="),
            raw.replace("实际发言 ID=", "源 ID="),
        ] {
            let mut previous = String::new();
            for (end, _) in raw.char_indices().chain(std::iter::once((raw.len(), ' '))) {
                let clean = without_internal_labels(&raw[..end], false);
                assert!(clean.starts_with(&previous), "{previous:?} -> {clean:?}");
                assert!(!clean.contains(ID));
                assert!(!clean.contains("实际发言"));
                previous = clean;
            }
            assert_eq!(
                previous,
                "查询资料显示，尚无明确日期。后续需以官方公告为准。"
            );
        }
    }
}
