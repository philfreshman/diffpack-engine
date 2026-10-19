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
