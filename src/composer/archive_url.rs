use crate::failure::{CodedFailure, FailureCode};

const GITHUB_API_HOST: &str = "api.github.com";
const BITBUCKET_HOST: &str = "bitbucket.org";
const GITLAB_HOST: &str = "gitlab.com";
const SUPPORTED_HOSTS: &[&str] = &[GITHUB_API_HOST, BITBUCKET_HOST, GITLAB_HOST];
const HTTPS: &str = "https://";
const FULL_SHA_LENGTH: usize = 40;
const SHORTEST_ABBREVIATED_SHA: usize = 7;
const DOT_SEGMENTS: &[&str] = &[".", ".."];

pub fn direct_url(listed_url: &str) -> Result<String, CodedFailure> {
    parts_of(listed_url)
        .and_then(|parts| match parts.authority {
            GITHUB_API_HOST => github_codeload_url(&parts),
            BITBUCKET_HOST => is_bitbucket_archive(&parts).then(|| listed_url.to_owned()),
            GITLAB_HOST => is_gitlab_archive(&parts).then(|| listed_url.to_owned()),
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

fn is_bitbucket_archive(parts: &UrlParts) -> bool {
    match (parts.segments.as_slice(), parts.query) {
        ([owner, repo, "get", file], None) => {
            is_repository_part(owner)
                && is_repository_part(repo)
                && file.strip_suffix(".zip").is_some_and(is_abbreviated_sha)
        }
        _ => false,
    }
}

fn is_gitlab_archive(parts: &UrlParts) -> bool {
    match (parts.segments.as_slice(), parts.query) {
        (["api", "v4", "projects", project, "repository", "archive.zip"], Some(query)) => {
            is_gitlab_project(project) && query.strip_prefix("sha=").is_some_and(is_full_sha)
        }
        _ => false,
    }
}

fn is_repository_part(part: &str) -> bool {
    is_plain_segment_of(part, "_.-")
}

fn is_gitlab_project(project: &str) -> bool {
    is_plain_segment_of(project, "%_.-")
}

fn is_plain_segment_of(segment: &str, punctuation: &str) -> bool {
    !segment.is_empty()
        && segment
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || punctuation.contains(character))
        && percent_decoded(segment).is_some_and(|decoded| !DOT_SEGMENTS.contains(&decoded.as_str()))
}

fn percent_decoded(segment: &str) -> Option<String> {
    let mut decoded = Vec::with_capacity(segment.len());
    let mut rest = segment.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%' {
            let escape = std::str::from_utf8(tail.get(..2)?)
                .ok()
                .filter(|escape| is_hex(escape))?;
            decoded.push(u8::from_str_radix(escape, 16).ok()?);
            rest = &tail[2..];
        } else {
            decoded.push(byte);
            rest = tail;
        }
    }
    String::from_utf8(decoded).ok()
}

fn is_full_sha(sha: &str) -> bool {
    sha.len() == FULL_SHA_LENGTH && is_hex(sha)
}

fn is_abbreviated_sha(sha: &str) -> bool {
    (SHORTEST_ABBREVIATED_SHA..=FULL_SHA_LENGTH).contains(&sha.len()) && is_hex(sha)
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
    without_port(without_user)
}

fn without_port(host_and_port: &str) -> &str {
    match host_and_port.find(']') {
        Some(end) if host_and_port.starts_with('[') => &host_and_port[..=end],
        _ => host_and_port.split(':').next().unwrap_or(host_and_port),
    }
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

    #[test]
    fn a_bitbucket_or_gitlab_archive_is_downloaded_from_the_url_packagist_lists() {
        for url in [
            "https://bitbucket.org/owner/repo/get/d7059e4.zip".to_string(),
            format!("https://bitbucket.org/my_owner/my.repo-x/get/{SHA}.zip"),
            format!("https://gitlab.com/api/v4/projects/12345/repository/archive.zip?sha={SHA}"),
            format!("https://gitlab.com/api/v4/projects/group%2Fsub%2Fproject/repository/archive.zip?sha={SHA}"),
        ] {
            assert_eq!(direct_url(&url), Ok(url.clone()));
        }
    }

    #[test]
    fn a_bitbucket_url_that_breaks_the_strict_pattern_is_unsupported() {
        for url in [
            "https://bitbucket.org/owner/repo/get/d7059e.zip".to_string(),
            format!("https://bitbucket.org/owner/repo/get/{SHA}0.zip"),
            "https://bitbucket.org/owner/repo/get/v1.0.0.zip".to_string(),
            "https://bitbucket.org/owner/repo/get/d7059e4.tar.gz".to_string(),
            "https://bitbucket.org/owner/repo/get/d7059e4.zip?x=1".to_string(),
            "https://bitbucket.org/../repo/get/d7059e4.zip".to_string(),
            "https://bitbucket.org:8443/owner/repo/get/d7059e4.zip".to_string(),
            "https://user@bitbucket.org/owner/repo/get/d7059e4.zip".to_string(),
        ] {
            let failure = rejection(&url);
            assert_eq!(
                failure.code,
                FailureCode::UnsupportedArchiveHost.as_str(),
                "{url}"
            );
            assert_eq!(
                failure.message,
                format!("diffpack can't download this bitbucket.org archive: {url}")
            );
        }
    }

    #[test]
    fn a_gitlab_url_that_breaks_the_strict_pattern_is_unsupported() {
        for url in [
            "https://gitlab.com/api/v4/projects/1/repository/archive.zip?sha=d7059e4".to_string(),
            format!("https://gitlab.com/api/v4/projects/1/repository/archive.zip?sha={SHA}&x=1"),
            format!("https://gitlab.com/api/v4/projects/1/repository/archive.zip?ref={SHA}"),
            "https://gitlab.com/api/v4/projects/1/repository/archive.zip".to_string(),
            format!(
                "https://gitlab.com/api/v4/projects/group/project/repository/archive.zip?sha={SHA}"
            ),
            format!("https://gitlab.com/api/v4/projects/../repository/archive.zip?sha={SHA}"),
            format!("https://gitlab.com/api/v4/projects/%2e%2E/repository/archive.zip?sha={SHA}"),
            format!("https://gitlab.com/api/v4/projects/.%2e/repository/archive.zip?sha={SHA}"),
            format!("https://gitlab.com/api/v4/projects/%2e/repository/archive.zip?sha={SHA}"),
            format!(
                "https://gitlab.com/api/v4/projects/bad%zzescape/repository/archive.zip?sha={SHA}"
            ),
            format!("https://gitlab.com/api/v4/projects/bad%ff/repository/archive.zip?sha={SHA}"),
            format!("https://gitlab.com/api/v4/projects/1/repository/archive.tar.gz?sha={SHA}"),
            format!("https://gitlab.com:443/api/v4/projects/1/repository/archive.zip?sha={SHA}"),
        ] {
            let failure = rejection(&url);
            assert_eq!(
                failure.code,
                FailureCode::UnsupportedArchiveHost.as_str(),
                "{url}"
            );
            assert_eq!(
                failure.message,
                format!("diffpack can't download this gitlab.com archive: {url}")
            );
        }
    }

    #[test]
    fn a_self_hosted_gitlab_is_another_host() {
        let url = format!(
            "https://gitlab.example.com/api/v4/projects/1/repository/archive.zip?sha={SHA}"
        );
        assert_eq!(
            rejection(&url).message,
            format!("diffpack can't download archives from gitlab.example.com yet: {url}")
        );
    }

    #[test]
    fn a_bracketed_ipv6_host_is_named_whole() {
        for url in [
            format!("https://[2001:db8::1]/repos/o/r/zipball/{SHA}"),
            format!("https://[2001:db8::1]:8443/repos/o/r/zipball/{SHA}"),
            format!("https://user@[2001:db8::1]:8443/repos/o/r/zipball/{SHA}"),
        ] {
            assert_eq!(
                rejection(&url).message,
                format!("diffpack can't download archives from [2001:db8::1] yet: {url}")
            );
        }
    }
}
