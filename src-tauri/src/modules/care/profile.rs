//! Compatibility adapter for known legacy care; ordinary Mod events stay native.
pub fn managed(mod_id: &str, state: &str) -> bool {
    matches!(mod_id, "ema" | "ema2" | "ema3")
        && matches!(state, "rain" | "cold" | "hot" | "snow" | "long_time")
}
pub fn allows_preset(mod_id: &str, state: &crate::modules::resource::StateInfo) -> bool {
    !state
        .care_category
        .as_deref()
        .is_some_and(|category| !category.trim().is_empty())
        && !managed(mod_id, state.name.as_ref())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::resource::StateInfo;
    #[test]
    fn login_greetings_and_ordinary_random_states_are_not_legacy_care() {
        for name in [
            "morning",
            "noon",
            "evening",
            "night",
            "random1_1",
            "random4_2",
            "music_random1_1",
            "every_hour",
            "birthday",
            "firstday",
        ] {
            assert!(
                allows_preset(
                    "ema",
                    &StateInfo {
                        name: name.into(),
                        ..Default::default()
                    }
                ),
                "{name}"
            );
        }
        for name in ["rain", "cold", "hot", "snow", "long_time"] {
            assert!(
                !allows_preset(
                    "ema",
                    &StateInfo {
                        name: name.into(),
                        ..Default::default()
                    }
                ),
                "{name}"
            );
        }
    }
    #[test]
    fn unrelated_mods_only_opt_in_to_takeover_with_an_explicit_care_marker() {
        let mut state = StateInfo {
            name: "rain".into(),
            ..Default::default()
        };
        assert!(allows_preset("other", &state));
        state.care_category = Some("weather".into());
        assert!(!allows_preset("other", &state));
        state.care_category = Some("".into());
        assert!(allows_preset("other", &state));
    }
}
