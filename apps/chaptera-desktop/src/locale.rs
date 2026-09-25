use std::env;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocaleSource {
    Override,
    WindowsUserLocale,
    Lang,
    None,
}

impl LocaleSource {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Override => "override",
            Self::WindowsUserLocale => "windows_user_locale",
            Self::Lang => "lang",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarketProfile {
    Us,
    Uk,
    Russia,
    NeutralEnglish,
}

impl MarketProfile {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Us => "US",
            Self::Uk => "UK",
            Self::Russia => "Russia",
            Self::NeutralEnglish => "NeutralEnglish",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocaleResolution {
    pub(crate) source: LocaleSource,
    pub(crate) raw: Option<String>,
    pub(crate) normalized: String,
    pub(crate) market_profile: MarketProfile,
}

impl LocaleResolution {
    fn from_source(source: LocaleSource, raw: Option<&str>) -> Self {
        let raw = raw.map(str::to_owned);
        let normalized = normalize_locale(raw.as_deref());
        let market_profile = market_profile_from_normalized(&normalized);
        Self {
            source,
            raw,
            normalized,
            market_profile,
        }
    }
}

pub(crate) fn resolve_product_locale() -> LocaleResolution {
    let override_locale = env::var("CHAPTERA_LOCALE").ok();
    let os_locale = platform_user_locale();
    let lang = env::var("LANG").ok();
    resolve_locale_sources(
        override_locale.as_deref(),
        os_locale.as_deref(),
        lang.as_deref(),
    )
}

fn resolve_locale_sources(
    override_locale: Option<&str>,
    os_locale: Option<&str>,
    lang: Option<&str>,
) -> LocaleResolution {
    for (source, candidate) in [
        (LocaleSource::Override, override_locale),
        (LocaleSource::WindowsUserLocale, os_locale),
        (LocaleSource::Lang, lang),
    ] {
        if let Some(value) = candidate.map(str::trim).filter(|value| !value.is_empty()) {
            return LocaleResolution::from_source(source, Some(value));
        }
    }
    LocaleResolution::from_source(LocaleSource::None, None)
}

fn normalize_locale(value: Option<&str>) -> String {
    let mut normalized = value
        .unwrap_or_default()
        .trim()
        .replace('_', "-")
        .to_ascii_lowercase();
    if let Some(index) = normalized.find(|ch| ch == '.' || ch == '@') {
        normalized.truncate(index);
    }
    normalized
}

fn market_profile_from_normalized(normalized: &str) -> MarketProfile {
    match normalized {
        "en-us" => MarketProfile::Us,
        "en-gb" => MarketProfile::Uk,
        "ru-ru" => MarketProfile::Russia,
        _ => MarketProfile::NeutralEnglish,
    }
}

#[cfg(target_os = "windows")]
fn platform_user_locale() -> Option<String> {
    const LOCALE_NAME_MAX_LENGTH: usize = 85;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultLocaleName(locale_name: *mut u16, locale_name_count: i32) -> i32;
    }

    let mut locale = [0_u16; LOCALE_NAME_MAX_LENGTH];
    let count = unsafe {
        GetUserDefaultLocaleName(
            locale.as_mut_ptr(),
            i32::try_from(locale.len()).expect("locale buffer length fits i32"),
        )
    };
    if count <= 1 {
        return None;
    }

    let len_without_nul = usize::try_from(count - 1).ok()?;
    String::from_utf16(locale.get(..len_without_nul)?).ok()
}

#[cfg(not(target_os = "windows"))]
fn platform_user_locale() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_market_mapping_is_bounded_and_fail_closed() {
        for (raw, expected) in [
            ("en-US", MarketProfile::Us),
            ("EN_us", MarketProfile::Us),
            ("en_US.UTF-8", MarketProfile::Us),
            ("en-US@custom", MarketProfile::Us),
            ("en-GB", MarketProfile::Uk),
            ("en_GB.UTF-8", MarketProfile::Uk),
            ("ru-RU", MarketProfile::Russia),
            ("ru_RU.UTF-8", MarketProfile::Russia),
            ("en-CA", MarketProfile::NeutralEnglish),
            ("ru-UA", MarketProfile::NeutralEnglish),
            ("fr-FR", MarketProfile::NeutralEnglish),
            ("en", MarketProfile::NeutralEnglish),
            ("ru", MarketProfile::NeutralEnglish),
            ("en-US-x-private", MarketProfile::NeutralEnglish),
        ] {
            let resolution = resolve_locale_sources(Some(raw), None, None);
            assert_eq!(resolution.market_profile, expected, "{raw}");
        }
    }

    #[test]
    fn source_precedence_is_override_then_os_then_lang() {
        let override_wins =
            resolve_locale_sources(Some("ru-RU"), Some("en-US"), Some("en_GB.UTF-8"));
        assert_eq!(override_wins.source, LocaleSource::Override);
        assert_eq!(override_wins.market_profile, MarketProfile::Russia);

        let os_wins = resolve_locale_sources(None, Some("en-GB"), Some("ru_RU.UTF-8"));
        assert_eq!(os_wins.source, LocaleSource::WindowsUserLocale);
        assert_eq!(os_wins.market_profile, MarketProfile::Uk);

        let lang_fallback = resolve_locale_sources(None, None, Some("en_US.UTF-8"));
        assert_eq!(lang_fallback.source, LocaleSource::Lang);
        assert_eq!(lang_fallback.market_profile, MarketProfile::Us);
    }

    #[test]
    fn non_target_higher_priority_source_does_not_fall_through() {
        let override_resolution =
            resolve_locale_sources(Some("fr-FR"), Some("en-US"), Some("ru_RU.UTF-8"));
        assert_eq!(override_resolution.source, LocaleSource::Override);
        assert_eq!(
            override_resolution.market_profile,
            MarketProfile::NeutralEnglish
        );

        let os_resolution = resolve_locale_sources(None, Some("de-DE"), Some("ru_RU.UTF-8"));
        assert_eq!(os_resolution.source, LocaleSource::WindowsUserLocale);
        assert_eq!(os_resolution.market_profile, MarketProfile::NeutralEnglish);
    }

    #[test]
    fn empty_sources_are_neutral() {
        let resolution = resolve_locale_sources(Some("   "), Some(""), None);
        assert_eq!(resolution.source, LocaleSource::None);
        assert_eq!(resolution.raw, None);
        assert_eq!(resolution.normalized, "");
        assert_eq!(resolution.market_profile, MarketProfile::NeutralEnglish);
    }
}
