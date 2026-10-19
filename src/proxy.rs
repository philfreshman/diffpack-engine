use std::cell::RefCell;

use crate::composer;
use crate::failure::{CodedFailure, FailureCode};

const HTTPS: &str = "https://";
const HTTP: &str = "http://";
const LOCAL_DEVELOPMENT_HOSTS: &[&str] = &["localhost", "127.0.0.1"];
const UNRESERVED_PUNCTUATION: &[u8] = b"-_.~";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyOrigin(String);

thread_local! {
    static CONFIGURED: RefCell<Option<ProxyOrigin>> = const { RefCell::new(None) };
}

pub fn configure(origin: &str) -> Result<(), String> {
    let origin = ProxyOrigin::parse(origin)?;
    CONFIGURED.with(|configured| *configured.borrow_mut() = Some(origin));
    Ok(())
}

pub fn configured() -> Option<ProxyOrigin> {
    CONFIGURED.with(|configured| configured.borrow().clone())
}

pub fn serves(url: &str) -> bool {
    configured().is_some_and(|origin| origin.serves(url))
}

pub fn required() -> Result<ProxyOrigin, CodedFailure> {
    configured().ok_or_else(|| {
        CodedFailure::new(
            FailureCode::ProxyNotConfigured,
            "Composer packages are downloaded through diffpack's proxy, and no proxy origin is set. Call set_proxy_origin first.".to_owned(),
        )
    })
}

impl ProxyOrigin {
    pub fn parse(origin: &str) -> Result<Self, String> {
        let without_slash = origin.strip_suffix('/').unwrap_or(origin);
        if is_https_origin(without_slash) || is_local_development_origin(without_slash) {
            Ok(ProxyOrigin(without_slash.to_owned()))
        } else {
            Err(format!(
                "Invalid proxy origin: {origin:?}. Use https://<host>, or http://localhost:<port> or http://127.0.0.1:<port> for development."
            ))
        }
    }

    fn serves(&self, url: &str) -> bool {
        url.strip_prefix(&self.0)
            .is_some_and(|rest| rest.starts_with('/'))
    }

    pub fn listing_url(&self, name: &str) -> Result<String, String> {
        let path = composer::package_path(name)?;
        Ok(format!("{}/v1/composer/p2/{path}.json", self.0))
    }

    pub fn archive_url(&self, name: &str, listed_url: &str) -> Result<String, String> {
        let path = composer::package_path(name)?;
        Ok(format!(
            "{}/v1/composer/archive?package={path}&url={}",
            self.0,
            percent_encoded(listed_url)
        ))
    }
}

fn percent_encoded(component: &str) -> String {
    component
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || UNRESERVED_PUNCTUATION.contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn is_https_origin(origin: &str) -> bool {
    origin
        .strip_prefix(HTTPS)
        .and_then(host_and_port)
        .is_some_and(is_host_name)
}

fn is_local_development_origin(origin: &str) -> bool {
    origin
        .strip_prefix(HTTP)
        .and_then(host_and_port)
        .is_some_and(|host| LOCAL_DEVELOPMENT_HOSTS.contains(&host))
}

fn host_and_port(authority: &str) -> Option<&str> {
    match authority.split_once(':') {
        Some((host, port)) if is_port(port) => Some(host),
        Some(_) => None,
        None => Some(authority),
    }
}

fn is_port(port: &str) -> bool {
    !port.is_empty() && port.chars().all(|character| character.is_ascii_digit())
}

fn is_host_name(host: &str) -> bool {
    !host.is_empty()
        && host
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-".contains(character))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(origin: &str) -> Result<String, String> {
        ProxyOrigin::parse(origin).map(|origin| origin.0)
    }

    #[test]
    fn an_https_origin_is_accepted_without_its_trailing_slash() {
        for (given, kept) in [
            (
                "https://diffpack-proxy.philipswiezak.workers.dev",
                "https://diffpack-proxy.philipswiezak.workers.dev",
            ),
            (
                "https://diffpack-proxy.philipswiezak.workers.dev/",
                "https://diffpack-proxy.philipswiezak.workers.dev",
            ),
            ("https://proxy.example:8443", "https://proxy.example:8443"),
        ] {
            assert_eq!(parsed(given), Ok(kept.to_owned()), "{given}");
        }
    }

    #[test]
    fn a_local_development_origin_is_accepted_over_http() {
        for origin in [
            "http://localhost:8787",
            "http://127.0.0.1:8787",
            "http://localhost",
        ] {
            assert_eq!(parsed(origin), Ok(origin.to_owned()));
        }
        assert_eq!(
            parsed("http://localhost:8787/"),
            Ok("http://localhost:8787".to_owned())
        );
    }

    #[test]
    fn an_origin_with_a_path_query_user_or_other_scheme_is_refused() {
        for origin in [
            "",
            "proxy.example",
            "https://",
            "https:///",
            "https://proxy.example//",
            "https://proxy.example/v1",
            "https://proxy.example?x=1",
            "https://proxy.example#top",
            "https://user@proxy.example",
            "https://proxy.example:",
            "https://proxy.example:80a",
            "https://proxy example",
            "http://proxy.example",
            "http://localhost.evil.example:8787",
            "http://127.0.0.2:8787",
            "ftp://proxy.example",
            "HTTPS://proxy.example",
        ] {
            assert_eq!(
                parsed(origin),
                Err(format!(
                    "Invalid proxy origin: {origin:?}. Use https://<host>, or http://localhost:<port> or http://127.0.0.1:<port> for development."
                )),
                "{origin}"
            );
        }
    }

    #[test]
    fn no_origin_is_configured_until_one_is_set() {
        assert_eq!(configured(), None);
        configure("https://proxy.test/").unwrap();
        assert_eq!(
            configured(),
            Some(ProxyOrigin::parse("https://proxy.test").unwrap())
        );
    }

    #[test]
    fn a_refused_origin_keeps_the_one_set_before() {
        configure("https://proxy.test").unwrap();
        assert!(configure("https://proxy.test/v1").is_err());
        assert_eq!(
            configured(),
            Some(ProxyOrigin::parse("https://proxy.test").unwrap())
        );
    }

    fn test_proxy() -> ProxyOrigin {
        ProxyOrigin::parse("https://proxy.test").unwrap()
    }

    #[test]
    fn a_composer_listing_is_read_through_the_proxy_p2_route() {
        assert_eq!(
            test_proxy().listing_url("Monolog/Monolog"),
            Ok("https://proxy.test/v1/composer/p2/monolog/monolog.json".to_owned())
        );
        assert_eq!(
            test_proxy().listing_url("not a name"),
            Err("Invalid Composer package name: not a name".to_owned())
        );
    }

    #[test]
    fn a_composer_archive_is_fetched_through_the_proxy_with_the_url_packagist_lists() {
        assert_eq!(
            test_proxy().archive_url(
                "Monolog/Monolog",
                "https://api.github.com/repos/Seldaek/monolog/zipball/d7059e4c691d05ea1222b5124bed0f961be46e15"
            ),
            Ok("https://proxy.test/v1/composer/archive?package=monolog/monolog&url=https%3A%2F%2Fapi.github.com%2Frepos%2FSeldaek%2Fmonolog%2Fzipball%2Fd7059e4c691d05ea1222b5124bed0f961be46e15".to_owned())
        );
    }

    #[test]
    fn every_reserved_character_of_the_listed_url_is_escaped() {
        assert_eq!(
            test_proxy().archive_url(
                "group/project",
                "https://gitlab.com/api/v4/projects/group%2Fproject/repository/archive.zip?sha=abc&x=a b+c~d_e.f-g"
            ),
            Ok("https://proxy.test/v1/composer/archive?package=group/project&url=https%3A%2F%2Fgitlab.com%2Fapi%2Fv4%2Fprojects%2Fgroup%252Fproject%2Frepository%2Farchive.zip%3Fsha%3Dabc%26x%3Da%20b%2Bc~d_e.f-g".to_owned())
        );
    }

    #[test]
    fn requiring_the_proxy_before_an_origin_is_set_is_a_proxy_not_configured_failure() {
        let failure = required().expect_err("no origin was set on this thread");
        assert_eq!(failure.code, FailureCode::ProxyNotConfigured.as_str());
        assert_eq!(
            failure.message,
            "Composer packages are downloaded through diffpack's proxy, and no proxy origin is set. Call set_proxy_origin first."
        );
        configure("https://proxy.test").unwrap();
        assert_eq!(required(), Ok(test_proxy()));
    }

    #[test]
    fn only_urls_under_the_configured_origin_are_served_by_the_proxy() {
        assert!(!serves("https://proxy.test/v1/composer/p2/o/r.json"));
        configure("https://proxy.test").unwrap();
        assert!(serves("https://proxy.test/v1/composer/p2/o/r.json"));
        assert!(serves(
            "https://proxy.test/v1/composer/archive?package=o/r&url=x"
        ));
        assert!(!serves(
            "https://proxy.test.evil.example/v1/composer/p2/o/r.json"
        ));
        assert!(!serves("https://proxy.testing/v1"));
        assert!(!serves(
            "https://static.crates.io/crates/itoa/itoa-1.0.0.crate"
        ));
    }
}
