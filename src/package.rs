use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Cursor, Read};
use std::rc::Rc;

use flate2::read::GzDecoder;
use js_sys::Uint8Array;
use serde::Deserialize;
use tar::Archive;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Response, Window, WorkerGlobalScope};
use zip::ZipArchive;

use crate::composer;
use crate::failure::Failure;
use crate::fetch_failure::FetchFailure;
use crate::in_flight::InFlight;
use crate::proxy;
use crate::types::{FileMapEntry, FileType};

const COMPOSER: &str = "composer";

#[derive(Deserialize)]
pub struct PyPiResponse {
    pub urls: Vec<PyPiUrl>,
}

#[derive(Deserialize)]
pub struct PyPiUrl {
    pub url: String,
    pub packagetype: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveSource {
    Archive { url: String },
    Listing { url: String },
}

pub fn archive_source(registry: &str, pkg: &str, version: &str) -> Result<ArchiveSource, String> {
    match registry {
        "npm" | "crates" => {
            build_tarball_url(registry, pkg, version).map(|url| ArchiveSource::Archive { url })
        }
        "go" => Ok(ArchiveSource::Archive {
            url: build_go_zip_url(pkg, version),
        }),
        "pypi" => Ok(ArchiveSource::Listing {
            url: format!("https://pypi.org/pypi/{pkg}/{version}/json"),
        }),
        COMPOSER => composer::listing_url(pkg).map(|url| ArchiveSource::Listing { url }),
        _ => Err(unsupported_registry(registry)),
    }
}

pub fn choose_archive(registry: &str, version: &str, listing: &str) -> Result<String, String> {
    choose_archive_url(registry, version, listing).map_err(Failure::into_message)
}

fn choose_archive_url(registry: &str, version: &str, listing: &str) -> Result<String, Failure> {
    match registry {
        COMPOSER => composer::find_dist(version, listing).map(|dist| dist.direct_url),
        "pypi" => choose_pypi_archive(listing).map_err(Failure::Uncoded),
        "npm" | "crates" | "go" => Err(Failure::Uncoded(format!(
            "Registry {registry} serves its archive directly; there is no listing to choose from"
        ))),
        _ => Err(Failure::Uncoded(unsupported_registry(registry))),
    }
}

fn choose_pypi_archive(listing: &str) -> Result<String, String> {
    let metadata: PyPiResponse = serde_json::from_str(listing)
        .map_err(|err| format!("Failed to parse PyPI metadata: {err}"))?;
    select_pypi_sdist_url(&metadata.urls)
}

pub fn unpack_archive(
    registry: &str,
    pkg: &str,
    version: &str,
    bytes: &[u8],
) -> Result<HashMap<String, FileMapEntry>, String> {
    match registry {
        "go" => {
            let files = extract_archive_bytes_with(bytes, false)?;
            Ok(strip_go_module_root(files, pkg, version))
        }
        "npm" | "crates" | "pypi" | COMPOSER => extract_archive_bytes(bytes),
        _ => Err(unsupported_registry(registry)),
    }
}

fn unsupported_registry(registry: &str) -> String {
    format!("Unsupported registry: {registry}")
}

pub async fn fetch_and_extract_package(
    registry: &str,
    pkg: &str,
    version: &str,
) -> Result<HashMap<String, FileMapEntry>, JsValue> {
    let bytes = match registry {
        COMPOSER => fetch_composer_archive(pkg, version).await?,
        _ => fetch_registry_archive(registry, pkg, version).await?,
    };
    unpack_archive(registry, pkg, version, &bytes).map_err(js_error)
}

async fn fetch_registry_archive(
    registry: &str,
    pkg: &str,
    version: &str,
) -> Result<Vec<u8>, JsValue> {
    let archive_url = match archive_source(registry, pkg, version).map_err(js_error)? {
        ArchiveSource::Archive { url } => url,
        ArchiveSource::Listing { url } => {
            let listing = fetch_bytes(&url).await?;
            let listing = listing_text(registry, &listing).map_err(js_error)?;
            choose_archive_url(registry, version, listing)?
        }
    };
    fetch_bytes(&archive_url).await
}

type ListingRequest = Result<Rc<Vec<u8>>, JsValue>;

thread_local! {
    static LISTINGS: Rc<InFlight<ListingRequest>> = InFlight::new();
}

async fn fetch_shared_listing(url: String) -> ListingRequest {
    let listings = LISTINGS.with(Rc::clone);
    let key = url.clone();
    let request =
        listings.join_or_start(&key, || async move { fetch_bytes(&url).await.map(Rc::new) });
    request.await
}

async fn fetch_composer_archive(pkg: &str, version: &str) -> Result<Vec<u8>, JsValue> {
    let proxy = proxy::required()?;
    let listing = fetch_shared_listing(proxy.listing_url(pkg).map_err(js_error)?).await?;
    let listing = listing_text(COMPOSER, &listing).map_err(js_error)?;
    let dist = composer::find_dist(version, listing)?;
    fetch_bytes(&proxy.archive_url(pkg, &dist.listed_url).map_err(js_error)?).await
}

fn listing_text<'a>(registry: &str, listing: &'a [u8]) -> Result<&'a str, String> {
    std::str::from_utf8(listing)
        .map_err(|err| format!("The {registry} listing is not valid UTF-8: {err}"))
}

fn js_error(err: String) -> JsValue {
    JsValue::from_str(&err)
}

pub fn escape_go_module_path(pkg: &str) -> String {
    go_case_escape(pkg)
}

pub fn escape_go_version(version: &str) -> String {
    go_case_escape(version)
}

fn go_case_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_uppercase() {
            escaped.push('!');
            escaped.push(ch.to_ascii_lowercase());
        } else {
            escaped.push(ch);
        }
    }
    escaped
}

pub fn build_go_zip_url(pkg: &str, version: &str) -> String {
    format!(
        "https://proxy.golang.org/{}/@v/{}.zip",
        escape_go_module_path(pkg),
        escape_go_version(version)
    )
}

pub fn strip_go_module_root(
    files: HashMap<String, FileMapEntry>,
    pkg: &str,
    version: &str,
) -> HashMap<String, FileMapEntry> {
    let prefix = format!("{pkg}@{version}/");
    if !files.keys().any(|path| path.starts_with(&prefix)) {
        return strip_common_root(files);
    }

    let mut stripped = HashMap::new();
    for (path, entry) in files {
        if let Some(rest) = path.strip_prefix(&prefix) {
            if !rest.is_empty() {
                stripped.insert(rest.to_string(), entry);
            }
        }
    }

    ensure_directories(&mut stripped);
    stripped
}

pub fn build_tarball_url(registry: &str, pkg: &str, version: &str) -> Result<String, String> {
    match registry {
        "npm" => {
            let unscoped = pkg.split('/').nth(1).unwrap_or(pkg);
            Ok(format!(
                "https://registry.npmjs.org/{pkg}/-/{unscoped}-{version}.tgz"
            ))
        }
        "crates" => Ok(format!(
            "https://static.crates.io/crates/{pkg}/{pkg}-{version}.crate"
        )),
        _ => Err(format!("Unsupported registry: {registry}")),
    }
}

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, JsValue> {
    let response = fetch_response(url).await?;
    if !response.ok() {
        return Err(failed_answer(url, &response).await.into());
    }
    Ok(read_body(url, &response).await?)
}

const PROXY_ERROR_HEADER: &str = "Diffpack-Proxy-Error";

async fn failed_answer(url: &str, response: &Response) -> FetchFailure {
    let proxy_error_body = if is_proxy_answer(response) {
        Some(read_body(url, response).await.unwrap_or_default())
    } else {
        None
    };
    FetchFailure::from_answer(url, response.status(), proxy_error_body.as_deref())
}

fn is_proxy_answer(response: &Response) -> bool {
    response.headers().has(PROXY_ERROR_HEADER).unwrap_or(false)
}

async fn fetch_response(url: &str) -> Result<Response, JsValue> {
    let answer = JsFuture::from(fetch_with_str(url)?)
        .await
        .map_err(|_| unanswered(url))?;
    answer.dyn_into()
}

fn unanswered(url: &str) -> FetchFailure {
    if proxy::serves(url) {
        FetchFailure::unanswered_by_the_proxy(url)
    } else {
        FetchFailure::unanswered(url)
    }
}

async fn read_body(url: &str, response: &Response) -> Result<Vec<u8>, FetchFailure> {
    let broken_off = |_| FetchFailure::broken_off(url, response.status());
    let body = response.array_buffer().map_err(broken_off)?;
    let buffer = JsFuture::from(body).await.map_err(broken_off)?;
    let array = Uint8Array::new(&buffer);
    let mut bytes = vec![0; array.length() as usize];
    array.copy_to(&mut bytes);
    Ok(bytes)
}

fn fetch_with_str(url: &str) -> Result<js_sys::Promise, FetchFailure> {
    let global = js_sys::global();
    if let Some(window) = global.dyn_ref::<Window>() {
        Ok(window.fetch_with_str(url))
    } else if let Some(worker) = global.dyn_ref::<WorkerGlobalScope>() {
        Ok(worker.fetch_with_str(url))
    } else {
        Err(FetchFailure::without_fetch(url))
    }
}

pub fn select_pypi_sdist_url(urls: &[PyPiUrl]) -> Result<String, String> {
    let mut sdist_supported = None;
    let mut sdist_fallback = None;
    let mut wheel_supported = None;
    let mut wheel_fallback = None;

    for entry in urls {
        if entry.packagetype == "sdist" {
            if is_supported_archive_url(&entry.url) {
                if sdist_supported.is_none() {
                    sdist_supported = Some(entry.url.clone());
                }
            } else if sdist_fallback.is_none() {
                sdist_fallback = Some(entry.url.clone());
            }
        } else if entry.packagetype == "bdist_wheel" {
            if is_supported_archive_url(&entry.url) {
                if wheel_supported.is_none() {
                    wheel_supported = Some(entry.url.clone());
                }
            } else if wheel_fallback.is_none() {
                wheel_fallback = Some(entry.url.clone());
            }
        }
    }

    sdist_supported
        .or(wheel_supported)
        .or(sdist_fallback)
        .or(wheel_fallback)
        .ok_or_else(|| "No downloadable artifacts found for PyPI package".to_string())
}

fn is_supported_archive_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
        || lower.ends_with(".tar")
        || lower.ends_with(".zip")
        || lower.ends_with(".whl")
}

pub fn extract_archive_bytes(bytes: &[u8]) -> Result<HashMap<String, FileMapEntry>, String> {
    extract_archive_bytes_with(bytes, true)
}

const GUNZIP_READAHEAD: usize = 64 * 1024;

const MAX_DEFLATE_RATIO: usize = 1032;

fn extract_archive_bytes_with(
    bytes: &[u8],
    strip_root: bool,
) -> Result<HashMap<String, FileMapEntry>, String> {
    if is_gzip(bytes) {
        let size_cap = bytes.len().saturating_mul(MAX_DEFLATE_RATIO);
        let mut decoder = BufReader::with_capacity(GUNZIP_READAHEAD, GzDecoder::new(bytes));
        let head = decoder
            .fill_buf()
            .map_err(|err| format!("Gzip decompression failed: {err}"))?;
        if is_zip(head) {
            let mut decompressed = Vec::with_capacity(gzip_uncompressed_size(bytes).min(size_cap));
            decoder
                .read_to_end(&mut decompressed)
                .map_err(|err| format!("Gzip decompression failed: {err}"))?;
            return parse_zip_bytes(&decompressed, strip_root);
        }
        return parse_tar(decoder, size_cap, strip_root);
    }

    if is_zip(bytes) {
        return parse_zip_bytes(bytes, strip_root);
    }

    parse_tar(bytes, bytes.len(), strip_root)
}

fn gzip_uncompressed_size(bytes: &[u8]) -> usize {
    match bytes {
        [.., a, b, c, d] => u32::from_le_bytes([*a, *b, *c, *d]) as usize,
        _ => 0,
    }
}

fn entry_buffer(declared: u64, cap: usize) -> Vec<u8> {
    let declared = usize::try_from(declared).unwrap_or(usize::MAX);
    Vec::with_capacity(declared.min(cap))
}

fn content_from_bytes(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes)
        .unwrap_or_else(|err| String::from_utf8_lossy(err.as_bytes()).into_owned())
}

fn file_entry(content: String) -> FileMapEntry {
    FileMapEntry {
        file_type: FileType::File,
        content,
    }
}

fn directory_entry() -> FileMapEntry {
    FileMapEntry {
        file_type: FileType::Directory,
        content: String::new(),
    }
}

fn parse_tar<R: Read>(
    reader: R,
    size_cap: usize,
    strip_root: bool,
) -> Result<HashMap<String, FileMapEntry>, String> {
    let mut archive = Archive::new(reader);
    let mut files = HashMap::new();
    let entries = archive
        .entries()
        .map_err(|err| format!("Tar parsing failed: {err}"))?;

    for entry in entries {
        let mut entry = entry.map_err(|err| format!("Tar entry error: {err}"))?;
        let entry_type = entry.header().entry_type();
        let path = entry
            .path()
            .map_err(|err| format!("Tar path error: {err}"))?;
        let normalized = normalize_path(&path.to_string_lossy(), entry_type.is_dir());
        if normalized.is_empty() {
            continue;
        }

        if entry_type.is_dir() {
            files.insert(normalized, directory_entry());
        } else if entry_type.is_file() {
            let mut contents = entry_buffer(entry.size(), size_cap);
            entry
                .read_to_end(&mut contents)
                .map_err(|err| format!("Tar read failed: {err}"))?;
            files.insert(normalized, file_entry(content_from_bytes(contents)));
        }
    }

    ensure_directories(&mut files);
    Ok(if strip_root {
        strip_common_root(files)
    } else {
        files
    })
}

fn parse_zip_bytes(
    bytes: &[u8],
    strip_root: bool,
) -> Result<HashMap<String, FileMapEntry>, String> {
    let size_cap = bytes.len().saturating_mul(MAX_DEFLATE_RATIO);
    let reader = Cursor::new(bytes);
    let mut archive =
        ZipArchive::new(reader).map_err(|err| format!("Zip parsing failed: {err}"))?;
    let mut files = HashMap::new();

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|err| format!("Zip entry error: {err}"))?;
        let name = entry
            .name()
            .map_err(|err| format!("Zip entry name error: {err}"))?;
        let normalized = normalize_path(&name, entry.is_dir());
        if normalized.is_empty() {
            continue;
        }

        if entry.is_dir() {
            files.insert(normalized, directory_entry());
        } else {
            let mut contents = entry_buffer(entry.size(), size_cap);
            entry
                .read_to_end(&mut contents)
                .map_err(|err| format!("Zip read failed: {err}"))?;
            files.insert(normalized, file_entry(content_from_bytes(contents)));
        }
    }

    ensure_directories(&mut files);
    Ok(if strip_root {
        strip_common_root(files)
    } else {
        files
    })
}

fn normalize_path(path: &str, is_directory: bool) -> String {
    let normalized_path = path.replace('\\', "/");
    let mut trimmed = normalized_path.as_str();
    while trimmed.starts_with("./") {
        trimmed = &trimmed[2..];
    }
    let trimmed = trimmed.trim_start_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        return String::new();
    }
    let normalized = if is_directory {
        trimmed.trim_end_matches('/').to_string()
    } else {
        trimmed.to_string()
    };
    normalized
}

fn is_gzip(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b
}

fn is_zip(bytes: &[u8]) -> bool {
    bytes.len() >= 4
        && ((bytes[0] == 0x50 && bytes[1] == 0x4b && bytes[2] == 0x03 && bytes[3] == 0x04)
            || (bytes[0] == 0x50 && bytes[1] == 0x4b && bytes[2] == 0x05 && bytes[3] == 0x06)
            || (bytes[0] == 0x50 && bytes[1] == 0x4b && bytes[2] == 0x07 && bytes[3] == 0x08))
}

fn ensure_directories(files: &mut HashMap<String, FileMapEntry>) {
    let paths: Vec<String> = files.keys().cloned().collect();
    for path in paths {
        let mut current = String::new();
        for part in path.split('/').take_while(|part| !part.is_empty()) {
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(part);
            if !files.contains_key(&current) {
                files.insert(current.clone(), directory_entry());
            }
        }
    }
}

fn strip_common_root(mut files: HashMap<String, FileMapEntry>) -> HashMap<String, FileMapEntry> {
    let paths: Vec<String> = files.keys().cloned().collect();
    if paths.is_empty() {
        return files;
    }

    let mut top_level = HashSet::new();
    for path in &paths {
        if let Some(first) = path.split('/').next() {
            if !first.is_empty() {
                top_level.insert(first.to_string());
            }
        }
    }

    if top_level.len() != 1 {
        return files;
    }

    let root = top_level.into_iter().next().unwrap();
    match files.get(&root) {
        Some(entry) if matches!(entry.file_type, FileType::Directory) => {}
        _ => return files,
    }

    let prefix = format!("{root}/");
    let mut new_files = HashMap::new();
    let mut has_files = false;
    for path in paths {
        if path == root {
            continue;
        }
        if let Some(new_path) = path.strip_prefix(&prefix) {
            if !new_path.is_empty() {
                if let Some(entry) = files.remove(&path) {
                    new_files.insert(new_path.to_string(), entry);
                    has_files = true;
                }
            }
        }
    }

    if has_files {
        new_files
    } else {
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::io::Write;

    use flate2::write::GzEncoder;
    use flate2::Compression;

    struct CountingAllocator;

    thread_local! {
        static REQUESTED: Cell<usize> = const { Cell::new(0) };
    }

    fn record(bytes: usize) {
        let _ = REQUESTED.try_with(|counter| counter.set(counter.get() + bytes));
    }

    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            record(layout.size());
            System.alloc(layout)
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            System.dealloc(ptr, layout)
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            record(new_size);
            System.realloc(ptr, layout, new_size)
        }
    }

    #[global_allocator]
    static ALLOCATOR: CountingAllocator = CountingAllocator;

    fn bytes_requested_during<T>(f: impl FnOnce() -> T) -> (T, usize) {
        let before = REQUESTED.with(Cell::get);
        let out = f();
        let after = REQUESTED.with(Cell::get);
        (out, after - before)
    }

    fn text(seed: usize, bytes: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes + 64);
        let mut line = 0;
        while out.len() < bytes {
            let _ = writeln!(
                out,
                "let value_{line} = compute({seed}, {});",
                line * 7 % 13
            );
            line += 1;
        }
        out.truncate(bytes);
        out
    }

    fn package(count: usize, size: usize) -> Vec<(String, Vec<u8>)> {
        (0..count)
            .map(|i| (format!("pkg-1.0.0/src/file_{i}.rs"), text(i, size)))
            .collect()
    }

    fn tar_bytes(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, path, bytes.as_slice())
                .unwrap();
        }
        builder.into_inner().unwrap()
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn zip_bytes(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (path, bytes) in entries {
            writer.start_file(path.as_str(), options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn content_bytes(entries: &[(String, Vec<u8>)]) -> usize {
        entries.iter().map(|(_, bytes)| bytes.len()).sum()
    }

    fn expected_content(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    fn assert_extracts_exactly(archive: &[u8], entries: &[(String, Vec<u8>)]) {
        let files = extract_archive_bytes(archive).unwrap();
        for (path, bytes) in entries {
            let stripped = path.strip_prefix("pkg-1.0.0/").unwrap();
            let entry = files
                .get(stripped)
                .unwrap_or_else(|| panic!("{stripped} missing from {:?}", files.keys()));
            assert!(matches!(entry.file_type, FileType::File));
            assert_eq!(entry.content, expected_content(bytes), "{stripped}");
        }
        let file_count = files
            .values()
            .filter(|entry| matches!(entry.file_type, FileType::File))
            .count();
        assert_eq!(file_count, entries.len());
    }

    fn mixed_entries() -> Vec<(String, Vec<u8>)> {
        vec![
            ("pkg-1.0.0/src/lib.rs".to_string(), text(1, 3000)),
            (
                "pkg-1.0.0/README.md".to_string(),
                "# héllo wörld ✓\n".as_bytes().to_vec(),
            ),
            (
                "pkg-1.0.0/data.bin".to_string(),
                vec![0x66, 0x6f, 0xff, 0xfe, 0x6f, 0x80, 0x0a],
            ),
            ("pkg-1.0.0/empty".to_string(), Vec::new()),
            ("pkg-1.0.0/src/exact_block.rs".to_string(), text(2, 512)),
            (
                "pkg-1.0.0/src/truncated_utf8.rs".to_string(),
                "abc€".as_bytes()[..5].to_vec(),
            ),
        ]
    }

    #[test]
    fn a_gzipped_tar_extracts_every_file_byte_for_byte() {
        let entries = mixed_entries();
        assert_extracts_exactly(&gzip(&tar_bytes(&entries)), &entries);
    }

    #[test]
    fn a_bare_tar_extracts_every_file_byte_for_byte() {
        let entries = mixed_entries();
        assert_extracts_exactly(&tar_bytes(&entries), &entries);
    }

    #[test]
    fn a_zip_extracts_every_file_byte_for_byte() {
        let entries = mixed_entries();
        assert_extracts_exactly(&zip_bytes(&entries), &entries);
    }

    #[test]
    fn a_gzipped_zip_extracts_every_file_byte_for_byte() {
        let entries = mixed_entries();
        assert_extracts_exactly(&gzip(&zip_bytes(&entries)), &entries);
    }

    #[test]
    fn invalid_utf8_is_rendered_lossily_not_dropped() {
        let entries = mixed_entries();
        let files = extract_archive_bytes(&gzip(&tar_bytes(&entries))).unwrap();
        assert_eq!(files["data.bin"].content, "fo\u{FFFD}\u{FFFD}o\u{FFFD}\n");
        assert_eq!(files["src/truncated_utf8.rs"].content, "abc\u{FFFD}");
    }

    #[test]
    fn a_corrupt_gzip_stream_is_an_error_not_a_partial_package() {
        let mut archive = gzip(&tar_bytes(&mixed_entries()));
        let cut = archive.len() / 2;
        archive.truncate(cut);
        assert!(extract_archive_bytes(&archive).is_err());
    }

    const COPIES_ALLOWED: usize = 2;

    fn assert_within_copy_budget(archive: &[u8], entries: &[(String, Vec<u8>)], shape: &str) {
        let (files, requested) = bytes_requested_during(|| extract_archive_bytes(archive).unwrap());
        let content = content_bytes(entries);
        assert!(
            files.len() > entries.len(),
            "directories are in the map too"
        );
        assert!(
            requested < COPIES_ALLOWED * content,
            "{shape}: extracting {content} bytes of content requested {requested} bytes \
             from the allocator ({:.2} copies); the budget is {COPIES_ALLOWED}",
            requested as f64 / content as f64
        );
    }

    #[test]
    fn extracting_a_gzipped_tar_copies_each_byte_of_content_about_once() {
        let entries = package(48, 96 * 1024);
        assert_within_copy_budget(&gzip(&tar_bytes(&entries)), &entries, "gzipped tar");
    }

    #[test]
    fn extracting_a_bare_tar_copies_each_byte_of_content_about_once() {
        let entries = package(48, 96 * 1024);
        assert_within_copy_budget(&tar_bytes(&entries), &entries, "bare tar");
    }

    #[test]
    fn extracting_a_zip_copies_each_byte_of_content_about_once() {
        let entries = package(48, 96 * 1024);
        assert_within_copy_budget(&zip_bytes(&entries), &entries, "zip");
    }

    #[test]
    fn a_go_module_path_escapes_every_uppercase_letter() {
        assert_eq!(
            escape_go_module_path("github.com/Masterminds/semver"),
            "github.com/!masterminds/semver"
        );
        assert_eq!(
            escape_go_module_path("github.com/sirupsen/logrus"),
            "github.com/sirupsen/logrus"
        );
        assert_eq!(escape_go_module_path("ABC"), "!a!b!c");
        assert_eq!(escape_go_module_path(""), "");
    }

    #[test]
    fn go_module_escaping_leaves_non_ascii_alone() {
        assert_eq!(
            escape_go_module_path("gopkg.in/Ünicode"),
            "gopkg.in/Ünicode"
        );
    }

    #[test]
    fn a_go_zip_url_carries_the_escaped_path_and_the_version() {
        assert_eq!(
            build_go_zip_url("github.com/Masterminds/semver", "v3.2.1"),
            "https://proxy.golang.org/github.com/!masterminds/semver/@v/v3.2.1.zip"
        );
    }

    #[test]
    fn a_go_version_escapes_every_uppercase_letter() {
        assert_eq!(escape_go_version("v1.0.0-RC1"), "v1.0.0-!r!c1");
        assert_eq!(escape_go_version("v3.2.1"), "v3.2.1");
        assert_eq!(
            escape_go_version("v0.0.0-20240101000000-AbCdEf123456"),
            "v0.0.0-20240101000000-!ab!cd!ef123456"
        );
        assert_eq!(escape_go_version(""), "");
    }

    #[test]
    fn a_go_zip_url_escapes_the_uppercase_in_its_version_too() {
        assert_eq!(
            build_go_zip_url("github.com/Masterminds/semver", "v1.0.0-RC1"),
            "https://proxy.golang.org/github.com/!masterminds/semver/@v/v1.0.0-!r!c1.zip"
        );
    }

    #[test]
    fn an_npm_tarball_url_uses_the_unscoped_name_for_the_file() {
        assert_eq!(
            build_tarball_url("npm", "left-pad", "1.3.0").unwrap(),
            "https://registry.npmjs.org/left-pad/-/left-pad-1.3.0.tgz"
        );
        assert_eq!(
            build_tarball_url("npm", "@types/node", "20.1.0").unwrap(),
            "https://registry.npmjs.org/@types/node/-/node-20.1.0.tgz"
        );
    }

    #[test]
    fn a_crates_tarball_url_repeats_the_name_in_the_file() {
        assert_eq!(
            build_tarball_url("crates", "serde", "1.0.200").unwrap(),
            "https://static.crates.io/crates/serde/serde-1.0.200.crate"
        );
    }

    #[test]
    fn an_unknown_registry_has_no_tarball_url() {
        assert_eq!(
            build_tarball_url("maven", "guava", "33.0.0").unwrap_err(),
            "Unsupported registry: maven"
        );
    }

    fn archive(url: &str) -> ArchiveSource {
        ArchiveSource::Archive {
            url: url.to_string(),
        }
    }

    #[test]
    fn npm_crates_and_go_serve_the_archive_itself() {
        assert_eq!(
            archive_source("npm", "left-pad", "1.3.0").unwrap(),
            archive("https://registry.npmjs.org/left-pad/-/left-pad-1.3.0.tgz")
        );
        assert_eq!(
            archive_source("crates", "serde", "1.0.200").unwrap(),
            archive("https://static.crates.io/crates/serde/serde-1.0.200.crate")
        );
    }

    #[test]
    fn a_scoped_npm_archive_drops_the_scope_from_the_file_name() {
        assert_eq!(
            archive_source("npm", "@types/node", "20.1.0").unwrap(),
            archive("https://registry.npmjs.org/@types/node/-/node-20.1.0.tgz")
        );
    }

    #[test]
    fn a_go_archive_escapes_the_uppercase_in_its_module_path() {
        assert_eq!(
            archive_source("go", "github.com/Masterminds/semver", "v3.2.1").unwrap(),
            archive("https://proxy.golang.org/github.com/!masterminds/semver/@v/v3.2.1.zip")
        );
    }

    #[test]
    fn a_go_archive_escapes_the_uppercase_in_its_version() {
        assert_eq!(
            archive_source("go", "github.com/Masterminds/semver", "v1.0.0-RC1").unwrap(),
            archive("https://proxy.golang.org/github.com/!masterminds/semver/@v/v1.0.0-!r!c1.zip")
        );
    }

    #[test]
    fn pypi_is_found_through_its_metadata_listing() {
        match archive_source("pypi", "requests", "2.32.3").unwrap() {
            ArchiveSource::Listing { url } => {
                assert!(url.starts_with("https://pypi.org/"), "{url}");
                assert!(url.ends_with("/requests/2.32.3/json"), "{url}");
            }
            other => panic!("expected a listing, got {other:?}"),
        }
    }

    fn listing(url: &str) -> ArchiveSource {
        ArchiveSource::Listing {
            url: url.to_string(),
        }
    }

    #[test]
    fn composer_is_found_through_the_packagist_listing_of_the_whole_package() {
        assert_eq!(
            archive_source("composer", "monolog/monolog", "3.12.1").unwrap(),
            listing("https://repo.packagist.org/p2/monolog/monolog.json")
        );
        assert_eq!(
            archive_source("composer", "symfony/password-hasher", "v8.0.0").unwrap(),
            listing("https://repo.packagist.org/p2/symfony/password-hasher.json")
        );
    }

    #[test]
    fn a_composer_name_is_looked_up_in_lowercase() {
        assert_eq!(
            archive_source("composer", "Monolog/Monolog", "3.12.1").unwrap(),
            listing("https://repo.packagist.org/p2/monolog/monolog.json")
        );
    }

    #[test]
    fn every_name_composer_allows_has_a_listing() {
        for name in [
            "a/b",
            "vendor0/package9",
            "my_vendor/my_package",
            "my.vendor/my.package",
            "my-vendor/my-package",
            "vendor/double--dash",
            "v-e_n.d/p_a.c-k--g",
        ] {
            assert!(archive_source("composer", name, "1.0.0").is_ok(), "{name}");
        }
    }

    #[test]
    fn a_name_composer_rejects_has_no_listing() {
        for name in [
            "",
            "monolog",
            "/monolog",
            "monolog/",
            "a/b/c",
            "-vendor/package",
            "vendor-/package",
            "vendor/_package",
            "vendor/package.",
            "ven--dor/package",
            "vendor/pack---age",
            "vendor/pack_-age",
            "vendor/pack..age",
            "ven dor/package",
            "vendor/pack%2fage",
            "../../evil/package",
            "vendor/package?x=1",
        ] {
            let err = archive_source("composer", name, "1.0.0").unwrap_err();
            assert_eq!(err, format!("Invalid Composer package name: {name}"));
        }
    }

    #[test]
    fn an_unknown_registry_has_no_archive_source() {
        assert_eq!(
            archive_source("maven", "guava", "33.0.0").unwrap_err(),
            "Unsupported registry: maven"
        );
    }

    #[test]
    fn a_pypi_listing_yields_its_preferred_archive() {
        let listing = r#"{
            "info": {"name": "x"},
            "urls": [
                {"packagetype": "bdist_wheel", "url": "https://files/x-1.0-py3-none-any.whl"},
                {"packagetype": "sdist", "url": "https://files/x-1.0.tar.gz"}
            ]
        }"#;
        assert_eq!(
            choose_archive("pypi", "1.0", listing).unwrap(),
            "https://files/x-1.0.tar.gz"
        );
    }

    #[test]
    fn a_pypi_listing_that_is_malformed_or_empty_is_an_error() {
        assert!(choose_archive("pypi", "1.0", "not json")
            .unwrap_err()
            .starts_with("Failed to parse PyPI metadata: "));
        assert_eq!(
            choose_archive("pypi", "1.0", r#"{"urls": []}"#).unwrap_err(),
            "No downloadable artifacts found for PyPI package"
        );
    }

    #[test]
    fn a_listing_is_read_as_text() {
        assert_eq!(
            listing_text("pypi", br#"{"urls": []}"#),
            Ok(r#"{"urls": []}"#)
        );
    }

    #[test]
    fn a_listing_that_is_not_text_names_the_registry_it_came_from() {
        let err = listing_text("composer", b"\xff\xfe").unwrap_err();
        assert!(
            err.starts_with("The composer listing is not valid UTF-8: "),
            "{err}"
        );
        assert!(!err.contains("PyPI"), "{err}");
    }

    #[test]
    fn a_composer_listing_yields_the_direct_archive_of_the_asked_version() {
        let listing = r#"{
            "minified": "composer/2.0",
            "packages": {"o/r": [
                {"version": "v2.0.0", "dist": {"url": "https://api.github.com/repos/o/r/zipball/2222222222222222222222222222222222222222"}},
                {"version": "v1.0.0", "dist": {"url": "https://api.github.com/repos/o/r/zipball/1111111111111111111111111111111111111111"}}
            ]}
        }"#;
        assert_eq!(
            choose_archive("composer", "1.0.0", listing).unwrap(),
            "https://codeload.github.com/o/r/legacy.zip/1111111111111111111111111111111111111111"
        );
        assert_eq!(
            choose_archive("composer", "3.0.0", listing).unwrap_err(),
            "Packagist lists no version 3.0.0 of o/r"
        );
    }

    #[test]
    fn a_registry_without_a_listing_cannot_choose_from_one() {
        for registry in ["npm", "crates", "go"] {
            let err = choose_archive(registry, "1.0", r#"{"urls": []}"#).unwrap_err();
            assert!(err.contains("no listing"), "{registry}: {err}");
        }
        assert_eq!(
            choose_archive("maven", "1.0", "{}").unwrap_err(),
            "Unsupported registry: maven"
        );
    }

    #[test]
    fn a_go_module_zip_unpacks_without_its_versioned_prefix() {
        let entries = vec![
            (
                "github.com/x/y@v1.2.3/go.mod".to_string(),
                b"module github.com/x/y\n".to_vec(),
            ),
            (
                "github.com/x/y@v1.2.3/src/lib.go".to_string(),
                b"package y\n".to_vec(),
            ),
        ];
        let zip = zip_bytes(&entries);

        let files = unpack_archive("go", "github.com/x/y", "v1.2.3", &zip).unwrap();
        assert_eq!(sorted_keys(&files), ["go.mod", "src", "src/lib.go"]);
        assert_eq!(files["src/lib.go"].content, "package y\n");

        let composed = strip_go_module_root(
            extract_archive_bytes(&zip).unwrap(),
            "github.com/x/y",
            "v1.2.3",
        );
        assert_eq!(
            sorted_keys(&composed),
            [
                "y@v1.2.3",
                "y@v1.2.3/go.mod",
                "y@v1.2.3/src",
                "y@v1.2.3/src/lib.go"
            ],
            "the public helpers composed are what unpack_archive replaces"
        );
    }

    #[test]
    fn every_other_registry_unpacks_with_the_single_root_stripped() {
        let entries = mixed_entries();
        let archive = gzip(&tar_bytes(&entries));
        let expected = sorted_keys(&extract_archive_bytes(&archive).unwrap());
        for registry in ["npm", "crates", "pypi"] {
            let files = unpack_archive(registry, "pkg", "1.0.0", &archive).unwrap();
            assert_eq!(sorted_keys(&files), expected, "{registry}");
        }
    }

    #[test]
    fn a_composer_zip_unpacks_without_the_folder_its_host_wraps_it_in() {
        let archive = zip_bytes(&[
            (
                "Seldaek-monolog-d7059e4/composer.json".to_string(),
                b"{}".to_vec(),
            ),
            (
                "Seldaek-monolog-d7059e4/src/Monolog/Logger.php".to_string(),
                b"<?php\n".to_vec(),
            ),
        ]);

        let files = unpack_archive("composer", "monolog/monolog", "3.12.1", &archive).unwrap();

        assert_eq!(
            sorted_keys(&files),
            [
                "composer.json",
                "src",
                "src/Monolog",
                "src/Monolog/Logger.php"
            ]
        );
        assert_eq!(files["src/Monolog/Logger.php"].content, "<?php\n");
    }

    #[test]
    fn an_unknown_registry_cannot_unpack() {
        let archive = gzip(&tar_bytes(&mixed_entries()));
        assert_eq!(
            unpack_archive("maven", "guava", "33.0.0", &archive).unwrap_err(),
            "Unsupported registry: maven"
        );
    }

    fn pypi(packagetype: &str, url: &str) -> PyPiUrl {
        PyPiUrl {
            url: url.to_string(),
            packagetype: packagetype.to_string(),
        }
    }

    #[test]
    fn every_shape_the_extractor_understands_is_a_supported_archive_url() {
        for url in [
            "https://x/a.tar.gz",
            "https://x/a.TGZ",
            "https://x/a.tar",
            "https://x/a.zip",
            "https://x/a.whl",
        ] {
            assert!(is_supported_archive_url(url), "{url}");
        }
        for url in [
            "https://x/a.tar.bz2",
            "https://x/a.exe",
            "https://x/a.egg",
            "https://x/a",
        ] {
            assert!(!is_supported_archive_url(url), "{url}");
        }
    }

    #[test]
    fn a_supported_sdist_outranks_everything_else() {
        let urls = [
            pypi("bdist_wheel", "https://x/a.whl"),
            pypi("sdist", "https://x/a.tar.bz2"),
            pypi("sdist", "https://x/a.tar.gz"),
        ];
        assert_eq!(select_pypi_sdist_url(&urls).unwrap(), "https://x/a.tar.gz");
    }

    #[test]
    fn a_supported_wheel_beats_an_sdist_the_extractor_cannot_open() {
        let urls = [
            pypi("sdist", "https://x/a.tar.bz2"),
            pypi("bdist_wheel", "https://x/a.whl"),
        ];
        assert_eq!(select_pypi_sdist_url(&urls).unwrap(), "https://x/a.whl");
    }

    #[test]
    fn an_unsupported_sdist_is_the_fallback_before_an_unsupported_wheel() {
        let urls = [
            pypi("bdist_wheel", "https://x/a.egg"),
            pypi("sdist", "https://x/a.tar.bz2"),
        ];
        assert_eq!(select_pypi_sdist_url(&urls).unwrap(), "https://x/a.tar.bz2");
    }

    #[test]
    fn the_first_candidate_of_a_rank_wins() {
        let urls = [
            pypi("sdist", "https://x/first.tar.gz"),
            pypi("sdist", "https://x/second.tar.gz"),
        ];
        assert_eq!(
            select_pypi_sdist_url(&urls).unwrap(),
            "https://x/first.tar.gz"
        );
    }

    #[test]
    fn a_release_with_no_usable_artifact_is_an_error() {
        assert_eq!(
            select_pypi_sdist_url(&[]).unwrap_err(),
            "No downloadable artifacts found for PyPI package"
        );
        assert!(select_pypi_sdist_url(&[pypi("bdist_egg", "https://x/a.zip")]).is_err());
    }

    #[test]
    fn gzip_is_recognised_by_its_two_magic_bytes() {
        assert!(is_gzip(&[0x1f, 0x8b, 0x08, 0x00]));
        assert!(!is_gzip(&[0x1f]));
        assert!(!is_gzip(&[]));
        assert!(!is_gzip(&[0x1f, 0x8c]));
    }

    #[test]
    fn zip_is_recognised_by_any_of_its_signatures() {
        assert!(is_zip(b"PK\x03\x04"));
        assert!(is_zip(b"PK\x05\x06"));
        assert!(is_zip(b"PK\x07\x08"));
        assert!(!is_zip(b"PK\x01\x02"));
        assert!(!is_zip(b"PK\x03"));
        assert!(!is_zip(&[]));
    }

    #[test]
    fn the_gzip_trailer_is_read_as_the_uncompressed_length() {
        let payload = text(0, 5000);
        assert_eq!(gzip_uncompressed_size(&gzip(&payload)), payload.len());
        assert_eq!(gzip_uncompressed_size(&[0x1f, 0x8b, 0x08]), 0);
    }

    #[test]
    fn an_entry_buffer_is_sized_from_its_header_but_capped_by_the_archive() {
        assert_eq!(entry_buffer(1024, 4096).capacity(), 1024);
        assert_eq!(entry_buffer(u64::MAX, 4096).capacity(), 4096);
        assert_eq!(entry_buffer(0, 4096).capacity(), 0);
    }

    #[test]
    fn utf8_content_is_taken_as_is_and_invalid_bytes_are_rendered_lossily() {
        assert_eq!(content_from_bytes("héllo".as_bytes().to_vec()), "héllo");
        assert_eq!(content_from_bytes(vec![0x66, 0xff, 0x6f]), "f\u{FFFD}o");
        assert_eq!(content_from_bytes(Vec::new()), "");
    }

    #[test]
    fn a_file_entry_keeps_its_content_and_a_directory_entry_has_none() {
        let file = file_entry("body".to_string());
        assert!(matches!(file.file_type, FileType::File));
        assert_eq!(file.content, "body");

        let dir = directory_entry();
        assert!(matches!(dir.file_type, FileType::Directory));
        assert_eq!(dir.content, "");
    }

    #[test]
    fn a_path_is_normalised_to_forward_slashes_without_a_leading_dot_or_root() {
        assert_eq!(normalize_path("src\\lib.rs", false), "src/lib.rs");
        assert_eq!(normalize_path("./src/lib.rs", false), "src/lib.rs");
        assert_eq!(normalize_path("././src/lib.rs", false), "src/lib.rs");
        assert_eq!(normalize_path("/src/lib.rs", false), "src/lib.rs");
    }

    #[test]
    fn a_directory_loses_its_trailing_slash_and_a_file_keeps_its_name() {
        assert_eq!(normalize_path("src/", true), "src");
        assert_eq!(normalize_path("src///", true), "src");
        assert_eq!(normalize_path("weird/name/", false), "weird/name/");
    }

    #[test]
    fn a_path_that_names_nothing_normalises_to_the_empty_string() {
        assert_eq!(normalize_path("", false), "");
        assert_eq!(normalize_path(".", false), "");
        assert_eq!(normalize_path("./", true), "");
        assert_eq!(normalize_path("/", true), "");
    }

    fn map(entries: &[(&str, FileMapEntry)]) -> HashMap<String, FileMapEntry> {
        entries
            .iter()
            .map(|(path, entry)| (path.to_string(), entry.clone()))
            .collect()
    }

    fn sorted_keys(files: &HashMap<String, FileMapEntry>) -> Vec<String> {
        let mut keys: Vec<String> = files.keys().cloned().collect();
        keys.sort();
        keys
    }

    #[test]
    fn every_ancestor_of_a_file_becomes_a_directory_entry() {
        let mut files = map(&[("a/b/c/file.rs", file_entry("x".to_string()))]);
        ensure_directories(&mut files);
        assert_eq!(sorted_keys(&files), ["a", "a/b", "a/b/c", "a/b/c/file.rs"]);
        for dir in ["a", "a/b", "a/b/c"] {
            assert!(matches!(files[dir].file_type, FileType::Directory));
        }
    }

    #[test]
    fn ensuring_directories_leaves_existing_entries_alone() {
        let mut files = map(&[
            ("a", directory_entry()),
            ("a/file.rs", file_entry("body".to_string())),
        ]);
        ensure_directories(&mut files);
        assert_eq!(files["a/file.rs"].content, "body");
        assert!(matches!(files["a/file.rs"].file_type, FileType::File));
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn a_single_top_level_directory_is_stripped() {
        let files = map(&[
            ("pkg-1.0.0", directory_entry()),
            ("pkg-1.0.0/src", directory_entry()),
            ("pkg-1.0.0/src/lib.rs", file_entry("x".to_string())),
        ]);
        assert_eq!(
            sorted_keys(&strip_common_root(files)),
            ["src", "src/lib.rs"]
        );
    }

    #[test]
    fn two_top_level_entries_keep_their_root() {
        let files = map(&[
            ("a", directory_entry()),
            ("a/lib.rs", file_entry("x".to_string())),
            ("b", directory_entry()),
        ]);
        assert_eq!(
            sorted_keys(&strip_common_root(files)),
            ["a", "a/lib.rs", "b"]
        );
    }

    #[test]
    fn a_lone_top_level_file_is_not_stripped() {
        let files = map(&[("README.md", file_entry("x".to_string()))]);
        assert_eq!(sorted_keys(&strip_common_root(files)), ["README.md"]);
    }

    #[test]
    fn a_root_with_nothing_under_it_is_kept() {
        let files = map(&[("pkg-1.0.0", directory_entry())]);
        assert_eq!(sorted_keys(&strip_common_root(files)), ["pkg-1.0.0"]);
    }

    #[test]
    fn an_empty_map_survives_root_stripping() {
        assert!(strip_common_root(HashMap::new()).is_empty());
    }

    #[test]
    fn a_go_module_root_is_stripped_prefix_and_all() {
        let files = map(&[
            ("github.com/x/y@v1.2.3", directory_entry()),
            (
                "github.com/x/y@v1.2.3/go.mod",
                file_entry("module x".to_string()),
            ),
            (
                "github.com/x/y@v1.2.3/internal/z.go",
                file_entry("package z".to_string()),
            ),
        ]);
        let stripped = strip_go_module_root(files, "github.com/x/y", "v1.2.3");
        assert_eq!(
            sorted_keys(&stripped),
            ["go.mod", "internal", "internal/z.go"]
        );
        assert!(matches!(
            stripped["internal"].file_type,
            FileType::Directory
        ));
    }

    #[test]
    fn a_zip_without_the_module_prefix_falls_back_to_the_common_root() {
        let files = map(&[
            ("pkg", directory_entry()),
            ("pkg/main.go", file_entry("package main".to_string())),
        ]);
        let stripped = strip_go_module_root(files, "github.com/x/y", "v1.2.3");
        assert_eq!(sorted_keys(&stripped), ["main.go"]);
    }

    #[test]
    fn extraction_can_be_asked_to_keep_the_top_level_directory() {
        let entries = vec![("pkg-1.0.0/src/lib.rs".to_string(), text(1, 128))];
        let files = extract_archive_bytes_with(&tar_bytes(&entries), false).unwrap();
        assert_eq!(
            sorted_keys(&files),
            ["pkg-1.0.0", "pkg-1.0.0/src", "pkg-1.0.0/src/lib.rs"]
        );
    }

    #[test]
    fn a_tar_directory_entry_becomes_a_directory_in_the_map() {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_entry_type(tar::EntryType::Directory);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "pkg-1.0.0/docs/", &[][..])
            .unwrap();

        let mut file_header = tar::Header::new_gnu();
        file_header.set_size(2);
        file_header.set_mode(0o644);
        file_header.set_cksum();
        builder
            .append_data(&mut file_header, "pkg-1.0.0/a.rs", &b"hi"[..])
            .unwrap();

        let files = extract_archive_bytes(&builder.into_inner().unwrap()).unwrap();
        assert!(matches!(files["docs"].file_type, FileType::Directory));
        assert_eq!(files["a.rs"].content, "hi");
    }

    #[test]
    fn a_zip_directory_entry_becomes_a_directory_in_the_map() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.add_directory("pkg-1.0.0/docs/", options).unwrap();
        writer.start_file("pkg-1.0.0/a.rs", options).unwrap();
        writer.write_all(b"hi").unwrap();
        let archive = writer.finish().unwrap().into_inner();

        let files = extract_archive_bytes(&archive).unwrap();
        assert!(matches!(files["docs"].file_type, FileType::Directory));
        assert_eq!(files["a.rs"].content, "hi");
    }

    #[test]
    fn bytes_that_are_neither_gzip_nor_zip_nor_tar_are_an_error() {
        assert!(extract_archive_bytes(b"not an archive at all").is_err());
    }

    #[test]
    fn a_corrupt_zip_central_directory_is_an_error() {
        let mut archive = zip_bytes(&mixed_entries());
        let len = archive.len();
        archive.truncate(len - 8);
        assert!(extract_archive_bytes(&archive).is_err());
    }
}
