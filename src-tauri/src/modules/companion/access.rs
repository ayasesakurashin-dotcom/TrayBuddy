use std::sync::Mutex;

/// Window getters can synchronously wait for the UI thread. That thread also
/// reads shared storage while following the pet, so the storage guard must be
/// gone before invoking any window callback (including automatic observations).
pub(super) fn allowed<T>(
    storage: &Mutex<T>,
    automatic: bool,
    preferences_allow: impl FnOnce(&T) -> bool,
    window_visible: impl FnOnce() -> bool,
) -> bool {
    let enabled = {
        let data = storage.lock().unwrap();
        preferences_allow(&data)
    };
    enabled && (!automatic || window_visible())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{mpsc, Arc},
        thread,
        time::Duration,
    };

    #[test]
    fn automatic_window_query_allows_queued_ui_work_to_read_storage() {
        let storage = Arc::new(Mutex::new(true));
        let worker_storage = storage.clone();
        let (query_tx, query_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            // Exercise multiple timer ticks rather than only a one-off query.
            for _ in 0..20 {
                assert!(allowed(
                    &worker_storage,
                    true,
                    |enabled| *enabled,
                    || {
                        let (reply_tx, reply_rx) = mpsc::channel();
                        query_tx.send(reply_tx).unwrap();
                        reply_rx
                            .recv_timeout(Duration::from_secs(2))
                            .unwrap_or(false)
                    }
                ));
            }
        });
        for _ in 0..20 {
            let reply = query_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            // This models the input-window sync queued before IsVisible. Under
            // the old implementation the worker holds this lock while waiting
            // for our reply; try_lock makes the regression fail without hanging.
            let enabled = storage
                .try_lock()
                .expect("UI blocked by observer's storage lock");
            let visible = *enabled;
            drop(enabled);
            reply.send(visible).unwrap();
        }
        worker.join().unwrap();
    }

    #[test]
    fn disabled_preferences_never_query_the_window() {
        assert!(!allowed(
            &Mutex::new(false),
            true,
            |value| *value,
            || { panic!("disabled observations must not query a window") }
        ));
        assert!(!allowed(&Mutex::new(true), true, |value| *value, || false));
    }

    #[test]
    fn input_visibility_policy_is_preserved_for_regular_chat() {
        assert!(allowed(
            &Mutex::new(true),
            false,
            |value| *value,
            || { panic!("regular chat does not need an observation visibility query") }
        ));
    }
}
