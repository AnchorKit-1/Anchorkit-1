//! `anchorkit watch` — poll a SEP-6 anchor for live transaction status updates.
//!
//! Streaming transports (SSE) require a browser-side `EventSource` and are not
//! available in a blocking CLI context.  This subcommand therefore uses the
//! most capable transport the anchor advertises:
//!
//!   1. **Long-poll** — `GET /transactions?id=<id>&long_poll_timeout=<n>`.
//!      The anchor holds the connection open until a status change occurs or
//!      the timeout expires, minimising both latency and polling noise.
//!   2. **Polling fallback** — plain `GET /transaction?id=<id>` on a fixed
//!      interval, used when the anchor responds with 404 or 405 to the
//!      long-poll path (indicating it does not support the extension).
//!
//! Reconnection uses the same exponential back-off as the TypeScript SDK: the
//! delay starts at 1 s and caps at 30 s, with up to 10 attempts before giving
//! up.

use std::thread;
use std::time::Duration;

use reqwest::blocking::Client;
use serde::Deserialize;

use crate::error::CliError;

/// All SEP-6 statuses that indicate the transaction has reached a final state.
const TERMINAL_STATUSES: &[&str] = &[
    "completed",
    "error",
    "refunded",
    "expired",
    "no_market",
    "too_small",
    "too_large",
];

fn is_terminal(status: &str) -> bool {
    TERMINAL_STATUSES.contains(&status)
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// Partial shape of `GET /transaction` response.  Only the fields the watch
/// command displays are modelled here.
#[derive(Deserialize, Debug)]
struct TransactionResponse {
    transaction: Transaction,
}

/// Partial shape of `GET /transactions` response (long-poll endpoint).
#[derive(Deserialize, Debug)]
struct TransactionsResponse {
    transactions: Vec<Transaction>,
}

/// The fields of a SEP-6 transaction relevant to status watching.
#[derive(Deserialize, Debug, Clone)]
struct Transaction {
    id: String,
    status: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    more_info_url: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Reconnect / back-off constants
// ---------------------------------------------------------------------------

const MAX_RECONNECT_ATTEMPTS: u32 = 10;
const INITIAL_BACKOFF_MS: u64 = 1_000;
const MAX_BACKOFF_MS: u64 = 30_000;
const DEFAULT_LONG_POLL_TIMEOUT_SECS: u64 = 30;
const DEFAULT_POLL_INTERVAL_MS: u64 = 5_000;

fn backoff_ms(attempt: u32) -> u64 {
    let delay = INITIAL_BACKOFF_MS.saturating_mul(2u64.saturating_pow(attempt));
    delay.min(MAX_BACKOFF_MS)
}

// ---------------------------------------------------------------------------
// Transport selection
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Transport {
    LongPoll,
    Polling,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Watch a single SEP-6 transaction until it reaches a terminal status or
/// `max_reconnect_attempts` consecutive failures occur.
pub fn run(
    transfer_server_url: &str,
    transaction_id: &str,
    auth_token: Option<&str>,
    long_poll_timeout_secs: Option<u64>,
    poll_interval_ms: Option<u64>,
) -> Result<(), CliError> {
    let base_url = transfer_server_url.trim_end_matches('/');
    let long_poll_secs = long_poll_timeout_secs.unwrap_or(DEFAULT_LONG_POLL_TIMEOUT_SECS);
    let poll_ms = poll_interval_ms.unwrap_or(DEFAULT_POLL_INTERVAL_MS);

    let client = Client::builder()
        // Add slack: long-poll timeout plus 5 s for network overhead.
        .timeout(Duration::from_secs(long_poll_secs + 5))
        .build()
        .map_err(|e| CliError::Rpc(e.to_string()))?;

    println!("Watching transaction {transaction_id} on {base_url}");
    println!("Press Ctrl+C to stop.\n");

    let mut transport = Transport::LongPoll;
    let mut consecutive_errors: u32 = 0;

    loop {
        let result = match transport {
            Transport::LongPoll => {
                long_poll_once(&client, base_url, transaction_id, auth_token, long_poll_secs)
            }
            Transport::Polling => {
                poll_once(&client, base_url, transaction_id, auth_token)
            }
        };

        match result {
            // Anchor doesn't support long-poll; switch to polling for all
            // future iterations without counting this as a failure.
            Err(CliError::LongPollUnsupported) => {
                println!("[watch] anchor does not support long-poll — switching to polling");
                transport = Transport::Polling;
                consecutive_errors = 0;
            }

            Err(e) => {
                consecutive_errors += 1;
                eprintln!("[watch] error (attempt {consecutive_errors}/{MAX_RECONNECT_ATTEMPTS}): {e}");

                if consecutive_errors >= MAX_RECONNECT_ATTEMPTS {
                    return Err(CliError::Rpc(format!(
                        "gave up after {MAX_RECONNECT_ATTEMPTS} consecutive errors"
                    )));
                }

                let delay = backoff_ms(consecutive_errors - 1);
                eprintln!("[watch] retrying in {delay}ms…");
                thread::sleep(Duration::from_millis(delay));
            }

            Ok(Some(tx)) => {
                // A status update was received.
                consecutive_errors = 0;
                print_status_update(&tx);

                if is_terminal(&tx.status) {
                    println!("\n✓ Transaction reached terminal status: {}", tx.status);
                    return Ok(());
                }

                // For polling, sleep before the next request.
                if transport == Transport::Polling {
                    thread::sleep(Duration::from_millis(poll_ms));
                }
                // For long-poll, immediately re-issue — the anchor already
                // held the connection for the configured timeout, so sending
                // another request right away is correct.
            }

            Ok(None) => {
                // Long-poll timed out without a status change (anchor returned
                // an empty transactions list or the same status).  Re-issue
                // immediately.
                consecutive_errors = 0;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Long-poll
// ---------------------------------------------------------------------------

fn long_poll_once(
    client: &Client,
    base_url: &str,
    transaction_id: &str,
    auth_token: Option<&str>,
    timeout_secs: u64,
) -> Result<Option<Transaction>, CliError> {
    let url = format!(
        "{base_url}/transactions?id={}&long_poll_timeout={}",
        urlencoding_simple(transaction_id),
        timeout_secs
    );

    let mut req = client.get(&url);
    if let Some(token) = auth_token {
        req = req.bearer_auth(token);
    }

    let response = req.send().map_err(|e| CliError::Rpc(e.to_string()))?;

    match response.status().as_u16() {
        // Anchor doesn't support long-poll at this path.
        404 | 405 => return Err(CliError::LongPollUnsupported),
        s if !(200..300).contains(&s) => {
            return Err(CliError::Rpc(format!("long-poll returned HTTP {s}")));
        }
        _ => {}
    }

    let body: TransactionsResponse = response
        .json()
        .map_err(|e| CliError::Rpc(format!("failed to parse long-poll response: {e}")))?;

    Ok(body.transactions.into_iter().next())
}

// ---------------------------------------------------------------------------
// Polling fallback
// ---------------------------------------------------------------------------

fn poll_once(
    client: &Client,
    base_url: &str,
    transaction_id: &str,
    auth_token: Option<&str>,
) -> Result<Option<Transaction>, CliError> {
    let url = format!(
        "{base_url}/transaction?id={}",
        urlencoding_simple(transaction_id)
    );

    let mut req = client.get(&url);
    if let Some(token) = auth_token {
        req = req.bearer_auth(token);
    }

    let response = req.send().map_err(|e| CliError::Rpc(e.to_string()))?;

    if !response.status().is_success() {
        return Err(CliError::Rpc(format!(
            "poll returned HTTP {}",
            response.status()
        )));
    }

    let body: TransactionResponse = response
        .json()
        .map_err(|e| CliError::Rpc(format!("failed to parse poll response: {e}")))?;

    Ok(Some(body.transaction))
}

// ---------------------------------------------------------------------------
// Display helpers
// ---------------------------------------------------------------------------

fn print_status_update(tx: &Transaction) {
    let timestamp = tx.updated_at.as_deref().unwrap_or("—");
    print!("[{}] status: {}", timestamp, tx.status);
    if let Some(msg) = &tx.message {
        print!("  — {msg}");
    }
    println!();
    if let Some(url) = &tx.more_info_url {
        println!("         more info: {url}");
    }
}

// ---------------------------------------------------------------------------
// Minimal URL-encoding for transaction IDs
// ---------------------------------------------------------------------------

/// Percent-encodes characters that are not unreserved URI characters.
/// Sufficient for transaction ID strings (alphanumeric + `-._~`).
fn urlencoding_simple(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~' => out.push(b as char),
            _ => {
                use std::fmt::Write as _;
                write!(out, "%{b:02X}").unwrap();
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_statuses_are_recognised() {
        for s in TERMINAL_STATUSES {
            assert!(is_terminal(s), "{s} should be terminal");
        }
    }

    #[test]
    fn non_terminal_statuses_are_not_terminal() {
        let non_terminal = [
            "pending_external",
            "pending_anchor",
            "pending_stellar",
            "pending_trust",
            "pending_user",
            "pending_user_transfer_start",
        ];
        for s in &non_terminal {
            assert!(!is_terminal(s), "{s} should not be terminal");
        }
    }

    #[test]
    fn backoff_is_capped_at_maximum() {
        // After many retries the delay should never exceed MAX_BACKOFF_MS.
        for attempt in 0..50 {
            assert!(backoff_ms(attempt) <= MAX_BACKOFF_MS);
        }
    }

    #[test]
    fn backoff_grows_up_to_cap() {
        // Early attempts should still grow.
        assert!(backoff_ms(1) > backoff_ms(0));
        assert!(backoff_ms(2) > backoff_ms(1));
        // And then it should plateau.
        assert_eq!(backoff_ms(20), backoff_ms(30));
    }

    #[test]
    fn urlencoding_simple_leaves_safe_chars_unchanged() {
        assert_eq!(urlencoding_simple("abc-123_tx.~"), "abc-123_tx.~");
    }

    #[test]
    fn urlencoding_simple_encodes_spaces_and_specials() {
        assert_eq!(urlencoding_simple("a b+c"), "a%20b%2Bc");
    }
}
