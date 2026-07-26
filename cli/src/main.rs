mod args;
mod discovery;
mod display;
mod error;
mod repl;
mod rpc;
mod watch;

use clap::{Parser, Subcommand};

use rpc::RpcClient;

#[derive(Parser)]
#[command(name = "anchorkit", version, about = "AnchorKit developer CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Interactive REPL for calling read-only methods against a deployed
    /// AnchorKit contract instance.
    Playground {
        /// Soroban RPC endpoint, e.g. https://soroban-testnet.stellar.org
        #[arg(long)]
        rpc_url: String,

        /// Deployed contract address (starts with 'C').
        #[arg(long)]
        contract_id: String,

        /// Any syntactically valid account address (starts with 'G') to use
        /// as the transaction's source for simulation. It never needs to be
        /// funded and nothing here is ever signed or submitted -- every
        /// supported method is read-only.
        #[arg(long)]
        source: String,
    },

    /// Discover which SEPs an anchor supports by reading its stellar.toml
    /// and querying its transfer server /info endpoint.
    Discover {
        /// Anchor domain, e.g. testanchor.stellar.org
        #[arg(long)]
        domain: String,
    },

    /// Watch a SEP-6 transaction until it reaches a terminal status,
    /// printing each status update as it arrives.
    ///
    /// Uses long-poll (GET /transactions?long_poll_timeout=N) when the anchor
    /// supports it, falling back to plain polling (GET /transaction) otherwise.
    Watch {
        /// SEP-6 transfer server base URL, e.g. https://testanchor.stellar.org/sep6
        #[arg(long)]
        transfer_server_url: String,

        /// SEP-6 transaction ID to watch.
        #[arg(long)]
        transaction_id: String,

        /// SEP-10 JWT for authenticated endpoints. Optional for anchors
        /// that allow unauthenticated transaction queries.
        #[arg(long)]
        auth_token: Option<String>,

        /// Long-poll timeout in seconds sent to the anchor. Default: 30.
        #[arg(long)]
        long_poll_timeout: Option<u64>,

        /// Polling interval in milliseconds (used when falling back from
        /// long-poll). Default: 5000.
        #[arg(long)]
        poll_interval_ms: Option<u64>,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Playground { rpc_url, contract_id, source } => {
            match RpcClient::new(&rpc_url, &contract_id, &source) {
                Ok(client) => repl::run(&client),
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Command::Discover { domain } => {
            match discovery::discover(&domain) {
                Ok(report) => discovery::print_report(&report),
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Command::Watch {
            transfer_server_url,
            transaction_id,
            auth_token,
            long_poll_timeout,
            poll_interval_ms,
        } => {
            if let Err(e) = watch::run(
                &transfer_server_url,
                &transaction_id,
                auth_token.as_deref(),
                long_poll_timeout,
                poll_interval_ms,
            ) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    }
}
