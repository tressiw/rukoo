//! How a failed dictation is reported: a short notice on the pill and the full text in a tray
//! notification (ownkey-providers.md 7.3, voice/ui/API.md).

use voice_core::{ErrorKind, ProviderError};
use voice_input::InputError;

pub const NO_KEY: &str = "pill.error.noKey";
pub const AUTH: &str = "pill.error.auth";
pub const OFFLINE: &str = "pill.error.offline";
pub const TIMEOUT: &str = "pill.error.timeout";
pub const MIC: &str = "pill.error.mic";
pub const MODEL: &str = "pill.error.model";
pub const MODEL_MISSING: &str = "pill.error.modelMissing";
pub const BLOCKED: &str = "pill.error.blocked";
pub const ACCESSIBILITY: &str = "pill.error.accessibility";
pub const SELECT_TEXT: &str = "pill.error.selectText";
pub const SELECTION_TOO_LONG: &str = "pill.error.selectionTooLong";
pub const GENERIC: &str = "pill.error.generic";

/// Notification titles. The app looks them up in voice/ui/i18n.
pub const TITLE_DICTATION: &str = "app.notify.dictationFailed";
pub const TITLE_REWRITE: &str = "app.notify.rewriteFailed";
pub const TITLE_NO_TEXT_BOX: &str = "app.notify.noTextBox";

/// voice-core reports a missing key as `auth` with this opening; the pill says "Add an API key"
/// for it, not "API key refused".
fn is_missing_key(error: &ProviderError) -> bool {
    error.message.starts_with("No ") && error.message.contains("API key set")
}

pub fn provider_notice(error: &ProviderError) -> &'static str {
    match error.kind {
        ErrorKind::Auth if is_missing_key(error) => NO_KEY,
        ErrorKind::Auth => AUTH,
        ErrorKind::Network => OFFLINE,
        ErrorKind::Timeout => TIMEOUT,
        ErrorKind::Config if is_missing_key(error) => NO_KEY,
        ErrorKind::LocalMissingModel if error.message.starts_with("Could not load") => MODEL,
        ErrorKind::LocalMissingModel => MODEL_MISSING,
        ErrorKind::Provider if error.message.starts_with("Could not load Orukeet") => MODEL,
        ErrorKind::Config | ErrorKind::Provider => GENERIC,
    }
}

/// Which step failed, for the notification's wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Transcribe,
    Rewrite,
}

/// The notification's body. Network and timeout messages from voice-core already say what to
/// do; everything else gets the step in front, so a rewrite failure no longer reads
/// "Transcription failed" as it did in Ownkey (ownkey-dictation.md 17.9).
pub fn provider_body(error: &ProviderError, step: Step) -> String {
    match error.kind {
        ErrorKind::Network | ErrorKind::Timeout => error.message.clone(),
        _ if is_missing_key(error) => error.message.clone(),
        _ => match step {
            Step::Transcribe => format!("Transcription failed: {}", error.message),
            Step::Rewrite => format!("Rewrite failed: {}", error.message),
        },
    }
}

pub fn input_notice(error: &InputError) -> &'static str {
    match error {
        InputError::Blocked => BLOCKED,
        // macOS's Accessibility grant is the one input failure that has its own switch to
        // turn on; the notification's text names the pane.
        InputError::Accessibility => ACCESSIBILITY,
        _ => GENERIC,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(kind: ErrorKind, message: &str) -> ProviderError {
        ProviderError::new(kind, message)
    }

    #[test]
    fn kinds_map_to_the_pill_notices_in_api_md() {
        let cases = [
            (
                error(ErrorKind::Auth, "API error 401: Invalid API key"),
                AUTH,
            ),
            (
                error(
                    ErrorKind::Auth,
                    "No audio API key set. Open Settings > Transcription and add a key.",
                ),
                NO_KEY,
            ),
            (
                error(ErrorKind::Network, "Couldn't reach api.mistral.ai."),
                OFFLINE,
            ),
            (
                error(
                    ErrorKind::Timeout,
                    "Mistral didn't answer within 30 seconds.",
                ),
                TIMEOUT,
            ),
            (
                error(
                    ErrorKind::LocalMissingModel,
                    "Model missing. Download it in Settings > Transcription.",
                ),
                MODEL_MISSING,
            ),
            (
                error(
                    ErrorKind::LocalMissingModel,
                    "Could not load Orukeet: bad. Try again in Settings > Transcription.",
                ),
                MODEL,
            ),
            (error(ErrorKind::Provider, "API error 500: oops"), GENERIC),
            (
                error(ErrorKind::Config, "Select an audio model in Settings."),
                GENERIC,
            ),
        ];
        for (e, notice) in cases {
            assert_eq!(provider_notice(&e), notice, "{}", e.message);
        }
        assert_eq!(input_notice(&InputError::Blocked), BLOCKED);
        assert_eq!(input_notice(&InputError::Accessibility), ACCESSIBILITY);
        assert_eq!(input_notice(&InputError::ClipboardBusy), GENERIC);
    }

    #[test]
    fn bodies_name_the_step_that_failed() {
        let e = error(ErrorKind::Provider, "API error 500: oops");
        assert_eq!(
            provider_body(&e, Step::Transcribe),
            "Transcription failed: API error 500: oops"
        );
        assert_eq!(
            provider_body(&e, Step::Rewrite),
            "Rewrite failed: API error 500: oops"
        );
        let offline = error(
            ErrorKind::Network,
            "Couldn't reach x. Check your internet connection.",
        );
        assert_eq!(provider_body(&offline, Step::Rewrite), offline.message);
    }
}
