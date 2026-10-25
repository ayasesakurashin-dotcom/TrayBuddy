//! Bundled character data. Capabilities depend on RuntimeProfile, never on a preset.
pub struct Preset {
    pub display_name: &'static str,
    pub persona: &'static str,
    pub voice_locale: &'static str,
    pub legacy_voice: bool,
}

pub fn for_mod(id: &str) -> Option<&'static Preset> {
    match id {
        "ema" => Some(&EMA),
        _ => None,
    }
}

static EMA: Preset = Preset {
    display_name: "艾玛",
    persona: include_str!("persona.txt"),
    voice_locale: "ja",
    legacy_voice: true,
};
