use std::fmt;

/// Errors surfaced by the `anchorkit playground` REPL. Every variant renders
/// to a single human-readable line via `Display` -- the REPL loop prints
/// that line and moves on to the next prompt, so a typo or a bad address
/// never panics the whole session.
#[derive(Debug)]
pub enum CliError {
    /// The REPL couldn't make sense of the input line itself (unknown
    /// command, wrong number of arguments).
    Usage(String),
    /// A specific argument didn't parse into the type the method expects.
    InvalidArgument { arg: String, reason: String },
    /// Talking to the RPC endpoint failed at the transport/HTTP level, or it
    /// returned something that isn't a well-formed JSON-RPC response.
    Rpc(String),
    /// The RPC endpoint understood the request but the simulated contract
    /// invocation itself failed (e.g. the contract returned an error).
    Simulation(String),
    /// Building or parsing the XDR transaction/result failed.
    Xdr(String),
    /// A domain string failed syntactic validation.
    InvalidDomain(String),
    /// The anchor returned HTTP 404/405 for the long-poll endpoint, indicating
    /// it doesn't support the `long_poll_timeout` extension.
    LongPollUnsupported,
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Usage(msg) => write!(f, "{msg}"),
            CliError::InvalidArgument { arg, reason } => {
                write!(f, "invalid argument '{arg}': {reason}")
            }
            CliError::Rpc(msg) => write!(f, "RPC request failed: {msg}"),
            CliError::Simulation(msg) => write!(f, "contract call failed: {msg}"),
            CliError::Xdr(msg) => write!(f, "XDR encoding error: {msg}"),
            CliError::InvalidDomain(domain) => {
                write!(f, "invalid domain '{domain}'")
            }
            CliError::LongPollUnsupported => {
                write!(f, "anchor does not support long-poll")
            }
        }
    }
}

impl std::error::Error for CliError {}
