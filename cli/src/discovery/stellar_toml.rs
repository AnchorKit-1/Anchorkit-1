use serde::Deserialize;

use crate::error::CliError;

/// The subset of SEP-1 `stellar.toml` fields relevant to capability
/// detection. Fields we don't otherwise use (CURRENCIES, PRINCIPALS, ORG_*,
/// ...) are left unparsed -- `toml`/`serde` ignore unknown keys by default.
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
pub struct StellarToml {
    #[serde(rename = "VERSION")]
    pub version: Option<String>,
    #[serde(rename = "NETWORK_PASSPHRASE")]
    pub network_passphrase: Option<String>,
    #[serde(rename = "FEDERATION_SERVER")]
    pub federation_server: Option<String>,
    #[serde(rename = "WEB_AUTH_ENDPOINT")]
    pub web_auth_endpoint: Option<String>,
    #[serde(rename = "SIGNING_KEY")]
    pub signing_key: Option<String>,
    #[serde(rename = "TRANSFER_SERVER")]
    pub transfer_server: Option<String>,
    #[serde(rename = "TRANSFER_SERVER_SEP0024")]
    pub transfer_server_sep24: Option<String>,
    #[serde(rename = "KYC_SERVER")]
    pub kyc_server: Option<String>,
    #[serde(rename = "DIRECT_PAYMENT_SERVER")]
    pub direct_payment_server: Option<String>,
    #[serde(rename = "ANCHOR_QUOTE_SERVER")]
    pub anchor_quote_server: Option<String>,
}

/// Fetches and parses `https://{domain}/.well-known/stellar.toml` per SEP-1.
/// Anchors are required to publish it over plain HTTPS with no redirects
/// needed, so we don't follow cross-origin redirects here.
pub fn fetch(client: &reqwest::blocking::Client, domain: &str) -> Result<StellarToml, CliError> {
    let url = format!("https://{domain}/.well-known/stellar.toml");

    let response = client
        .get(&url)
        .send()
        .map_err(|e| CliError::Unreachable { url: url.clone(), reason: e.to_string() })?;

    let status = response.status();
    if !status.is_success() {
        return Err(CliError::HttpStatus { url, status: status.as_u16() });
    }

    let body = response
        .text()
        .map_err(|e| CliError::Unreachable { url: url.clone(), reason: e.to_string() })?;

    toml::from_str(&body).map_err(|e| CliError::Malformed { url, reason: e.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A representative `stellar.toml` body covering every field this CLI
    // reads. Anchors host `stellar.toml` on their own infrastructure, which
    // may save the file with either LF or CRLF line endings depending on
    // OS and editor -- both must parse to the exact same result.
    const TOML_BODY_LF: &str = concat!(
        "VERSION=\"2.7.0\"\n",
        "NETWORK_PASSPHRASE=\"Test SDF Network ; September 2015\"\n",
        "FEDERATION_SERVER=\"https://example.com/federation\"\n",
        "WEB_AUTH_ENDPOINT=\"https://example.com/auth\"\n",
        "SIGNING_KEY=\"GABCDEXAMPLEKEY\"\n",
        "TRANSFER_SERVER=\"https://example.com/sep6\"\n",
        "TRANSFER_SERVER_SEP0024=\"https://example.com/sep24\"\n",
        "KYC_SERVER=\"https://example.com/kyc\"\n",
        "DIRECT_PAYMENT_SERVER=\"https://example.com/sep31\"\n",
        "ANCHOR_QUOTE_SERVER=\"https://example.com/sep38\"\n",
    );

    fn to_crlf(lf: &str) -> String {
        // Build from LF source rather than editing a literal CRLF string in
        // this file, since most editors/formatters (and git's own
        // autocrlf) tend to normalize embedded \r\n back to \n.
        lf.replace('\n', "\r\n")
    }

    #[test]
    fn parses_lf_line_endings() {
        let toml: StellarToml = toml::from_str(TOML_BODY_LF).expect("valid LF stellar.toml");
        assert_eq!(toml.version.as_deref(), Some("2.7.0"));
        assert_eq!(toml.transfer_server_sep24.as_deref(), Some("https://example.com/sep24"));
        assert_eq!(toml.anchor_quote_server.as_deref(), Some("https://example.com/sep38"));
    }

    #[test]
    fn parses_crlf_line_endings_identically_to_lf() {
        let crlf_body = to_crlf(TOML_BODY_LF);
        assert!(crlf_body.contains("\r\n"), "test fixture must actually contain CRLF");

        let from_lf: StellarToml = toml::from_str(TOML_BODY_LF).expect("valid LF stellar.toml");
        let from_crlf: StellarToml = toml::from_str(&crlf_body)
            .expect("a CRLF-terminated stellar.toml must parse just like its LF counterpart");

        assert_eq!(
            from_lf, from_crlf,
            "CRLF and LF versions of the same stellar.toml must parse to identical values"
        );
    }

    #[test]
    fn crlf_body_preserves_field_values_without_stray_carriage_returns() {
        // A naive line-ending fix (e.g. splitting on "\n" and re-joining
        // without stripping "\r") would leave a trailing '\r' embedded in
        // the last value on each line. Assert directly that no field value
        // parsed from CRLF ever carries one.
        let crlf_body = to_crlf(TOML_BODY_LF);
        let toml: StellarToml = toml::from_str(&crlf_body).expect("valid CRLF stellar.toml");

        for field in [
            &toml.version,
            &toml.network_passphrase,
            &toml.federation_server,
            &toml.web_auth_endpoint,
            &toml.signing_key,
            &toml.transfer_server,
            &toml.transfer_server_sep24,
            &toml.kyc_server,
            &toml.direct_payment_server,
            &toml.anchor_quote_server,
        ] {
            if let Some(value) = field {
                assert!(
                    !value.contains('\r'),
                    "field value {value:?} retained a stray carriage return from CRLF parsing"
                );
            }
        }
    }
}
