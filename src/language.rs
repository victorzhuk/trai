// Whisper reports languages as full English names ("russian") while
// configuration and the server's probability map use ISO 639-1 codes
// ("ru"). Everything downstream compares codes, so both spellings are
// funnelled through `canonical` first.
const LANGUAGES: &[(&str, &str)] = &[
    ("en", "english"),
    ("zh", "chinese"),
    ("de", "german"),
    ("es", "spanish"),
    ("ru", "russian"),
    ("ko", "korean"),
    ("fr", "french"),
    ("ja", "japanese"),
    ("pt", "portuguese"),
    ("tr", "turkish"),
    ("pl", "polish"),
    ("ca", "catalan"),
    ("nl", "dutch"),
    ("ar", "arabic"),
    ("sv", "swedish"),
    ("it", "italian"),
    ("id", "indonesian"),
    ("hi", "hindi"),
    ("fi", "finnish"),
    ("vi", "vietnamese"),
    ("he", "hebrew"),
    ("uk", "ukrainian"),
    ("el", "greek"),
    ("ms", "malay"),
    ("cs", "czech"),
    ("ro", "romanian"),
    ("da", "danish"),
    ("hu", "hungarian"),
    ("ta", "tamil"),
    ("no", "norwegian"),
    ("th", "thai"),
    ("ur", "urdu"),
    ("hr", "croatian"),
    ("bg", "bulgarian"),
    ("lt", "lithuanian"),
    ("la", "latin"),
    ("mi", "maori"),
    ("ml", "malayalam"),
    ("cy", "welsh"),
    ("sk", "slovak"),
    ("te", "telugu"),
    ("fa", "persian"),
    ("lv", "latvian"),
    ("bn", "bengali"),
    ("sr", "serbian"),
    ("az", "azerbaijani"),
    ("sl", "slovenian"),
    ("kn", "kannada"),
    ("et", "estonian"),
    ("mk", "macedonian"),
    ("br", "breton"),
    ("eu", "basque"),
    ("is", "icelandic"),
    ("hy", "armenian"),
    ("ne", "nepali"),
    ("mn", "mongolian"),
    ("bs", "bosnian"),
    ("kk", "kazakh"),
    ("sq", "albanian"),
    ("sw", "swahili"),
    ("gl", "galician"),
    ("mr", "marathi"),
    ("pa", "punjabi"),
    ("si", "sinhala"),
    ("km", "khmer"),
    ("sn", "shona"),
    ("yo", "yoruba"),
    ("so", "somali"),
    ("af", "afrikaans"),
    ("oc", "occitan"),
    ("ka", "georgian"),
    ("be", "belarusian"),
    ("tg", "tajik"),
    ("sd", "sindhi"),
    ("gu", "gujarati"),
    ("am", "amharic"),
    ("yi", "yiddish"),
    ("lo", "lao"),
    ("uz", "uzbek"),
    ("fo", "faroese"),
    ("ht", "haitian creole"),
    ("ps", "pashto"),
    ("tk", "turkmen"),
    ("nn", "nynorsk"),
    ("mt", "maltese"),
    ("sa", "sanskrit"),
    ("lb", "luxembourgish"),
    ("my", "myanmar"),
    ("bo", "tibetan"),
    ("tl", "tagalog"),
    ("mg", "malagasy"),
    ("as", "assamese"),
    ("tt", "tatar"),
    ("haw", "hawaiian"),
    ("ln", "lingala"),
    ("ha", "hausa"),
    ("ba", "bashkir"),
    ("jw", "javanese"),
    ("su", "sundanese"),
    ("yue", "cantonese"),
];

// Spellings that reach us from a server or a config file but are not
// the name whisper itself reports.
const ALIASES: &[(&str, &str)] = &[
    ("mandarin", "zh"),
    ("castilian", "es"),
    ("flemish", "nl"),
    ("valencian", "ca"),
    ("moldavian", "ro"),
    ("moldovan", "ro"),
    ("sinhalese", "si"),
    ("burmese", "my"),
    ("panjabi", "pa"),
    ("pushto", "ps"),
    ("letzeburgesch", "lb"),
    ("haitian", "ht"),
    ("iw", "he"),
    ("jv", "jw"),
];

/// Reduces a language code or an English language name to its ISO
/// 639-1 code. Anything unrecognised yields `None` rather than itself,
/// so an unknown language can never compare equal to a known one.
pub fn canonical(raw: &str) -> Option<&'static str> {
    let needle = raw.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }

    if let Some((code, _)) = LANGUAGES
        .iter()
        .find(|(code, name)| *code == needle || *name == needle)
    {
        return Some(code);
    }

    ALIASES
        .iter()
        .find(|(alias, _)| *alias == needle)
        .and_then(|(_, code)| canonical(code))
}

/// True only when both sides name the same known language.
pub fn same(left: &str, right: &str) -> bool {
    match (canonical(left), canonical(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_codes_reduce_to_the_same_code() {
        assert_eq!(canonical("russian"), Some("ru"));
        assert_eq!(canonical("ru"), Some("ru"));
        assert_eq!(canonical("Russian"), Some("ru"));
        assert_eq!(canonical("  RU  "), Some("ru"));
        assert_eq!(canonical("haitian creole"), Some("ht"));
        assert_eq!(canonical("mandarin"), Some("zh"));
    }

    #[test]
    fn unrecognised_language_has_no_canonical_form() {
        assert_eq!(canonical("klingon"), None);
        assert_eq!(canonical(""), None);
        assert_eq!(canonical("   "), None);
        assert_eq!(canonical("xx"), None);
    }

    #[test]
    fn same_matches_across_spellings_and_never_matches_the_unknown() {
        assert!(same("english", "en"));
        assert!(same("ru", "russian"));
        assert!(!same("en", "ru"));
        // An undetectable source language must not be taken for the
        // target language: the segment gets translated instead.
        assert!(!same("klingon", "klingon"));
        assert!(!same("", "en"));
    }

    #[test]
    fn every_table_entry_is_reachable_by_both_spellings() {
        for (code, name) in LANGUAGES {
            assert_eq!(canonical(code), Some(*code), "code {code} must resolve");
            assert_eq!(canonical(name), Some(*code), "name {name} must resolve");
        }
        for (alias, code) in ALIASES {
            assert_eq!(canonical(alias), Some(*code), "alias {alias} must resolve");
        }
    }
}
