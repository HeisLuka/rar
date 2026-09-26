#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocaleSource {
    Override,
    WindowsUserLocale,
    Lang,
}

impl LocaleSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Override => "override",
            Self::WindowsUserLocale => "windows_user_locale",
            Self::Lang => "lang",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DetectedLocale {
    raw: String,
    source: LocaleSource,
}

impl DetectedLocale {
    pub(crate) fn raw(&self) -> &str {
        &self.raw
    }

    pub(crate) fn source(&self) -> LocaleSource {
        self.source
    }
}

pub(crate) fn detect_user_locale() -> Option<DetectedLocale> {
    let override_locale = std::env::var("CHAPTERA_LOCALE").ok();
    let os_locale = platform_user_locale();
    let lang = std::env::var("LANG").ok();

    resolve_locale_source(
        override_locale.as_deref(),
        os_locale.as_deref(),
        lang.as_deref(),
    )
    .map(|(source, raw)| DetectedLocale {
        raw: raw.to_owned(),
        source,
    })
}

fn resolve_locale_source<'a>(
    override_locale: Option<&'a str>,
    os_locale: Option<&'a str>,
    lang: Option<&'a str>,
) -> Option<(LocaleSource, &'a str)> {
    [
        (LocaleSource::Override, override_locale),
        (LocaleSource::WindowsUserLocale, os_locale),
        (LocaleSource::Lang, lang),
    ]
    .into_iter()
    .filter_map(|(source, value)| value.map(|value| (source, value.trim())))
    .find(|(_, value)| !value.is_empty())
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
    fn explicit_override_wins_over_os_and_lang() {
        assert_eq!(
            resolve_locale_source(Some("ru-RU"), Some("en-US"), Some("en_GB.UTF-8")),
            Some((LocaleSource::Override, "ru-RU"))
        );
    }

    #[test]
    fn os_locale_wins_over_lang_when_no_override_exists() {
        assert_eq!(
            resolve_locale_source(None, Some("en-GB"), Some("en_US.UTF-8")),
            Some((LocaleSource::WindowsUserLocale, "en-GB"))
        );
    }

    #[test]
    fn lang_is_only_a_fallback() {
        assert_eq!(
            resolve_locale_source(None, None, Some("ru_RU.UTF-8")),
            Some((LocaleSource::Lang, "ru_RU.UTF-8"))
        );
    }

    #[test]
    fn empty_sources_fail_closed() {
        assert_eq!(resolve_locale_source(Some("  "), Some(""), None), None);
    }

    #[test]
    fn source_labels_are_stable_and_non_identifying() {
        assert_eq!(LocaleSource::Override.as_str(), "override");
        assert_eq!(
            LocaleSource::WindowsUserLocale.as_str(),
            "windows_user_locale"
        );
        assert_eq!(LocaleSource::Lang.as_str(), "lang");
    }
}
