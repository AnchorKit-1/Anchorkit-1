# `anchorkit` CLI

Developer tooling that lives alongside the on-chain contract, in `cli/`
(binary crate `anchorkit-cli`, executable name `anchorkit`).

```sh
cargo run -p anchorkit-cli -- <command>
# or, after `cargo install --path cli`:
anchorkit <command>
```

## `anchorkit playground`

An interactive REPL for calling **read-only** methods against a deployed
`AnchorKit` contract instance, without writing a one-off script every time
you want to check an attestation.

```sh
anchorkit playground \
  --rpc-url https://soroban-testnet.stellar.org \
  --contract-id CCONTRACTIDXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX \
  --source GSOURCEACCOUNTXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
```

`--source` just needs to be *a* well-formed account address -- it never
needs to be funded, and nothing the playground does is ever signed or
submitted to the network. Every supported method is a read, invoked via the
RPC endpoint's `simulateTransaction`, so `--source` only exists to make a
syntactically valid transaction envelope.

Supported commands:

| Command | Contract method |
|---|---|
| `get_attestation <subject> <attestation_type>` | `get_attestation` |
| `is_valid <subject> <attestation_type>` | `is_valid` |
| `is_attestor <attestor>` | `is_attestor` |
| `get_attestation_count` | `get_attestation_count` |
| `help` | -- |
| `exit` / `quit` | -- |

`<subject>` / `<attestor>` are Stellar addresses (`G...` accounts or `C...`
contracts); `<attestation_type>` is a contract Symbol (ASCII, 32 characters
or fewer).

### Sample session

```text
$ anchorkit playground --rpc-url https://soroban-testnet.stellar.org --contract-id CCONTRACT... --source GSOURCE...
anchorkit playground -- read-only contract calls. Type 'help' for commands, 'exit' to quit.
anchorkit> get_attestation_count
42
anchorkit> is_attestor GATTESTORXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
true
anchorkit> get_attestation GSUBJECTXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX kyc_approved
{attestor: GATTESTORXXX..., subject: GSUBJECTXXX..., attestation_type: kyc_approved, payload_hash: 0x9f86d0..., issued_at: 1732300000, expires_at: 1763836000, status: [Active]}
anchorkit> is_valid GSUBJECTXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX missing_type
false
anchorkit> exit
```

### Error handling

Malformed commands, bad addresses/symbols, and RPC/simulation failures all
print a one-line `Error: ...` message and return to the prompt -- never a
panic:

```text
anchorkit> get_attestation not-an-address kyc_approved
Error: invalid argument 'not-an-address': expected a 'G...' account address or a 'C...' contract address

anchorkit> get_attestation GSUBJECTXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
Error: usage: get_attestation <subject> <attestation_type> (got 1 argument(s))

anchorkit> get_attestation GSUBJECTXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX unregistered_type
Error: contract call failed: HostError: Error(Contract, #7)
```

## `anchorkit watch`

Watch a SEP-6 transaction until it reaches a terminal status, printing each
status change as it arrives. Useful for debugging deposit/withdrawal flows
against a live anchor without writing a custom polling script.

```sh
anchorkit watch \
  --transfer-server-url https://testanchor.stellar.org/sep6 \
  --transaction-id <TX_ID> \
  [--auth-token <JWT>] \
  [--long-poll-timeout <seconds>] \
  [--poll-interval-ms <ms>]
```

### Transport selection

`watch` automatically picks the best transport the anchor supports:

1. **Long-poll** (`GET /transactions?id=<id>&long_poll_timeout=N`) — the
   anchor holds the request open until a status change occurs or the timeout
   expires. Minimal latency, minimal polling noise.  This is attempted first.

2. **Polling fallback** (`GET /transaction?id=<id>` on a fixed interval) —
   used automatically when the anchor responds with HTTP 404 or 405 to the
   long-poll endpoint. The `--poll-interval-ms` flag controls the interval
   (default: 5000 ms).

The selected transport is printed at startup. A one-line message is printed
whenever the anchor does not support long-poll and the stream switches to
polling.

### Reconnection

Transient network failures are retried with truncated exponential back-off
(starts at 1 s, caps at 30 s, up to 10 consecutive attempts). The stream
exits with a non-zero status if all attempts fail.

### Flags

| Flag | Default | Description |
|---|---|---|
| `--transfer-server-url` | — | SEP-6 transfer server base URL (required) |
| `--transaction-id` | — | Transaction ID to watch (required) |
| `--auth-token` | (none) | SEP-10 JWT; sent as `Authorization: Bearer` |
| `--long-poll-timeout` | `30` | Timeout in seconds for long-poll requests |
| `--poll-interval-ms` | `5000` | Interval between polls (polling fallback) |

### Terminal statuses

The command exits `0` when the transaction reaches any terminal status:
`completed`, `error`, `refunded`, `expired`, `no_market`, `too_small`,
`too_large`.

### Sample session

```text
$ anchorkit watch \
    --transfer-server-url https://testanchor.stellar.org/sep6 \
    --transaction-id de8d1d3c-... \
    --auth-token eyJ...

Watching transaction de8d1d3c-... on https://testanchor.stellar.org/sep6
Press Ctrl+C to stop.

[2026-07-26T12:00:01Z] status: pending_external
[2026-07-26T12:00:34Z] status: pending_anchor  — awaiting internal processing
[2026-07-26T12:01:15Z] status: pending_stellar
[2026-07-26T12:01:45Z] status: completed

✓ Transaction reached terminal status: completed
```

### Fallback session (anchor without long-poll support)

```text
$ anchorkit watch \
    --transfer-server-url https://anchor.example.com/sep6 \
    --transaction-id abc-123

Watching transaction abc-123 on https://anchor.example.com/sep6
Press Ctrl+C to stop.

[watch] anchor does not support long-poll — switching to polling
[2026-07-26T12:00:05Z] status: pending_external
[2026-07-26T12:00:10Z] status: pending_anchor
[2026-07-26T12:00:20Z] status: completed

✓ Transaction reached terminal status: completed
```
