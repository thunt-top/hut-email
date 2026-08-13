use std::sync::LazyLock;

use regex::Regex;

// Practical email syntax check (same shape as the HTML5 `type="email"`
// pattern, extended to require a dotted domain). Not a deliverability
// check — no DNS/MX lookup is performed.
static EMAIL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        ^[a-zA-Z0-9.!\#$%&'*+/=?^_`{|}~-]+
        @
        [a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?
        (?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)+$
        ",
    )
    .expect("static email regex is valid")
});

pub fn is_valid_email(address: &str) -> bool {
    EMAIL_RE.is_match(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plausible_addresses() {
        assert!(is_valid_email("xyz99@mails.tsinghua.edu.cn"));
        assert!(is_valid_email("staff@no-reply.thunt.top"));
        assert!(is_valid_email("a.b+tag@sub.example.co"));
    }

    #[test]
    fn rejects_malformed_addresses() {
        assert!(!is_valid_email("not-an-email"));
        assert!(!is_valid_email("missing-domain@"));
        assert!(!is_valid_email("@missing-local.com"));
        assert!(!is_valid_email("no-dot-domain@localhost"));
        assert!(!is_valid_email("has spaces@example.com"));
    }
}
