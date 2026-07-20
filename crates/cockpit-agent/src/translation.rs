//! On-demand, non-real-time translation hook.
//!
//! Simulation text (a human's `utterance` and `narrative`) is generated once,
//! in the scenario's configured `language` — never in two languages at once,
//! to keep backend cost bounded. Translation may be applied while that text is
//! transient; raw backend prose is redacted before durable simulation or
//! recording state is produced.
//!
//! This module defines the seam for that translation. The default
//! [`IdentityTranslator`] returns text unchanged (a no-op used when no
//! translation service is configured, so the pipeline stays offline and
//! deterministic). A real deployment can plug in a [`Translator`] backed by a
//! model or translation service before the redaction boundary.

/// Translates generated simulation text into a target language on demand.
/// Implementations must be side-effect free with respect to the simulation and
/// must not persist the supplied transient text.
pub trait Translator {
    /// Translate `text` into `target_language` ("en"/"zh"). `source_language`
    /// is the language the text was generated in (the scenario's `language`).
    /// Returning `text` unchanged is always acceptable (e.g. when source and
    /// target match, or no translation is available).
    fn translate(&self, text: &str, source_language: &str, target_language: &str) -> String;
}

/// No-op translator: returns text unchanged. Used when no translation service
/// is configured, keeping the pipeline offline and deterministic.
#[derive(Debug, Default, Clone, Copy)]
pub struct IdentityTranslator;

impl Translator for IdentityTranslator {
    fn translate(&self, text: &str, _source_language: &str, _target_language: &str) -> String {
        text.to_string()
    }
}

/// Normalize a language tag to the coarse buckets the simulation distinguishes.
/// Unknown tags pass through unchanged so callers can still branch on them.
pub fn normalize_language(tag: &str) -> &str {
    match tag {
        "zh" | "zh-CN" | "zh-Hans" => "zh",
        "en" | "en-US" | "en-GB" => "en",
        other => other,
    }
}

/// Whether two language tags refer to the same coarse language, so a caller can
/// skip a translation round-trip when source and target already match.
pub fn same_language(a: &str, b: &str) -> bool {
    normalize_language(a) == normalize_language(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_translator_returns_text_unchanged() {
        let translator = IdentityTranslator;
        assert_eq!(translator.translate("hello", "en", "zh"), "hello");
    }

    #[test]
    fn language_normalization_buckets_variants() {
        assert_eq!(normalize_language("zh-CN"), "zh");
        assert_eq!(normalize_language("en-US"), "en");
        assert_eq!(normalize_language("fr"), "fr");
    }

    #[test]
    fn same_language_ignores_region_variants() {
        assert!(same_language("zh", "zh-CN"));
        assert!(same_language("en-US", "en"));
        assert!(!same_language("en", "zh"));
    }
}
