use std::{fs, io::Write, path::Path, sync::Mutex};

static LOG_LOCK: Mutex<()> = Mutex::new(());
const MAX_BYTES: u64 = 256 * 1024;

/// Only static stage names and numeric/boolean fields are accepted. Never log
/// prompts, window titles, replies, screenshots, credentials or bank identities.
pub(super) fn record(
    dir: &Path,
    event: &'static str,
    request: Option<u64>,
    success: Option<bool>,
    elapsed_ms: Option<u128>,
) {
    if dir.as_os_str().is_empty() {
        return;
    }
    let _guard = LOG_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let path = dir.join("diagnostics.log");
    if fs::metadata(&path)
        .map(|m| m.len() >= MAX_BYTES)
        .unwrap_or(false)
    {
        let previous = dir.join("diagnostics.previous.log");
        let _ = fs::remove_file(&previous);
        let _ = fs::rename(&path, previous);
    }
    let line = serde_json::json!({
        "time": chrono::Utc::now().to_rfc3339(),
        "pid": std::process::id(),
        "event": event,
        "request": request,
        "success": success,
        "elapsed_ms": elapsed_ms,
    });
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_stages_are_bounded_and_contain_no_conversation_fields() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("diagnostics.log"),
            vec![b' '; MAX_BYTES as usize],
        )
        .unwrap();
        record(dir.path(), "automatic_due", None, None, None);
        assert!(dir.path().join("diagnostics.previous.log").exists());
        let line = fs::read_to_string(dir.path().join("diagnostics.log")).unwrap();
        let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["event"], "automatic_due");
        assert_eq!(value.as_object().unwrap().len(), 6);
        assert!(line.len() < 1024);
    }
}
