use super::info::InfoResponse;
use super::stellar_toml::StellarToml;

/// One SEP capability finding: whether the anchor appears to support it, and
/// (when supported) a short human-readable detail -- an endpoint URL or an
/// enabled-asset count, whichever is more informative for that SEP.
pub struct SepFinding {
    pub sep: &'static str,
    pub name: &'static str,
    pub supported: bool,
    pub detail: Option<String>,
}

/// Which transfer server (if any) we fetched `/info` from, so the SEP-6/
/// SEP-24 findings can attach asset counts only to the one we actually
/// queried, rather than guessing about the other.
pub enum InfoSource {
    None,
    Sep6,
    Sep24,
}

/// Runs `toml` (and, if fetched, `info`) through capability detection and
/// returns one finding per SEP `anchorkit discover` knows how to recognize.
///
/// Most SEPs are detected from `stellar.toml` field presence alone --
/// `stellar.toml` is the only signal we have for them. SEP-6 and SEP-24 are
/// the exception: neither `stellar.toml` nor `/info` alone tells the full
/// story, so their support is inferred by combining both. When we fetched
/// `/info` from that specific endpoint, its enabled deposit/withdraw
/// operations corroborate (or contradict) the `stellar.toml` field -- an
/// anchor can advertise a transfer server whose `/info` reports every asset
/// disabled, in which case `/info` wins: it's the anchor's live word on
/// what's actually enabled, `stellar.toml` is just a pointer to where to
/// ask. When `/info` wasn't fetched for a given endpoint (e.g. it lost out
/// to the other one -- see `discover`'s SEP-24-preferred fetch), field
/// presence is the only signal available and we fall back to it.
pub fn detect(toml: &StellarToml, info: Option<&InfoResponse>, info_source: InfoSource) -> Vec<SepFinding> {
    let mut findings = Vec::with_capacity(6);

    findings.push(SepFinding {
        sep: "SEP-1",
        name: "stellar.toml (anchor metadata)",
        supported: true,
        detail: toml.version.clone().map(|v| format!("VERSION {v}")),
    });

    let sep10_supported = toml.web_auth_endpoint.is_some() && toml.signing_key.is_some();
    findings.push(SepFinding {
        sep: "SEP-10",
        name: "Web Authentication",
        supported: sep10_supported,
        detail: toml.web_auth_endpoint.clone(),
    });

    let sep6_is_queried_endpoint = matches!(info_source, InfoSource::Sep6);
    let sep6_supported = sep_transfer_supported(toml.transfer_server.as_deref(), info, sep6_is_queried_endpoint);
    findings.push(SepFinding {
        sep: "SEP-6",
        name: "Deposit/Withdrawal",
        supported: sep6_supported,
        detail: asset_detail(toml.transfer_server.as_deref(), info, sep6_is_queried_endpoint),
    });

    let sep24_is_queried_endpoint = matches!(info_source, InfoSource::Sep24);
    let sep24_supported = sep_transfer_supported(toml.transfer_server_sep24.as_deref(), info, sep24_is_queried_endpoint);
    findings.push(SepFinding {
        sep: "SEP-24",
        name: "Hosted Deposit/Withdrawal",
        supported: sep24_supported,
        detail: asset_detail(toml.transfer_server_sep24.as_deref(), info, sep24_is_queried_endpoint),
    });

    findings.push(SepFinding {
        sep: "SEP-12",
        name: "KYC API",
        supported: toml.kyc_server.is_some(),
        detail: toml.kyc_server.clone(),
    });

    findings.push(SepFinding {
        sep: "SEP-31",
        name: "Cross-Border Payments",
        supported: toml.direct_payment_server.is_some(),
        detail: toml.direct_payment_server.clone(),
    });

    findings.push(SepFinding {
        sep: "SEP-38",
        name: "Anchor RFQ (Quotes)",
        supported: toml.anchor_quote_server.is_some(),
        detail: toml.anchor_quote_server.clone(),
    });

    findings
}

/// Whether a SEP-6/24 transfer server counts as supported: the
/// `stellar.toml` field must be present, and -- when `/info` was fetched
/// from this exact endpoint -- at least one asset there must have an
/// enabled deposit or withdraw operation. Field presence is trusted on its
/// own only when `/info` wasn't queried for this endpoint, since we have no
/// stronger signal to combine it with in that case.
fn sep_transfer_supported(endpoint: Option<&str>, info: Option<&InfoResponse>, is_this_endpoint: bool) -> bool {
    if endpoint.is_none() {
        return false;
    }
    if is_this_endpoint {
        // `info` is always `Some` here in practice (fetching it is what
        // sets `is_this_endpoint`), but fall back to the field-presence
        // signal rather than an unwarranted "unsupported" if that
        // invariant ever breaks.
        info.map(|i| i.enabled_asset_count() > 0).unwrap_or(true)
    } else {
        true
    }
}

fn asset_detail(endpoint: Option<&str>, info: Option<&InfoResponse>, is_this_endpoint: bool) -> Option<String> {
    if is_this_endpoint {
        if let Some(info) = info {
            let count = info.enabled_asset_count();
            return Some(format!(
                "{} ({} asset{} enabled)",
                endpoint?,
                count,
                if count == 1 { "" } else { "s" }
            ));
        }
    }
    endpoint.map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_toml() -> StellarToml {
        StellarToml::default()
    }

    #[test]
    fn sep1_always_reported_supported() {
        let findings = detect(&empty_toml(), None, InfoSource::None);
        let sep1 = findings.iter().find(|f| f.sep == "SEP-1").unwrap();
        assert!(sep1.supported);
    }

    #[test]
    fn sep10_requires_both_endpoint_and_signing_key() {
        let mut toml = empty_toml();
        toml.web_auth_endpoint = Some("https://auth.example.com".into());
        // signing_key missing -- SEP-10 needs both.
        let findings = detect(&toml, None, InfoSource::None);
        assert!(!findings.iter().find(|f| f.sep == "SEP-10").unwrap().supported);

        toml.signing_key = Some("GABC...".into());
        let findings = detect(&toml, None, InfoSource::None);
        assert!(findings.iter().find(|f| f.sep == "SEP-10").unwrap().supported);
    }

    #[test]
    fn sep6_and_sep24_are_detected_independently() {
        let mut toml = empty_toml();
        toml.transfer_server = Some("https://transfer.example.com".into());
        let findings = detect(&toml, None, InfoSource::None);
        assert!(findings.iter().find(|f| f.sep == "SEP-6").unwrap().supported);
        assert!(!findings.iter().find(|f| f.sep == "SEP-24").unwrap().supported);
    }

    #[test]
    fn asset_count_only_attached_to_the_endpoint_actually_queried() {
        let mut toml = empty_toml();
        toml.transfer_server = Some("https://transfer.example.com".into());
        toml.transfer_server_sep24 = Some("https://sep24.example.com".into());

        let info: InfoResponse =
            serde_json::from_str(r#"{"deposit": {"USD": {"enabled": true}}}"#).unwrap();

        // Info was fetched from the SEP-24 endpoint (the preferred one), so
        // only its finding should carry an asset count.
        let findings = detect(&toml, Some(&info), InfoSource::Sep24);
        let sep6 = findings.iter().find(|f| f.sep == "SEP-6").unwrap();
        let sep24 = findings.iter().find(|f| f.sep == "SEP-24").unwrap();
        assert_eq!(sep6.detail.as_deref(), Some("https://transfer.example.com"));
        assert!(sep24.detail.as_ref().unwrap().contains("1 asset enabled"));
    }

    #[test]
    fn sep6_unsupported_when_queried_info_shows_no_enabled_operations() {
        let mut toml = empty_toml();
        toml.transfer_server = Some("https://transfer.example.com".into());

        // stellar.toml advertises the endpoint, but /info -- fetched from
        // that very endpoint -- reports every operation disabled. /info is
        // the stronger, live signal here and should win.
        let info: InfoResponse =
            serde_json::from_str(r#"{"deposit": {"USD": {"enabled": false}}}"#).unwrap();

        let findings = detect(&toml, Some(&info), InfoSource::Sep6);
        let sep6 = findings.iter().find(|f| f.sep == "SEP-6").unwrap();
        assert!(!sep6.supported);
    }

    #[test]
    fn sep24_unsupported_when_queried_info_shows_no_enabled_operations() {
        let mut toml = empty_toml();
        toml.transfer_server_sep24 = Some("https://sep24.example.com".into());

        let info: InfoResponse =
            serde_json::from_str(r#"{"withdraw": {"EUR": {"enabled": false}}}"#).unwrap();

        let findings = detect(&toml, Some(&info), InfoSource::Sep24);
        let sep24 = findings.iter().find(|f| f.sep == "SEP-24").unwrap();
        assert!(!sep24.supported);
    }

    #[test]
    fn sep_transfer_support_falls_back_to_field_presence_when_info_not_queried_for_it() {
        let mut toml = empty_toml();
        toml.transfer_server = Some("https://transfer.example.com".into());
        toml.transfer_server_sep24 = Some("https://sep24.example.com".into());

        // Info was fetched from the SEP-24 endpoint (the preferred one) and
        // shows nothing enabled there -- SEP-24 should reflect that, but
        // SEP-6 wasn't queried and has no /info to contradict its
        // stellar.toml field, so it stays supported on field presence alone.
        let info: InfoResponse =
            serde_json::from_str(r#"{"deposit": {"USD": {"enabled": false}}}"#).unwrap();

        let findings = detect(&toml, Some(&info), InfoSource::Sep24);
        let sep6 = findings.iter().find(|f| f.sep == "SEP-6").unwrap();
        let sep24 = findings.iter().find(|f| f.sep == "SEP-24").unwrap();
        assert!(sep6.supported, "SEP-6 has no /info signal to contradict its stellar.toml field");
        assert!(!sep24.supported, "SEP-24's own /info shows nothing enabled");
    }

    #[test]
    fn sep6_supported_when_queried_info_shows_an_enabled_operation() {
        let mut toml = empty_toml();
        toml.transfer_server = Some("https://transfer.example.com".into());

        let info: InfoResponse =
            serde_json::from_str(r#"{"deposit": {"USD": {"enabled": true}}}"#).unwrap();

        let findings = detect(&toml, Some(&info), InfoSource::Sep6);
        let sep6 = findings.iter().find(|f| f.sep == "SEP-6").unwrap();
        assert!(sep6.supported);
    }

    #[test]
    fn unsupported_seps_have_no_detail() {
        let findings = detect(&empty_toml(), None, InfoSource::None);
        for finding in &findings {
            if finding.sep != "SEP-1" {
                assert!(!finding.supported);
                assert!(finding.detail.is_none());
            }
        }
    }
}
