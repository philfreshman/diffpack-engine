use js_sys::{Error, Reflect};
use wasm_bindgen::JsValue;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchFailure {
    Answered { url: String, status: u16 },
    Unanswered { url: String },
}

const NOT_FOUND: u16 = 404;
const TOO_MANY_REQUESTS: u16 = 429;

impl FetchFailure {
    pub fn answered(url: &str, status: u16) -> Self {
        FetchFailure::Answered {
            url: url.to_owned(),
            status,
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
            FetchFailure::Unanswered { .. } => "network",
        }
    }

    pub fn status(&self) -> Option<u16> {
        match self {
            FetchFailure::Answered { status, .. } => Some(*status),
            FetchFailure::Unanswered { .. } => None,
        }
    }

    pub fn resets_at(&self) -> Option<&str> {
        None
    }

    pub fn message(&self) -> String {
        match self {
            FetchFailure::Answered { url, status } => {
                format!("Couldn't download {url} (HTTP {status})")
            }
            FetchFailure::Unanswered { url } => format!("Couldn't download {url} (no answer)"),
        }
    }
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
        _ => "upstream_failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://static.crates.io/crates/itoa/itoa-9.9.9.crate";

    fn plain_answer(status: u16) -> FetchFailure {
        FetchFailure::answered(URL, status)
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
}
