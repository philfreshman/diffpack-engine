use js_sys::{Error, Reflect};
use serde::Deserialize;
use wasm_bindgen::JsValue;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchFailure {
    Answered {
        url: String,
        status: u16,
    },
    AnsweredByProxy {
        url: String,
        status: u16,
        error: ProxyError,
    },
    UnreadableProxyAnswer {
        url: String,
        status: u16,
    },
    Unanswered {
        url: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyError {
    #[serde(rename = "error")]
    code: String,
    message: Option<String>,
    resets_at: Option<String>,
}

const NOT_FOUND: u16 = 404;
const TOO_MANY_REQUESTS: u16 = 429;
const UPSTREAM_FAILED: &str = "upstream_failed";

impl FetchFailure {
    pub fn from_answer(url: &str, status: u16, proxy_error_body: Option<&[u8]>) -> Self {
        let url = url.to_owned();
        let Some(body) = proxy_error_body else {
            return FetchFailure::Answered { url, status };
        };
        match serde_json::from_slice(body) {
            Ok(error) => FetchFailure::AnsweredByProxy { url, status, error },
            Err(_) => FetchFailure::UnreadableProxyAnswer { url, status },
        }
    }

    pub fn unanswered(url: &str) -> Self {
        FetchFailure::Unanswered {
            url: url.to_owned(),
        }
    }

    pub fn code(&self) -> &str {
        match self {
            FetchFailure::Answered { status, .. } => code_for_status(*status),
            FetchFailure::AnsweredByProxy { error, .. } => &error.code,
            FetchFailure::UnreadableProxyAnswer { .. } => UPSTREAM_FAILED,
            FetchFailure::Unanswered { .. } => "network",
        }
    }

    pub fn status(&self) -> Option<u16> {
        match self {
            FetchFailure::Answered { status, .. }
            | FetchFailure::AnsweredByProxy { status, .. }
            | FetchFailure::UnreadableProxyAnswer { status, .. } => Some(*status),
            FetchFailure::Unanswered { .. } => None,
        }
    }

    pub fn resets_at(&self) -> Option<&str> {
        match self {
            FetchFailure::AnsweredByProxy { error, .. } => error.resets_at.as_deref(),
            _ => None,
        }
    }

    pub fn message(&self) -> String {
        match self {
            FetchFailure::Answered { url, status }
            | FetchFailure::UnreadableProxyAnswer { url, status } => {
                download_failed_with_status(url, *status)
            }
            FetchFailure::AnsweredByProxy { url, status, error } => match &error.message {
                Some(explanation) => {
                    format!(
                        "{}: {explanation}",
                        download_failed_with_status(url, *status)
                    )
                }
                None => download_failed_with_status(url, *status),
            },
            FetchFailure::Unanswered { url } => format!("Couldn't download {url} (no answer)"),
        }
    }
}

fn download_failed_with_status(url: &str, status: u16) -> String {
    format!("Couldn't download {url} (HTTP {status})")
}

impl From<FetchFailure> for JsValue {
    fn from(failure: FetchFailure) -> Self {
        let error = Error::new(&failure.message());
        set_property(&error, "code", failure.code().into());
        if let Some(status) = failure.status() {
            set_property(&error, "status", status.into());
        }
        if let Some(resets_at) = failure.resets_at() {
            set_property(&error, "resetsAt", resets_at.into());
        }
        error.into()
    }
}

fn set_property(error: &Error, name: &str, value: JsValue) {
    Reflect::set(error, &name.into(), &value).expect("a new Error accepts any property");
}

fn code_for_status(status: u16) -> &'static str {
    match status {
        NOT_FOUND => "not_found",
        TOO_MANY_REQUESTS => "rate_limited",
        _ => UPSTREAM_FAILED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://static.crates.io/crates/itoa/itoa-9.9.9.crate";

    const QUOTA_EXHAUSTED_BODY: &str = r#"{
        "error": "quota_exhausted",
        "message": "diffpack's Composer proxy has used today's free allowance. It resets at 00:00 UTC.",
        "resetsAt": "2026-10-20T00:00:00Z"
    }"#;

    fn plain_answer(status: u16) -> FetchFailure {
        FetchFailure::from_answer(URL, status, None)
    }

    fn proxy_answer(status: u16, body: &str) -> FetchFailure {
        FetchFailure::from_answer(URL, status, Some(body.as_bytes()))
    }

    #[test]
    fn a_plain_answer_is_coded_by_its_status() {
        assert_eq!(plain_answer(404).code(), "not_found");
        assert_eq!(plain_answer(429).code(), "rate_limited");
        assert_eq!(plain_answer(500).code(), "upstream_failed");
        assert_eq!(plain_answer(403).code(), "upstream_failed");
    }

    #[test]
    fn a_plain_answer_keeps_its_status_and_has_no_reset_time() {
        let failure = plain_answer(404);
        assert_eq!(failure.status(), Some(404));
        assert_eq!(failure.resets_at(), None);
    }

    #[test]
    fn a_plain_answer_reads_as_a_download_that_failed_with_its_status() {
        assert_eq!(
            plain_answer(404).message(),
            format!("Couldn't download {URL} (HTTP 404)")
        );
    }

    #[test]
    fn a_download_that_got_no_answer_is_a_network_failure_without_a_status() {
        let failure = FetchFailure::unanswered(URL);
        assert_eq!(failure.code(), "network");
        assert_eq!(failure.status(), None);
        assert_eq!(failure.resets_at(), None);
        assert_eq!(
            failure.message(),
            format!("Couldn't download {URL} (no answer)")
        );
    }

    #[test]
    fn a_proxy_answer_carries_its_own_code_its_status_and_its_reset_time() {
        let failure = proxy_answer(503, QUOTA_EXHAUSTED_BODY);
        assert_eq!(failure.code(), "quota_exhausted");
        assert_eq!(failure.status(), Some(503));
        assert_eq!(failure.resets_at(), Some("2026-10-20T00:00:00Z"));
    }

    #[test]
    fn a_proxy_answer_reads_as_a_failed_download_followed_by_the_proxy_explanation() {
        assert_eq!(
            proxy_answer(503, QUOTA_EXHAUSTED_BODY).message(),
            format!(
                "Couldn't download {URL} (HTTP 503): diffpack's Composer proxy has used today's free allowance. It resets at 00:00 UTC."
            )
        );
    }

    #[test]
    fn a_proxy_code_wins_over_what_its_status_would_mean_and_needs_no_reset_time() {
        let failure = proxy_answer(
            403,
            r#"{"error": "archive_not_listed", "message": "Packagist does not list that archive."}"#,
        );
        assert_eq!(failure.code(), "archive_not_listed");
        assert_eq!(failure.status(), Some(403));
        assert_eq!(failure.resets_at(), None);
    }

    #[test]
    fn a_proxy_answer_with_a_broken_body_is_an_upstream_failure_with_its_status() {
        let broken_bodies = [
            r#"{"message": "no code"}"#,
            r#"{"error": 7, "message": "a code that is not text"}"#,
            r#"{"error": "quota_exhausted", "message": "cut off"#,
            "",
        ];
        for body in broken_bodies {
            let failure = proxy_answer(404, body);
            assert_eq!(failure.code(), "upstream_failed", "{body}");
            assert_eq!(failure.status(), Some(404), "{body}");
            assert_eq!(failure.resets_at(), None, "{body}");
            assert_eq!(
                failure.message(),
                format!("Couldn't download {URL} (HTTP 404)"),
                "{body}"
            );
        }
    }

    #[test]
    fn a_proxy_answer_whose_body_is_not_json_is_an_upstream_failure_with_its_status() {
        let failure = proxy_answer(503, "<html><h1>Error 1027</h1></html>");
        assert_eq!(failure.code(), "upstream_failed");
        assert_eq!(failure.status(), Some(503));
    }
}
