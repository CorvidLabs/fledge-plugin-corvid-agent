# fledge-plugin-corvid-agent

A [Fledge](https://github.com/CorvidLabs/fledge) plugin for interacting with a [CorvidAgent](https://github.com/CorvidLabs/corvid-agent) server from the command line.

## Install

```bash
fledge plugin install CorvidLabs/fledge-plugin-corvid-agent
```

## Commands

| Command | Description |
|---------|-------------|
| `fledge corvid-agent health` | Check server status and uptime |
| `fledge corvid-agent agents` | List registered agents |
| `fledge corvid-agent sessions` | List active sessions |
| `fledge corvid-agent work list` | List work tasks |
| `fledge corvid-agent work create` | Create a new work task (interactive) |
| `fledge corvid-agent chat` | View recent AlgoChat messages |
| `fledge corvid-agent chat <message>` | Send an AlgoChat message |
| `fledge corvid-agent restart` | Restart the server (with confirmation) |
| `fledge corvid-agent config` | Set the server URL |

## Configuration

By default, the plugin connects to `http://localhost:3578`. Use `fledge corvid-agent config` to change the server URL — it's persisted via Fledge's plugin store.

## Development

```bash
# Build
cargo build --release

# Verify
fledge lanes run verify

# Install locally for testing
fledge plugin install --path .
```

## License

MIT
