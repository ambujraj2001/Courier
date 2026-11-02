//! Dynamic `{{$name}}` variables that produce a fresh value on every use.
//! The word lists below are small, generic and written for Courier.

use chrono::{Duration, Utc};
use rand::Rng;
use rand::seq::SliceRandom;

pub const NAMES: &[&str] = &[
    "$guid",
    "$randomUUID",
    "$timestamp",
    "$isoTimestamp",
    "$randomInt",
    "$randomAlphaNumeric",
    "$randomBoolean",
    "$randomColor",
    "$randomHexColor",
    "$randomEmail",
    "$randomExampleEmail",
    "$randomUserName",
    "$randomFirstName",
    "$randomLastName",
    "$randomFullName",
    "$randomPassword",
    "$randomPhoneNumber",
    "$randomIP",
    "$randomIPV6",
    "$randomMACAddress",
    "$randomUrl",
    "$randomDomainName",
    "$randomUserAgent",
    "$randomCity",
    "$randomCountry",
    "$randomCountryCode",
    "$randomStreetAddress",
    "$randomCompanyName",
    "$randomJobTitle",
    "$randomWord",
    "$randomWords",
    "$randomLoremSentence",
    "$randomLoremParagraph",
    "$randomPrice",
    "$randomCurrencyCode",
    "$randomDatePast",
    "$randomDateFuture",
    "$randomDateRecent",
    "$randomSemver",
    "$randomFileName",
    "$randomMimeType",
    "$randomLatitude",
    "$randomLongitude",
];

const FIRST: &[&str] = &["Ada", "Ben", "Chloe", "Dev", "Elena", "Femi", "Gus", "Hana", "Ivan", "Jo", "Kai", "Lena"];
const LAST: &[&str] = &["Archer", "Brooks", "Chen", "Diaz", "Evans", "Fischer", "Garcia", "Hughes", "Ito", "Kowal"];
const WORDS: &[&str] = &[
    "amber", "brisk", "cobalt", "delta", "ember", "fable", "granite", "harbor", "indigo", "juniper", "kettle",
    "lantern", "meadow", "nimbus", "orbit", "pebble", "quartz", "river", "summit", "timber", "umber", "velvet",
    "willow", "zephyr",
];
const CITIES: &[&str] = &["Lisbon", "Osaka", "Nairobi", "Denver", "Tallinn", "Perth", "Quito", "Bergen", "Pune"];
const COUNTRIES: &[(&str, &str)] =
    &[("Portugal", "PT"), ("Japan", "JP"), ("Kenya", "KE"), ("Canada", "CA"), ("Estonia", "EE"), ("Chile", "CL")];
const STREETS: &[&str] = &["Oak Lane", "Mill Road", "Harbor Street", "Cedar Avenue", "Station Way"];
const COMPANY_SUFFIX: &[&str] = &["Labs", "Works", "Systems", "Group", "Partners"];
const JOBS: &[&str] = &["Engineer", "Designer", "Analyst", "Product Manager", "Architect", "Consultant"];
const COLORS: &[&str] = &["red", "green", "blue", "orange", "purple", "teal", "yellow", "gray"];
const TLDS: &[&str] = &["com", "net", "org", "io", "dev"];
const CURRENCIES: &[&str] = &["USD", "EUR", "GBP", "JPY", "INR", "AUD"];
const MIME: &[&str] = &["application/json", "text/plain", "image/png", "application/pdf", "text/csv"];
const EXT: &[&str] = &["txt", "json", "png", "pdf", "csv"];
const AGENTS: &[&str] = &[
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:128.0) Gecko/20100101 Firefox/128.0",
    "curl/8.5.0",
];

pub fn is_known(name: &str) -> bool {
    NAMES.contains(&name)
}

fn pick<T: Copy>(items: &[T]) -> T {
    *items.choose(&mut rand::thread_rng()).expect("non-empty list")
}

fn alnum(len: usize) -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..len).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect()
}

fn sentence(words: usize) -> String {
    let mut s: String = (0..words).map(|_| pick(WORDS)).collect::<Vec<_>>().join(" ");
    if let Some(first) = s.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    s.push('.');
    s
}

/// Produce a value for a dynamic variable, or `None` if the name is unknown.
pub fn generate(name: &str) -> Option<String> {
    let mut rng = rand::thread_rng();
    let now = Utc::now();
    let value = match name {
        "$guid" | "$randomUUID" => uuid::Uuid::new_v4().to_string(),
        "$timestamp" => now.timestamp().to_string(),
        "$isoTimestamp" => now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "$randomInt" => rng.gen_range(0..=1000).to_string(),
        "$randomAlphaNumeric" => alnum(1),
        "$randomBoolean" => rng.gen_bool(0.5).to_string(),
        "$randomColor" => pick(COLORS).to_string(),
        "$randomHexColor" => format!("#{:06x}", rng.gen_range(0..0x1000000)),
        "$randomEmail" => format!("{}.{}@{}.{}", pick(FIRST), pick(LAST), pick(WORDS), pick(TLDS)).to_lowercase(),
        "$randomExampleEmail" => format!("{}.{}@example.com", pick(FIRST), pick(LAST)).to_lowercase(),
        "$randomUserName" => format!("{}_{}{}", pick(FIRST), pick(LAST), rng.gen_range(1..100)).to_lowercase(),
        "$randomFirstName" => pick(FIRST).to_string(),
        "$randomLastName" => pick(LAST).to_string(),
        "$randomFullName" => format!("{} {}", pick(FIRST), pick(LAST)),
        "$randomPassword" => alnum(15),
        "$randomPhoneNumber" => {
            format!("{}-{}-{}", rng.gen_range(200..999), rng.gen_range(200..999), rng.gen_range(1000..9999))
        }
        "$randomIP" => {
            format!("{}.{}.{}.{}", rng.gen_range(1..255), rng.r#gen::<u8>(), rng.r#gen::<u8>(), rng.gen_range(1..255))
        }
        "$randomIPV6" => (0..8).map(|_| format!("{:x}", rng.r#gen::<u16>())).collect::<Vec<_>>().join(":"),
        "$randomMACAddress" => (0..6).map(|_| format!("{:02x}", rng.r#gen::<u8>())).collect::<Vec<_>>().join(":"),
        "$randomUrl" => format!("https://{}.{}", pick(WORDS), pick(TLDS)),
        "$randomDomainName" => format!("{}-{}.{}", pick(WORDS), pick(WORDS), pick(TLDS)),
        "$randomUserAgent" => pick(AGENTS).to_string(),
        "$randomCity" => pick(CITIES).to_string(),
        "$randomCountry" => pick(COUNTRIES).0.to_string(),
        "$randomCountryCode" => pick(COUNTRIES).1.to_string(),
        "$randomStreetAddress" => format!("{} {}", rng.gen_range(1..9999), pick(STREETS)),
        "$randomCompanyName" => {
            let mut w = pick(WORDS).to_string();
            w[..1].make_ascii_uppercase();
            format!("{w} {}", pick(COMPANY_SUFFIX))
        }
        "$randomJobTitle" => format!("{} {}", pick(&["Senior", "Lead", "Junior", "Principal"]), pick(JOBS)),
        "$randomWord" => pick(WORDS).to_string(),
        "$randomWords" => (0..3).map(|_| pick(WORDS)).collect::<Vec<_>>().join(" "),
        "$randomLoremSentence" => sentence(rng.gen_range(5..10)),
        "$randomLoremParagraph" => (0..3).map(|_| sentence(7)).collect::<Vec<_>>().join(" "),
        "$randomPrice" => format!("{}.{:02}", rng.gen_range(1..1000), rng.gen_range(0..100)),
        "$randomCurrencyCode" => pick(CURRENCIES).to_string(),
        "$randomDatePast" => (now - Duration::days(rng.gen_range(1..365))).to_rfc2822(),
        "$randomDateFuture" => (now + Duration::days(rng.gen_range(1..365))).to_rfc2822(),
        "$randomDateRecent" => (now - Duration::hours(rng.gen_range(1..48))).to_rfc2822(),
        "$randomSemver" => format!("{}.{}.{}", rng.gen_range(0..10), rng.gen_range(0..20), rng.gen_range(0..50)),
        "$randomFileName" => format!("{}.{}", pick(WORDS), pick(EXT)),
        "$randomMimeType" => pick(MIME).to_string(),
        "$randomLatitude" => format!("{:.4}", rng.gen_range(-90.0..90.0)),
        "$randomLongitude" => format!("{:.4}", rng.gen_range(-180.0..180.0)),
        _ => return None,
    };
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_name_generates_a_value() {
        for name in NAMES {
            let v = generate(name).unwrap_or_else(|| panic!("{name} produced nothing"));
            assert!(!v.is_empty(), "{name} was empty");
        }
    }

    #[test]
    fn iso_timestamp_parses() {
        let v = generate("$isoTimestamp").unwrap();
        assert!(chrono::DateTime::parse_from_rfc3339(&v).is_ok());
    }
}
