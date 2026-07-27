use std::sync::OnceLock;
use std::time::Instant;

static ENABLED: OnceLock<bool> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();

/// Anchors the timestamp column at process start rather than at the first
/// line logged, and settles the on/off read once so the hot paths never
/// touch the environment again.
pub fn init() {
    let _ = enabled();
    let _ = START.set(Instant::now());
}

pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| match std::env::var("TRAI_DEBUG") {
        Ok(value) => !value.is_empty() && value != "0",
        Err(_) => false,
    })
}

pub fn uptime_secs() -> f64 {
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// Single-line, char-boundary-safe excerpt for logging transcribed text.
pub fn preview(text: &str, max_chars: usize) -> String {
    let flat = text.trim().replace('\n', " ");
    let mut out: String = flat.chars().take(max_chars).collect();
    if flat.chars().count() > max_chars {
        out.push('…');
    }
    out
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        if $crate::log::enabled() {
            eprintln!(
                "[{:>8.3}] {}",
                $crate::log::uptime_secs(),
                format_args!($($arg)*)
            );
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_truncates_on_char_boundaries_and_flattens_newlines() {
        assert_eq!(preview("hello", 10), "hello");
        assert_eq!(preview("  hello\nworld  ", 20), "hello world");
        // Multi-byte input must not panic or split a char.
        assert_eq!(preview("привет мир", 6), "привет…");
    }
}
