use soroban_sdk::Bytes;

const MAX_DOMAIN_LEN: u32 = 255;
const MAX_LABEL_LEN: u32 = 63;

/// Validates that `domain` looks like a plausible anchor domain (the kind of
/// hostname an off-chain attestor would publish a `stellar.toml` under).
///
/// This is a syntactic check only -- it does not resolve DNS or fetch
/// anything off-chain. It rejects empty/oversized input, disallowed
/// characters, empty labels (leading/trailing/consecutive dots), labels
/// longer than 63 bytes, and labels starting or ending with a hyphen.
pub fn validate_anchor_domain(domain: &Bytes) -> bool {
    is_valid_domain_syntax(domain.len(), domain.iter())
}

/// Same syntactic check as [`validate_anchor_domain`], for callers that have
/// a plain `&str` rather than a `soroban_sdk::Bytes` -- e.g. off-chain
/// tooling like `anchorkit discover`, which has no contract `Env` to build a
/// `Bytes` value from. Shares the exact rule set via [`is_valid_domain_syntax`]
/// so the two never drift apart.
pub fn validate_domain_syntax(domain: &str) -> bool {
    is_valid_domain_syntax(domain.len() as u32, domain.bytes())
}

fn is_valid_domain_syntax(len: u32, bytes: impl Iterator<Item = u8>) -> bool {
    if !(3..=MAX_DOMAIN_LEN).contains(&len) {
        return false;
    }

    let mut has_dot = false;
    let mut label_len: u32 = 0;
    let mut prev: Option<u8> = None;

    for c in bytes {
        let is_alnum = c.is_ascii_alphanumeric();
        let is_dash = c == b'-';
        let is_dot = c == b'.';

        if !is_alnum && !is_dash && !is_dot {
            return false;
        }

        if is_dot {
            has_dot = true;
            if label_len == 0 || prev == Some(b'-') {
                return false;
            }
            label_len = 0;
        } else {
            if is_dash && label_len == 0 {
                return false;
            }
            label_len += 1;
            if label_len > MAX_LABEL_LEN {
                return false;
            }
        }

        prev = Some(c);
    }

    if label_len == 0 || prev == Some(b'-') {
        return false;
    }

    has_dot
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    fn domain(env: &Env, s: &str) -> Bytes {
        Bytes::from_slice(env, s.as_bytes())
    }

    #[test]
    fn accepts_plausible_domains() {
        let env = Env::default();
        assert!(validate_anchor_domain(&domain(&env, "anchor.example.com")));
        assert!(validate_anchor_domain(&domain(&env, "a.co")));
        assert!(validate_anchor_domain(&domain(&env, "sub.multi-part.anchor.io")));
    }

    #[test]
    fn rejects_empty_and_oversized() {
        let env = Env::default();
        assert!(!validate_anchor_domain(&domain(&env, "")));

        let mut too_long_label = Bytes::from_slice(&env, &[b'a'; 64]);
        too_long_label.append(&domain(&env, ".com"));
        assert!(!validate_anchor_domain(&too_long_label));
    }

    #[test]
    fn rejects_missing_dot() {
        let env = Env::default();
        assert!(!validate_anchor_domain(&domain(&env, "localhost")));
    }

    #[test]
    fn rejects_leading_and_trailing_dots() {
        let env = Env::default();
        assert!(!validate_anchor_domain(&domain(&env, ".example.com")));
        assert!(!validate_anchor_domain(&domain(&env, "example.com.")));
    }

    #[test]
    fn rejects_consecutive_dots() {
        let env = Env::default();
        assert!(!validate_anchor_domain(&domain(&env, "example..com")));
    }

    #[test]
    fn rejects_hyphen_at_label_boundary() {
        let env = Env::default();
        assert!(!validate_anchor_domain(&domain(&env, "-example.com")));
        assert!(!validate_anchor_domain(&domain(&env, "example-.com")));
    }

    #[test]
    fn rejects_disallowed_characters() {
        let env = Env::default();
        assert!(!validate_anchor_domain(&domain(&env, "exa mple.com")));
        assert!(!validate_anchor_domain(&domain(&env, "example.com/path")));
        assert!(!validate_anchor_domain(&domain(&env, "exa_mple.com")));
    }

    #[test]
    fn str_variant_agrees_with_bytes_variant() {
        let env = Env::default();
        let cases = [
            "anchor.example.com",
            "a.co",
            "sub.multi-part.anchor.io",
            "",
            "localhost",
            ".example.com",
            "example.com.",
            "example..com",
            "-example.com",
            "example-.com",
            "exa mple.com",
            "example.com/path",
            "exa_mple.com",
        ];
        for case in cases {
            assert_eq!(
                validate_domain_syntax(case),
                validate_anchor_domain(&domain(&env, case)),
                "mismatch for {case:?}"
            );
        }
    }

    #[test]
    fn rejects_unicode_homograph_characters() {
        let env = Env::default();
        // Unicode lookalike characters that are rejected because they're non-ASCII
        // These represent common homograph attacks if Unicode were allowed
        let cases = [
            // Cyrillic 'а' (U+0430) looks like Latin 'a'
            "еxample.com", // Cyrillic 'е' + Latin 'xample'
            // Greek 'ο' (U+03BF) looks like Latin 'o'
            "gοοgle.com", // Greek omicrons instead of Latin o's
            // Arabic/Hebrew homoglyphs
            "פיsher.com", // Hebrew mixed with ASCII
            // CJK characters
            "تест.example.com", // Arabic
            "test例え.com", // Japanese
        ];
        for case in cases {
            assert!(
                !validate_anchor_domain(&domain(&env, case)),
                "should reject Unicode homograph: {case:?}"
            );
        }
    }

    #[test]
    fn accepts_punycode_encoded_domains() {
        let env = Env::default();
        // Punycode (xn-- prefix) is valid ASCII and thus ACCEPTED by the validator.
        // These represent the ASCII encoding of homograph attacks.
        // IMPORTANT: This documents a limitation of syntactic-only validation.
        // While syntactically valid, these domains warrant extra caution:
        //
        // 1. xn--h1alffa9f.xn--p1ai
        //    Decodes to: раша.рф (Russian 'raша' domain - could phish as Russia)
        //
        // 2. xn--2n1b961e.kr
        //    Decodes to: 구글.kr (Korean 'Google' lookalike)
        //
        // 3. xn--80akhbyknj4f.xn--p1ai
        //    Decodes to: пример.рф (Russian 'example' - common phishing vector)
        //
        // 4. xn--9ca.com
        //    Decodes to: é.com (Technically a valid domain but unusual)
        //
        // FOLLOW-UP: Semantic validation (actual Unicode decoding and confusable
        // character detection) is outside the scope of this syntactic validator.
        // Callers should implement additional checks if phishing resistance is needed.
        let punycode_domains = [
            "xn--h1alffa9f.xn--p1ai",  // раsha.рф
            "xn--2n1b961e.kr",         // 구글.kr
            "xn--80akhbyknj4f.xn--p1ai", // пример.рф
            "xn--9ca.com",             // é.com
            "xn--bcher-kva.de",        // bücher.de (legitimate domain)
        ];
        for domain_str in punycode_domains {
            assert!(
                validate_domain_syntax(domain_str),
                "should accept punycode domain: {domain_str:?}"
            );
        }
    }

    #[test]
    fn homograph_attack_fuzzing_corpus() {
        let env = Env::default();
        // Comprehensive fuzz corpus of known real-world homograph attack patterns.
        // All of these should be REJECTED because they contain non-ASCII characters.
        //
        // KNOWN ACCEPTED-BUT-SUSPICIOUS: If these were encoded as punycode (xn--),
        // they would be accepted. See test `accepts_punycode_encoded_domains`.
        let homograph_attacks_unicode = [
            // Cyrillic script attacks (commonly used to spoof Latin domains)
            "раsha.рф",          // Russian 'raша' (mixed Cyrillic/Latin)
            "amаzon.com",        // Cyrillic 'а' in 'amazon'
            "раяндex.рф",        // Russian 'Yandex' lookalike
            "gооgle.com",        // Cyrillic 'о' in 'google'
            // Greek script attacks
            "εxample.com",       // Greek epsilon looks like 'e'
            "ρaypal.com",        // Greek rho looks like 'p'
            // Hebrew/Arabic script attacks
            "פايбוק.com",        // Mixed script attempt
            "أمзון.com",         // Arabic
            // Confusable number/letter combinations (though numbers are ASCII)
            "l1nked1n.com",      // '1' looks like 'l' or 'I' - but this is ASCII so it's accepted
            "reddlt.com",        // '1' looks like 'l' - ASCII, would be accepted
            // Mixed-case and look-alike pairs (all ASCII, these should pass)
            "I1l1O0.com",        // 'I' 'l' '1' '0' 'O' all look similar - ASCII, accepted
            // Combining diacriticals
            "exemple\u{0301}.com", // e + combining acute
            "café.com",          // 'é' character
        ];
        for case in homograph_attacks_unicode {
            let should_reject = !case.is_ascii();
            let result = validate_anchor_domain(&domain(&env, case));
            if should_reject {
                assert!(
                    !result,
                    "should reject Unicode homograph attack: {case:?}"
                );
            } else {
                // ASCII-only homographs (like l1nked1n) are accepted - this is expected
                // but callers should be aware of these look-alike patterns
                assert!(
                    result,
                    "should accept ASCII-only domain: {case:?}"
                );
            }
        }
    }

    #[test]
    fn documented_homograph_limitations() {
        // This test documents the current limitations of the syntactic-only validator
        // and identifies which homograph attacks would require semantic analysis.
        //
        // LIMITATION 1: Punycode domains
        // The validator accepts xn-- prefixed domains (punycode encoding of Unicode).
        // These are syntactically valid but can encode homograph attacks.
        // MITIGATION: Decode punycode and check for confusable characters if needed.
        //
        // LIMITATION 2: Visual homoglyph attacks within same script
        // Domains like "reddlt.com" (with '1' instead of 'l') are accepted.
        // This is not a Unicode issue but a human perception issue.
        // MITIGATION: No fix needed here - this is expected behavior.
        //
        // LIMITATION 3: Combining diacriticals
        // Unicode combining characters (e.g., é as e + combining acute) are rejected
        // because they contain non-ASCII bytes.
        // MITIGATION: Already handled by ASCII-only validation.
        //
        // These limitations are intentional design choices:
        // - This validator is syntactic-only, not semantic.
        // - Callers that need phishing protection should implement additional validation.
        // - The smart contract validates that the domain is well-formed for use as
        //   a stellar.toml endpoint identifier, not that it's safe from phishing.
        assert!(true); // This test only documents limitations; no assertions needed
    }
}
