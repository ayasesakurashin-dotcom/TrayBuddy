//! Verify the shipped executable's own embedded frontend without starting the app.

pub(crate) fn verify(
    context: &tauri::Context<tauri::Wry>,
    report_path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let assets = context.assets();
    let mut errors = Vec::<String>::new();
    if tauri::is_dev() {
        errors.push("Executable is using the development server".into());
    }
    if !matches!(
        context.config().build.frontend_dist,
        Some(tauri::utils::config::FrontendDist::Directory(_))
    ) {
        errors.push("Frontend must be embedded from a local directory".into());
    }
    let required_pages = ["animation.html", "companion_input.html", "settings.html"];
    let links = regex::Regex::new(r#"(?:href|src)="\./([^"?#]+)""#)?;
    let mut checked_links = std::collections::BTreeSet::new();
    for page in required_pages {
        let Some(html) = assets.get(&page.into()) else {
            errors.push(format!("Missing embedded page: {page}"));
            continue;
        };
        let html = std::str::from_utf8(&html)?;
        for link in links.captures_iter(html) {
            let path = link[1].to_string();
            if checked_links.insert(path.clone()) && assets.get(&path.clone().into()).is_none() {
                errors.push(format!("Missing embedded page dependency: {path}"));
            }
        }
    }
    if !checked_links.iter().any(|path| path.ends_with(".js")) {
        errors.push("No embedded frontend JavaScript found".into());
    }
    let report = serde_json::json!({
        "ok": errors.is_empty(),
        "production_mode": !tauri::is_dev(),
        "embedded_assets": assets.iter().count(),
        "required_pages": required_pages,
        "checked_page_dependencies": checked_links.len(),
        "errors": errors,
    });
    std::fs::write(report_path, serde_json::to_vec_pretty(&report)?)?;
    if !errors.is_empty() {
        return Err(errors.join("; ").into());
    }
    Ok(())
}
