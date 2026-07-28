# Domain Validation Security - Homograph Attack Resistance

## Overview

The `validate_anchor_domain` function performs **syntactic-only validation** of domain names used for off-chain attestor endpoints (stellar.toml hosting). While this function ensures well-formed domain structure, it is NOT designed to detect homograph (lookalike) attacks used in phishing campaigns.

## Threat Model: Homograph Attacks

Attackers can use Unicode characters and punycode encoding to create domain names that:
1. Look identical or nearly identical to legitimate domains when displayed to users
2. Resolve to attacker-controlled servers for phishing

### Example Attack Vectors

| Attack Vector | Example | Display | Issue |
|---|---|---|---|
| **Cyrillic homoglyphs** | `амаzon.com` | Looks like `amazon.com` | Mixed Latin/Cyrillic script |
| **Greek homoglyphs** | `gοοgle.com` | Looks like `google.com` | Greek omicron (ο) replaces Latin o |
| **Punycode encoding** | `xn--h1alffa9f.xn--p1ai` | Decodes to `раша.рф` | Russian domain masquerading as Russia |
| **Mixed scripts** | `पaypal.com` | Looks like `paypal.com` | Devanagari 'प' instead of 'p' |

## Current Validator Behavior

### What Gets Rejected ✅
- **Unicode characters** (non-ASCII): The validator only accepts ASCII alphanumeric, dash, and dot characters
  - Any domain with Cyrillic, Greek, Arabic, CJK, or combining diacriticals is rejected
  - Example: `امаzon.com` is rejected ✓

### What Gets Accepted ⚠️
- **Punycode-encoded domains** (xn-- prefix): These are syntactically valid ASCII and therefore accepted
  - `xn--h1alffa9f.xn--p1ai` (Cyrillic for `раша.рф`) is accepted
  - `xn--2n1b961e.kr` (Korean for `구글.kr`) is accepted
  - This is expected behavior for the validator; see "Limitations" below

## Fuzzing Corpus: Known Homograph Attack Examples

The test suite (`src/domain_validator.rs`) includes comprehensive fuzzing against known homograph attack patterns:

### Unicode Homographs (Rejected) ✓
```rust
// Cyrillic attacks
"раsha.рф"           // Mixed Cyrillic/Latin
"amаzon.com"         // Cyrillic 'а' instead of Latin 'a'
"раяндex.рф"         // Russian 'Yandex' lookalike
"gооgle.com"         // Cyrillic 'о' instead of Latin 'o'

// Greek attacks
"εxample.com"        // Greek epsilon instead of 'e'
"ρaypal.com"         // Greek rho instead of 'p'

// Arabic/Hebrew attacks
"פайбוק.com"         // Mixed script
"أمزون.com"          // Arabic 'Amazon' lookalike

// Combining diacriticals
"exemple\u{0301}.com" // é as combining character
"café.com"           // Accented character
```

### Punycode Domains (Accepted ⚠️)
```rust
// Known real-world lookalikes that would be encoded as punycode
"xn--h1alffa9f.xn--p1ai"   // Decodes to: раша.рф
"xn--2n1b961e.kr"          // Decodes to: 구글.kr
"xn--80akhbyknj4f.xn--p1ai" // Decodes to: пример.рф
"xn--9ca.com"              // Decodes to: é.com

// Legitimate internationalized domains (also accepted)
"xn--bcher-kva.de"         // Decodes to: bücher.de (German 'books')
```

## Validator Limitations

### Limitation 1: Punycode Encoding
**Scope:** Punycode-encoded domains (xn-- prefix) are accepted  
**Why:** These are syntactically valid ASCII domain names  
**Impact:** An attacker could use punycode to hide a homograph attack  
**Mitigation:** Decode punycode when needed and implement semantic validation

### Limitation 2: Visual Homoglyphs Within ASCII
**Scope:** Domains like `reddlt.com` (digit '1' looks like letter 'l')  
**Why:** These are valid ASCII characters with no syntactic difference  
**Impact:** Human users may be fooled by visual similarity  
**Mitigation:** This is a human perception issue, not a technical issue

### Limitation 3: Intentional by Design
**Scope:** The validator is syntactic-only by design  
**Why:** Semantic validation (checking confusable characters, script mixing) is complex and language-dependent  
**Impact:** Callers must implement additional checks if phishing protection is critical

## Recommended Follow-ups for Phishing Protection

For applications that need protection against homograph attacks:

1. **Punycode Decoding** (if Unicode inspection needed)
   ```rust
   // Decode xn-- prefixed domains to their Unicode form
   // Check for script mixing (e.g., Cyrillic + Latin in same domain)
   ```

2. **Confusable Character Detection**
   - Use libraries like Unicode Consortium's confusables.txt
   - Flag domains mixing scripts
   - Warn on character sequences known to be homograph vectors

3. **Domain Reputation Checking**
   - Query domain reputation services (VirusTotal, URLhaus, etc.)
   - Check WHOIS registration date (newly registered + lookalike = suspicious)
   - Monitor for typosquatting patterns

4. **User Awareness**
   - Display the punycode representation alongside the Unicode form
   - Highlight unusual or newly registered domains
   - Require explicit user confirmation for suspicious domains

## Testing Notes

The homograph fuzzing tests are located in `src/domain_validator.rs`:

- `rejects_unicode_homograph_characters()` — Confirms Unicode attacks are rejected
- `accepts_punycode_encoded_domains()` — Documents that punycode is accepted
- `homograph_attack_fuzzing_corpus()` — Comprehensive real-world attack examples
- `documented_homograph_limitations()` — Explains design choices and limitations

These tests serve dual purposes:
1. **Regression testing** — Ensure the validator behaves consistently
2. **Documentation** — Make explicit which attacks are caught and which aren't

## SEP-10 Authentication Implications

For SEP-10 (Stellar authentication protocol) implementations:

- The `validate_anchor_domain` check ensures the domain string is well-formed for use as a stellar.toml endpoint
- It does NOT verify that the domain is legitimate or owned by the claimed anchor
- Applications should implement **domain reputation checks** and **HTTPS certificate validation** as additional layers
- Consider displaying the punycode representation of internationalized domains to users

## References

- [Unicode Consortium Confusables](http://www.unicode.org/reports/tr36/)
- [RFC 3492 - Punycode](https://tools.ietf.org/html/rfc3492)
- [OWASP - Homograph Attack](https://owasp.org/www-community/attacks/Homograph_attack)
- [Stellar Anchor Requirements](https://developers.stellar.org/docs/anchoring-assets/requirements)
