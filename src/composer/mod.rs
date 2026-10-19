mod archive_url;
mod listing;

use crate::failure::{CodedFailure, Failure, FailureCode};

const PACKAGIST_LISTING_ROOT: &str = "https://repo.packagist.org/p2";
const VENDOR_SEPARATORS: &[&str] = &["_", ".", "-"];
const PACKAGE_SEPARATORS: &[&str] = &["_", ".", "-", "--"];

pub fn listing_url(name: &str) -> Result<String, String> {
    let lowercase = name.to_ascii_lowercase();
    match lowercase.split_once('/') {
        Some((vendor, package))
            if is_name_part(vendor, VENDOR_SEPARATORS)
                && is_name_part(package, PACKAGE_SEPARATORS) =>
        {
            Ok(format!("{PACKAGIST_LISTING_ROOT}/{vendor}/{package}.json"))
        }
        _ => Err(format!("Invalid Composer package name: {name}")),
    }
}

fn is_name_part(part: &str, separators: &[&str]) -> bool {
    part.starts_with(is_name_character)
        && part.ends_with(is_name_character)
        && part
            .split(is_name_character)
            .all(|run| run.is_empty() || separators.contains(&run))
}

fn is_name_character(character: char) -> bool {
    character.is_ascii_lowercase() || character.is_ascii_digit()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerDist {
    pub listed_url: String,
    pub direct_url: String,
}

pub fn find_dist(version: &str, listing: &str) -> Result<ComposerDist, Failure> {
    let package = listing::read(listing).map_err(Failure::Uncoded)?;
    let listed = package
        .find(version)
        .ok_or_else(|| no_such_version(&package.name, version))?;
    let listed_url = listed
        .dist_url
        .clone()
        .ok_or_else(|| no_archive(&package.name, version))?;
    let direct_url = archive_url::direct_url(&listed_url)?;
    Ok(ComposerDist {
        listed_url,
        direct_url,
    })
}

fn no_such_version(name: &str, version: &str) -> CodedFailure {
    CodedFailure::new(
        FailureCode::NoSuchVersion,
        format!("Packagist lists no version {version} of {name}"),
    )
}

fn no_archive(name: &str, version: &str) -> CodedFailure {
    CodedFailure::new(
        FailureCode::NoArchive,
        format!("Packagist lists no archive for {name} {version}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD_HASHER_LISTING: &str = r#"{
        "minified": "composer/2.0",
        "packages": {
            "symfony/password-hasher": [
                {
                    "version": "v8.0.4",
                    "version_normalized": "8.0.4.0",
                    "dist": {
                        "url": "https://api.github.com/repos/symfony/password-hasher/zipball/ca6af4e20357d58d50c818d676cf2e2dd5e53b02",
                        "type": "zip",
                        "shasum": "",
                        "reference": "ca6af4e20357d58d50c818d676cf2e2dd5e53b02"
                    },
                    "time": "2026-01-01T23:07:29+00:00"
                },
                {
                    "version": "v8.0.0",
                    "version_normalized": "8.0.0.0",
                    "dist": {
                        "url": "https://api.github.com/repos/symfony/password-hasher/zipball/80e7c7e3fdb27214ba3af9fd449cb638aa98daa4",
                        "type": "zip",
                        "shasum": "",
                        "reference": "80e7c7e3fdb27214ba3af9fd449cb638aa98daa4"
                    },
                    "time": "2025-08-13T16:50:52+00:00"
                },
                {
                    "version": "v8.0.0-RC1",
                    "version_normalized": "8.0.0.0-RC1"
                },
                {
                    "version": "v8.0.0-BETA1",
                    "version_normalized": "8.0.0.0-beta1"
                },
                {
                    "version": "v7.4.8",
                    "version_normalized": "7.4.8.0",
                    "dist": {
                        "url": "https://api.github.com/repos/symfony/password-hasher/zipball/18a7d92126c95962f7efbcc9e421ba710a366847",
                        "type": "zip",
                        "shasum": "",
                        "reference": "18a7d92126c95962f7efbcc9e421ba710a366847"
                    },
                    "time": "2026-03-24T13:12:05+00:00"
                }
            ]
        }
    }"#;

    const MONOLOG_LISTING: &str = r#"{
        "minified": "composer/2.0",
        "packages": {
            "monolog/monolog": [
                {
                    "name": "monolog/monolog",
                    "version": "3.12.1",
                    "version_normalized": "3.12.1.0",
                    "dist": {
                        "url": "https://api.github.com/repos/Seldaek/monolog/zipball/d7059e4c691d05ea1222b5124bed0f961be46e15",
                        "type": "zip",
                        "shasum": "",
                        "reference": "d7059e4c691d05ea1222b5124bed0f961be46e15"
                    },
                    "time": "2026-09-29T19:21:12+00:00"
                },
                {
                    "version": "3.12.0",
                    "version_normalized": "3.12.0.0",
                    "dist": {
                        "url": "https://api.github.com/repos/Seldaek/monolog/zipball/72c534fc0ab181ef52d92a68382318631e301608",
                        "type": "zip",
                        "shasum": "",
                        "reference": "72c534fc0ab181ef52d92a68382318631e301608"
                    },
                    "time": "2026-09-09T08:34:20+00:00"
                }
            ]
        }
    }"#;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn github_dist(sha: &str) -> String {
        format!(
            r#"{{"type": "zip", "url": "https://api.github.com/repos/o/r/zipball/{sha}", "reference": "{sha}", "shasum": ""}}"#
        )
    }

    fn codeload(sha: &str) -> String {
        format!("https://codeload.github.com/o/r/legacy.zip/{sha}")
    }

    fn listing_of(minified: bool, entries: &[String]) -> String {
        let minified = if minified {
            r#""minified": "composer/2.0","#
        } else {
            ""
        };
        format!(
            r#"{{{minified} "packages": {{"o/r": [{}]}}}}"#,
            entries.join(",")
        )
    }

    fn direct_url_of(version: &str, listing: &str) -> String {
        find_dist(version, listing)
            .unwrap_or_else(|failure| panic!("{version}: {failure:?}"))
            .direct_url
    }

    fn coded_failure(version: &str, listing: &str) -> CodedFailure {
        match find_dist(version, listing) {
            Err(Failure::Coded(failure)) => failure,
            other => panic!("{version}: expected a coded failure, got {other:?}"),
        }
    }

    #[test]
    fn a_version_with_its_own_dist_resolves_to_it() {
        assert_eq!(
            direct_url_of("v8.0.4", PASSWORD_HASHER_LISTING),
            "https://codeload.github.com/symfony/password-hasher/legacy.zip/ca6af4e20357d58d50c818d676cf2e2dd5e53b02"
        );
        assert_eq!(
            direct_url_of("v7.4.8", PASSWORD_HASHER_LISTING),
            "https://codeload.github.com/symfony/password-hasher/legacy.zip/18a7d92126c95962f7efbcc9e421ba710a366847"
        );
    }

    #[test]
    fn a_version_without_a_dist_of_its_own_inherits_the_one_listed_before_it() {
        let v8_0_0 = "https://codeload.github.com/symfony/password-hasher/legacy.zip/80e7c7e3fdb27214ba3af9fd449cb638aa98daa4";
        assert_eq!(direct_url_of("v8.0.0-RC1", PASSWORD_HASHER_LISTING), v8_0_0);
        assert_eq!(
            direct_url_of("v8.0.0-BETA1", PASSWORD_HASHER_LISTING),
            v8_0_0
        );
    }

    #[test]
    fn the_dist_keeps_the_url_packagist_lists_next_to_the_direct_one() {
        let dist = find_dist("3.12.1", MONOLOG_LISTING).unwrap();
        assert_eq!(
            dist.listed_url,
            "https://api.github.com/repos/Seldaek/monolog/zipball/d7059e4c691d05ea1222b5124bed0f961be46e15"
        );
        assert_eq!(
            dist.direct_url,
            "https://codeload.github.com/Seldaek/monolog/legacy.zip/d7059e4c691d05ea1222b5124bed0f961be46e15"
        );
    }

    #[test]
    fn a_version_matches_with_or_without_its_leading_v() {
        assert_eq!(
            direct_url_of("v3.12.1", MONOLOG_LISTING),
            direct_url_of("3.12.1", MONOLOG_LISTING)
        );
        assert_eq!(
            direct_url_of("8.0.4", PASSWORD_HASHER_LISTING),
            direct_url_of("v8.0.4", PASSWORD_HASHER_LISTING)
        );
    }

    #[test]
    fn an_exact_version_wins_over_one_that_matches_without_its_v() {
        let listing = listing_of(
            true,
            &[
                format!(r#"{{"version": "v1.0.0", "dist": {}}}"#, github_dist(SHA_A)),
                format!(r#"{{"version": "1.0.0", "dist": {}}}"#, github_dist(SHA_B)),
            ],
        );
        assert_eq!(direct_url_of("1.0.0", &listing), codeload(SHA_B));
        assert_eq!(direct_url_of("v1.0.0", &listing), codeload(SHA_A));
    }

    #[test]
    fn only_one_leading_v_is_ignored() {
        let failure = coded_failure("vv3.12.1", MONOLOG_LISTING);
        assert_eq!(failure.code, FailureCode::NoSuchVersion.as_str());
    }

    #[test]
    fn a_version_packagist_does_not_list_is_no_such_version() {
        let failure = coded_failure("9.9.9", MONOLOG_LISTING);
        assert_eq!(failure.code, FailureCode::NoSuchVersion.as_str());
        assert_eq!(failure.status, None);
        assert_eq!(
            failure.message,
            "Packagist lists no version 9.9.9 of monolog/monolog"
        );
    }

    #[test]
    fn an_unset_dist_is_no_archive_until_a_later_entry_lists_one_again() {
        let listing = listing_of(
            true,
            &[
                format!(r#"{{"version": "3.0.0", "dist": {}}}"#, github_dist(SHA_A)),
                r#"{"version": "2.0.0", "dist": "__unset"}"#.to_string(),
                r#"{"version": "1.5.0"}"#.to_string(),
                format!(r#"{{"version": "1.0.0", "dist": {}}}"#, github_dist(SHA_B)),
            ],
        );
        assert_eq!(direct_url_of("3.0.0", &listing), codeload(SHA_A));
        for version in ["2.0.0", "1.5.0"] {
            let failure = coded_failure(version, &listing);
            assert_eq!(failure.code, FailureCode::NoArchive.as_str());
            assert_eq!(
                failure.message,
                format!("Packagist lists no archive for o/r {version}")
            );
        }
        assert_eq!(direct_url_of("1.0.0", &listing), codeload(SHA_B));
    }

    #[test]
    fn a_version_never_given_a_dist_is_no_archive() {
        let listing = listing_of(true, &[r#"{"version": "1.0.0"}"#.to_string()]);
        assert_eq!(
            coded_failure("1.0.0", &listing).code,
            FailureCode::NoArchive.as_str()
        );
    }

    #[test]
    fn a_dist_without_a_url_is_no_archive() {
        let listing = listing_of(
            true,
            &[r#"{"version": "1.0.0", "dist": {"type": "zip"}}"#.to_string()],
        );
        assert_eq!(
            coded_failure("1.0.0", &listing).code,
            FailureCode::NoArchive.as_str()
        );
    }

    #[test]
    fn a_listing_that_is_not_minified_inherits_nothing() {
        let listing = listing_of(
            false,
            &[
                format!(r#"{{"version": "2.0.0", "dist": {}}}"#, github_dist(SHA_A)),
                r#"{"version": "1.0.0"}"#.to_string(),
            ],
        );
        assert_eq!(direct_url_of("2.0.0", &listing), codeload(SHA_A));
        assert_eq!(
            coded_failure("1.0.0", &listing).code,
            FailureCode::NoArchive.as_str()
        );
    }

    #[test]
    fn a_dist_on_an_unsupported_host_is_reported_as_such() {
        let listing = listing_of(
            true,
            &[r#"{"version": "1.0.0", "dist": {"type": "zip", "url": "https://git.drupalcode.org/x.zip"}}"#.to_string()],
        );
        assert_eq!(
            coded_failure("1.0.0", &listing).code,
            FailureCode::UnsupportedArchiveHost.as_str()
        );
    }

    #[test]
    fn a_listing_that_is_not_a_packagist_document_is_an_uncoded_error() {
        for listing in ["not json", r#"{"packages": []}"#, r#"{"packages": {}}"#] {
            match find_dist("1.0.0", listing) {
                Err(Failure::Uncoded(message)) => {
                    assert!(
                        message.starts_with("Failed to read the Packagist listing"),
                        "{message}"
                    )
                }
                other => panic!("{listing}: expected an uncoded error, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_listing_must_describe_exactly_one_package() {
        let listing = format!(
            r#"{{"packages": {{"a/one": [{{"version": "1.0.0", "dist": {first}}}], "b/two": [{{"version": "1.0.0", "dist": {second}}}]}}}}"#,
            first = github_dist(SHA_A),
            second = github_dist(SHA_B),
        );
        assert_eq!(
            find_dist("1.0.0", &listing),
            Err(Failure::Uncoded(
                "Failed to read the Packagist listing: it lists 2 packages instead of one"
                    .to_string()
            ))
        );
    }
}
