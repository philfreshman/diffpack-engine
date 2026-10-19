use js_sys::{Error, Reflect};
use wasm_bindgen::JsValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureCode {
    NotFound,
    RateLimited,
    UpstreamFailed,
    Network,
    NoSuchVersion,
    NoArchive,
    UnsupportedArchiveHost,
}

impl FailureCode {
    pub fn as_str(self) -> &'static str {
        match self {
            FailureCode::NotFound => "not_found",
            FailureCode::RateLimited => "rate_limited",
            FailureCode::UpstreamFailed => "upstream_failed",
            FailureCode::Network => "network",
            FailureCode::NoSuchVersion => "no_such_version",
            FailureCode::NoArchive => "no_archive",
            FailureCode::UnsupportedArchiveHost => "unsupported_archive_host",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodedFailure {
    pub message: String,
    pub code: String,
    pub status: Option<u16>,
    pub resets_at: Option<String>,
}

impl CodedFailure {
    pub fn new(code: FailureCode, message: String) -> Self {
        CodedFailure {
            message,
            code: code.as_str().to_owned(),
            status: None,
            resets_at: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Coded(CodedFailure),
    Uncoded(String),
}

impl Failure {
    pub fn into_message(self) -> String {
        match self {
            Failure::Coded(failure) => failure.message,
            Failure::Uncoded(message) => message,
        }
    }
}

impl From<CodedFailure> for Failure {
    fn from(failure: CodedFailure) -> Self {
        Failure::Coded(failure)
    }
}

impl From<CodedFailure> for JsValue {
    fn from(failure: CodedFailure) -> Self {
        let error = Error::new(&failure.message);
        set_property(&error, "code", failure.code.into());
        if let Some(status) = failure.status {
            set_property(&error, "status", status.into());
        }
        if let Some(resets_at) = failure.resets_at {
            set_property(&error, "resetsAt", resets_at.into());
        }
        error.into()
    }
}

impl From<Failure> for JsValue {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Coded(failure) => failure.into(),
            Failure::Uncoded(message) => JsValue::from_str(&message),
        }
    }
}

fn set_property(error: &Error, name: &str, value: JsValue) {
    Reflect::set(error, &name.into(), &value).expect("a new Error accepts any property");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_failure_code_is_spelled_as_the_wasm_contract_spells_it() {
        assert_eq!(FailureCode::NotFound.as_str(), "not_found");
        assert_eq!(FailureCode::RateLimited.as_str(), "rate_limited");
        assert_eq!(FailureCode::UpstreamFailed.as_str(), "upstream_failed");
        assert_eq!(FailureCode::Network.as_str(), "network");
        assert_eq!(FailureCode::NoSuchVersion.as_str(), "no_such_version");
        assert_eq!(FailureCode::NoArchive.as_str(), "no_archive");
        assert_eq!(
            FailureCode::UnsupportedArchiveHost.as_str(),
            "unsupported_archive_host"
        );
    }
}
