use ashpd::WindowIdentifier;
use ashpd::desktop::ResponseError;
use ashpd::desktop::screenshot::Screenshot;
use ashpd::{Error as PortalError, PortalError as DesktopPortalError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureMode {
    /// Interactive capture, with a full-screen fallback when the portal cannot
    /// show a region selector. A user dismissal never falls back.
    Auto,
    Region,
    Fullscreen,
}

#[derive(Debug)]
pub struct CaptureFailure {
    pub message: String,
    user_abort: bool,
}

impl CaptureFailure {
    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            user_abort: false,
        }
    }

    pub fn is_user_abort(&self) -> bool {
        self.user_abort
    }
}

#[derive(Debug, PartialEq, Eq)]
enum InteractiveFailureAction {
    Stop,
    Fallback,
}

impl From<PortalError> for CaptureFailure {
    fn from(err: PortalError) -> Self {
        // GNOME's interactive screenshot UI returns portal response 2 ("Other")
        // when the user closes it with Escape. That is a dismissal, not a broken
        // selector, so it must not trigger a full-screen capture.
        let user_abort = matches!(
            &err,
            PortalError::Response(ResponseError::Cancelled | ResponseError::Other)
                | PortalError::Portal(DesktopPortalError::Cancelled(_))
        );
        Self {
            message: err.to_string(),
            user_abort,
        }
    }
}

fn interactive_failure_action(
    mode: CaptureMode,
    failure: &CaptureFailure,
) -> InteractiveFailureAction {
    if mode == CaptureMode::Auto && !failure.is_user_abort() {
        InteractiveFailureAction::Fallback
    } else {
        InteractiveFailureAction::Stop
    }
}

fn niri_session() -> bool {
    ["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP"]
        .iter()
        .any(|variable| {
            std::env::var(variable).is_ok_and(|value| {
                value
                    .split(':')
                    .any(|desktop| desktop.eq_ignore_ascii_case("niri"))
            })
        })
}

async fn request_screenshot(
    identifier: Option<WindowIdentifier>,
    interactive: bool,
) -> Result<String, CaptureFailure> {
    Screenshot::request()
        .identifier(identifier)
        .interactive(interactive)
        .modal(true)
        .send()
        .await
        .and_then(|request| request.response())
        .map(|response| response.uri().to_string())
        .map_err(CaptureFailure::from)
}

pub async fn capture_screenshot(
    identifier: Option<WindowIdentifier>,
    mode: CaptureMode,
) -> Result<String, CaptureFailure> {
    let mode = if mode == CaptureMode::Auto && niri_session() {
        CaptureMode::Fullscreen
    } else {
        mode
    };

    match mode {
        CaptureMode::Fullscreen => request_screenshot(identifier, false).await,
        CaptureMode::Region => request_screenshot(identifier, true).await,
        CaptureMode::Auto => {
            let interactive = request_screenshot(identifier, true).await;
            match interactive {
                Ok(uri) => Ok(uri),
                Err(error)
                    if interactive_failure_action(CaptureMode::Auto, &error)
                        == InteractiveFailureAction::Stop =>
                {
                    Err(error)
                }
                Err(interactive_error) => {
                    request_screenshot(None, false)
                        .await
                        .map_err(|fallback| CaptureFailure {
                            message: format!(
                                "Interactive capture failed: {}. Full-screen fallback failed: {}",
                                interactive_error.message, fallback.message
                            ),
                            user_abort: fallback.user_abort,
                        })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnome_escape_does_not_fall_back_to_fullscreen() {
        let dismissed = CaptureFailure::from(PortalError::Response(ResponseError::Other));
        let cancelled = CaptureFailure::from(PortalError::Response(ResponseError::Cancelled));
        assert!(dismissed.is_user_abort());
        assert!(cancelled.is_user_abort());
        assert_eq!(
            interactive_failure_action(CaptureMode::Auto, &dismissed),
            InteractiveFailureAction::Stop
        );
        assert_eq!(
            interactive_failure_action(CaptureMode::Region, &cancelled),
            InteractiveFailureAction::Stop
        );
    }

    #[test]
    fn technical_interactive_failure_still_falls_back() {
        let failed = CaptureFailure::from(PortalError::Portal(DesktopPortalError::Failed(
            "interactive capture is unavailable".to_string(),
        )));
        assert!(!failed.is_user_abort());
        assert_eq!(
            interactive_failure_action(CaptureMode::Auto, &failed),
            InteractiveFailureAction::Fallback
        );
        assert_eq!(
            interactive_failure_action(CaptureMode::Region, &failed),
            InteractiveFailureAction::Stop
        );
    }
}
