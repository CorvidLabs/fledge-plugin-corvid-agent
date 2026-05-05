# fledge-plugin-corvid-agent

A [Fledge](https://github.com/CorvidLabs/fledge) plugin for interacting with a [CorvidAgent](https://github.com/CorvidLabs/corvid-agent) server from the command line.

## Install

```bash
fledge plugin install CorvidLabs/fledge-plugin-corvid-agent
```

## Commands

| Command | Description |
|---------|-------------|
| `health` | Check server status and uptime |
| `agents` | List registered agents |
| `sessions` | List running sessions |
| `work list` | List work tasks (most recent first) |
| `work create` | Create a new work task |
| `chat` | View recent AlgoChat conversations |
| `chat <message>` | Send an AlgoChat message |
| `restart` | Restart the CorvidAgent server |
| `config` | Set the server URL |
| `help` | Show help text |

### Global Flags

| Flag | Description |
|------|-------------|
| `--yes`, `-y` | Skip confirmations (non-interactive mode) |
| `--json` | Output raw JSON (machine-readable) |

### health

Check server status and uptime.

```bash
fledge corvid-agent health
fledge corvid-agent health --json
```

Aliases: `status`

### agents

List all registered agents with their model and current status.

```bash
fledge corvid-agent agents
fledge corvid-agent agents --json
```

### sessions

List currently running sessions with their assigned agent and cost.

```bash
fledge corvid-agent sessions
fledge corvid-agent sessions --json
```

### work

Manage work tasks.

```bash
# List recent work tasks
fledge corvid-agent work list

# Create a task interactively
fledge corvid-agent work create

# Create a task non-interactively
fledge corvid-agent work create --agent Jackdaw "Fix the login bug"
fledge corvid-agent work create -a Rook "Deploy staging"
```

#### work create flags

| Flag | Description |
|------|-------------|
| `--agent`, `-a` | Assign to a specific agent by name |

Any remaining positional arguments are joined as the task description.

### chat

View or send AlgoChat messages.

```bash
# List recent conversations
fledge corvid-agent chat

# Send a message
fledge corvid-agent chat Hello from Fledge!
```

### restart

Restart the CorvidAgent server. Prompts for confirmation unless `--yes` is passed.

```bash
fledge corvid-agent restart
fledge corvid-agent restart --yes
```

### config

Configure the server URL. The URL is persisted via Fledge's plugin store.

```bash
# Interactive
fledge corvid-agent config

# Non-interactive
fledge corvid-agent config --url http://myserver:3000
```

## Configuration

By default, the plugin connects to `http://localhost:3000`. Use `fledge corvid-agent config` to change the server URL — it's persisted via Fledge's plugin store.

## Development

```bash
# Build
cargo build --release

# Run tests
cargo test

# Lint
cargo clippy -- -D warnings

# Format check
cargo fmt --check

# Full verification lane
fledge lanes run verify

# Install locally for testing
fledge plugin install --path .
```

## License

MIT
