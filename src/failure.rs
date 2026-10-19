use js_sys::{Error, Reflect};
use wasm_bindgen::JsValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureCode {
    NotFound,
    RateLimited,
    UpstreamFailed,
    Network,
}

impl FailureCode {
    pub fn as_str(self) -> &'static str {
        match self {
            FailureCode::NotFound => "not_found",
            FailureCode::RateLimited => "rate_limited",
            FailureCode::UpstreamFailed => "upstream_failed",
            FailureCode::Network => "network",
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
    }
}
