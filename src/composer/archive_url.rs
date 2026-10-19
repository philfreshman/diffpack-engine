use crate::failure::{CodedFailure, FailureCode};

const GITHUB_API_HOST: &str = "api.github.com";
const SUPPORTED_HOSTS: &[&str] = &[GITHUB_API_HOST];
const HTTPS: &str = "https://";
const FULL_SHA_LENGTH: usize = 40;

pub fn direct_url(listed_url: &str) -> Result<String, CodedFailure> {
    parts_of(listed_url)
        .and_then(|parts| match parts.authority {
            GITHUB_API_HOST => github_codeload_url(&parts),
            _ => None,
        })
        .ok_or_else(|| unsupported(listed_url))
}

struct UrlParts<'a> {
    authority: &'a str,
    segments: Vec<&'a str>,
    query: Option<&'a str>,
}

fn parts_of(url: &str) -> Option<UrlParts<'_>> {
    let rest = url.strip_prefix(HTTPS)?;
    let (authority, path_and_query) = rest.split_at(rest.find('/')?);
    let (path, query) = match path_and_query.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (path_and_query, None),
    };
    Some(UrlParts {
        authority,
        segments: path.strip_prefix('/')?.split('/').collect(),
        query,
    })
}

fn github_codeload_url(parts: &UrlParts) -> Option<String> {
    match (parts.segments.as_slice(), parts.query) {
        (["repos", owner, repo, "zipball", sha], None)
            if is_repository_part(owner) && is_repository_part(repo) && is_full_sha(sha) =>
        {
            Some(format!(
                "https://codeload.github.com/{owner}/{repo}/legacy.zip/{sha}"
            ))
        }
        _ => None,
    }
}

fn is_repository_part(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_.-".contains(character))
}

fn is_full_sha(sha: &str) -> bool {
    sha.len() == FULL_SHA_LENGTH && is_hex(sha)
}

fn is_hex(text: &str) -> bool {
    text.chars().all(|character| character.is_ascii_hexdigit())
}

fn unsupported(listed_url: &str) -> CodedFailure {
    let host = host_of(listed_url);
    let message = if SUPPORTED_HOSTS.contains(&host) {
        format!("diffpack can't download this {host} archive: {listed_url}")
    } else {
        format!("diffpack can't download archives from {host} yet: {listed_url}")
    };
    CodedFailure::new(FailureCode::UnsupportedArchiveHost, message)
}

fn host_of(url: &str) -> &str {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    let without_user = authority.rsplit('@').next().unwrap_or(authority);
    without_user.split(':').next().unwrap_or(without_user)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "d7059e4c691d05ea1222b5124bed0f961be46e15";

    fn rejection(listed_url: &str) -> CodedFailure {
        direct_url(listed_url).expect_err(listed_url)
    }

    #[test]
    fn a_github_zipball_is_downloaded_from_codeload_without_the_api_hop() {
        assert_eq!(
            direct_url(&format!(
                "https://api.github.com/repos/Seldaek/monolog/zipball/{SHA}"
            )),
            Ok(format!(
                "https://codeload.github.com/Seldaek/monolog/legacy.zip/{SHA}"
            ))
        );
        assert_eq!(
            direct_url(&format!(
                "https://api.github.com/repos/my_org.x/my-repo.php/zipball/{SHA}"
            )),
            Ok(format!(
                "https://codeload.github.com/my_org.x/my-repo.php/legacy.zip/{SHA}"
            ))
        );
    }

    #[test]
    fn an_archive_on_another_host_is_unsupported_and_names_the_host() {
        let failure = rejection("https://git.drupalcode.org/project/x/-/archive/1.0.zip");
        assert_eq!(failure.code, FailureCode::UnsupportedArchiveHost.as_str());
        assert_eq!(failure.status, None);
        assert_eq!(
            failure.message,
            "diffpack can't download archives from git.drupalcode.org yet: https://git.drupalcode.org/project/x/-/archive/1.0.zip"
        );
    }

    #[test]
    fn a_host_that_only_looks_like_github_is_unsupported() {
        for host in [
            "api.github.com.evil.example",
            "evilapi.github.com",
            "github.com",
        ] {
            let url = format!("https://{host}/repos/o/r/zipball/{SHA}");
            let failure = rejection(&url);
            assert_eq!(failure.code, FailureCode::UnsupportedArchiveHost.as_str());
            assert!(failure.message.contains(host), "{}", failure.message);
        }
    }

    #[test]
    fn a_github_url_that_breaks_the_strict_pattern_is_unsupported() {
        for url in [
            format!("http://api.github.com/repos/o/r/zipball/{SHA}"),
            format!("https://api.github.com:443/repos/o/r/zipball/{SHA}"),
            format!("https://user@api.github.com/repos/o/r/zipball/{SHA}"),
            format!("https://user:pass@api.github.com/repos/o/r/zipball/{SHA}"),
            "https://api.github.com/repos/o/r/zipball/d7059e4".to_string(),
            format!("https://api.github.com/repos/o/r/zipball/{SHA}0"),
            "https://api.github.com/repos/o/r/zipball/g7059e4c691d05ea1222b5124bed0f961be46e15"
                .to_string(),
            format!("https://api.github.com/repos/../r/zipball/{SHA}"),
            format!("https://api.github.com/repos/o/./zipball/{SHA}"),
            format!("https://api.github.com/repos/o/r/../zipball/{SHA}"),
            format!("https://api.github.com/repos/o%2fx/r/zipball/{SHA}"),
            format!("https://api.github.com/repos/o/r/zipball/{SHA}?access_token=x"),
            format!("https://api.github.com/repos/o/r/zipball/{SHA}#top"),
            format!("https://api.github.com/repos/o/r/tarball/{SHA}"),
            format!("https://api.github.com/repos/o/r/zipball/{SHA}/"),
            format!("https://api.github.com/repos//r/zipball/{SHA}"),
        ] {
            let failure = rejection(&url);
            assert_eq!(
                failure.code,
                FailureCode::UnsupportedArchiveHost.as_str(),
                "{url}"
            );
            assert_eq!(
                failure.message,
                format!("diffpack can't download this api.github.com archive: {url}")
            );
        }
    }
}
