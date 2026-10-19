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
