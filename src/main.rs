use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> String {
    NEXT_ID.fetch_add(1, Ordering::Relaxed).to_string()
}

// ---------------------------------------------------------------------------
// Inbound messages (fledge -> plugin)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct InitMessage {
    #[allow(dead_code)]
    protocol: String,
    args: Vec<String>,
    #[allow(dead_code)]
    project: Option<ProjectInfo>,
    #[allow(dead_code)]
    plugin: PluginInfo,
    #[allow(dead_code)]
    fledge: FledgeInfo,
}

#[derive(Debug, Deserialize)]
struct ProjectInfo {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    root: String,
    #[allow(dead_code)]
    language: Option<String>,
    #[allow(dead_code)]
    git: Option<GitInfo>,
}

#[derive(Debug, Deserialize)]
struct GitInfo {
    #[allow(dead_code)]
    branch: String,
    #[allow(dead_code)]
    dirty: bool,
}

#[derive(Debug, Deserialize)]
struct PluginInfo {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    version: String,
    #[allow(dead_code)]
    dir: String,
}

#[derive(Debug, Deserialize)]
struct FledgeInfo {
    #[allow(dead_code)]
    version: String,
}

#[derive(Debug, Deserialize)]
struct InboundMessage {
    #[serde(rename = "type")]
    msg_type: String,
    #[allow(dead_code)]
    id: Option<String>,
    value: Option<serde_json::Value>,
    #[allow(dead_code)]
    reason: Option<String>,
}

// ---------------------------------------------------------------------------
// Outbound messages (plugin -> fledge)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OutboundMessage {
    Prompt {
        id: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        default: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        validate: Option<String>,
    },
    Confirm {
        id: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        default: Option<bool>,
    },
    Select {
        id: String,
        message: String,
        options: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        default: Option<usize>,
    },
    Progress {
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        current: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        total: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        done: Option<bool>,
    },
    Log {
        level: String,
        message: String,
    },
    Output {
        text: String,
    },
    Store {
        key: String,
        value: String,
    },
    Load {
        id: String,
        key: String,
    },
    Exec {
        id: String,
        command: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timeout: Option<u64>,
    },
}

// ---------------------------------------------------------------------------
// IO helpers
// ---------------------------------------------------------------------------

struct PluginIO {
    stdin: io::StdinLock<'static>,
    stdout: io::StdoutLock<'static>,
}

impl PluginIO {
    fn new() -> Self {
        let stdin = Box::leak(Box::new(io::stdin())).lock();
        let stdout = Box::leak(Box::new(io::stdout())).lock();
        Self { stdin, stdout }
    }

    fn recv_line(&mut self) -> Option<String> {
        let mut line = String::new();
        match self.stdin.read_line(&mut line) {
            Ok(0) => None,
            Ok(_) => Some(line),
            Err(e) => {
                eprintln!("stdin read error: {e}");
                None
            }
        }
    }

    fn recv_init(&mut self) -> InitMessage {
        let line = self.recv_line().expect("expected init message on stdin");
        serde_json::from_str(&line).expect("failed to parse init message")
    }

    fn recv_response(&mut self) -> InboundMessage {
        let line = self.recv_line().expect("expected response on stdin");
        let msg: InboundMessage =
            serde_json::from_str(&line).expect("failed to parse inbound message");
        if msg.msg_type == "cancel" {
            std::process::exit(1);
        }
        msg
    }

    fn send(&mut self, msg: &OutboundMessage) {
        serde_json::to_writer(&mut self.stdout, msg).expect("failed to serialize");
        writeln!(self.stdout).expect("failed to write newline");
        self.stdout.flush().expect("flush failed");
    }

    fn request(&mut self, msg: &OutboundMessage) -> InboundMessage {
        self.send(msg);
        self.recv_response()
    }

    fn output(&mut self, text: &str) {
        self.send(&OutboundMessage::Output {
            text: text.to_string(),
        });
    }

    fn log(&mut self, level: &str, message: &str) {
        self.send(&OutboundMessage::Log {
            level: level.to_string(),
            message: message.to_string(),
        });
    }

    fn curl(&mut self, url: &str) -> String {
        let resp = self.request(&OutboundMessage::Exec {
            id: next_id(),
            command: format!("curl -s '{url}'"),
            cwd: None,
            timeout: Some(30),
        });
        resp.value
            .as_ref()
            .and_then(|v| v.get("stdout"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    }

    fn curl_post(&mut self, url: &str, body: &str) -> String {
        let escaped = body.replace('\'', "'\\''");
        let resp = self.request(&OutboundMessage::Exec {
            id: next_id(),
            command: format!("curl -s -X POST -H 'Content-Type: application/json' -d '{escaped}' {url}"),
            cwd: None,
            timeout: Some(30),
        });
        resp.value
            .as_ref()
            .and_then(|v| v.get("stdout"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn get_base_url(io: &mut PluginIO) -> String {
    let resp = io.request(&OutboundMessage::Load {
        id: next_id(),
        key: "corvid_base_url".to_string(),
    });
    resp.value
        .as_ref()
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("http://localhost:3000")
        .to_string()
}

fn cmd_health(io: &mut PluginIO, base_url: &str) {
    io.send(&OutboundMessage::Progress {
        message: Some("Checking server health".into()),
        current: None,
        total: None,
        done: None,
    });

    let raw = io.curl(&format!("{base_url}/api/health"));
    io.send(&OutboundMessage::Progress {
        message: None,
        current: None,
        total: None,
        done: Some(true),
    });

    if raw.is_empty() {
        io.output("  Server is not reachable.\n");
        return;
    }

    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(health) => {
            let status = health.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
            let uptime = health.get("uptime").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let hours = (uptime / 3600.0) as u64;
            let mins = ((uptime % 3600.0) / 60.0) as u64;

            io.output(&format!(
                "\n  Server Health\n  \
                 Status:  {status}\n  \
                 Uptime:  {hours}h {mins}m\n\n"
            ));

            if let Some(agents) = health.get("agents").and_then(|v| v.as_u64()) {
                io.output(&format!("  Active agents: {agents}\n"));
            }
            if let Some(sessions) = health.get("activeSessions").and_then(|v| v.as_u64()) {
                io.output(&format!("  Active sessions: {sessions}\n"));
            }
            io.output("\n");
        }
        Err(_) => {
            io.output(&format!("  Raw response: {raw}\n"));
        }
    }
}

fn cmd_agents(io: &mut PluginIO, base_url: &str) {
    let raw = io.curl(&format!("{base_url}/api/agents"));
    if raw.is_empty() {
        io.output("  Server not reachable.\n");
        return;
    }

    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(data) => {
            let agents = data.as_array().or_else(|| {
                data.get("agents").and_then(|v| v.as_array())
            });

            if let Some(agents) = agents {
                io.output(&format!("\n  Agents ({} total)\n\n", agents.len()));
                for agent in agents {
                    let name = agent.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let model = agent.get("model").and_then(|v| v.as_str()).unwrap_or("?");
                    let status = agent.get("status").and_then(|v| v.as_str()).unwrap_or("idle");
                    io.output(&format!("  {name:<16} {model:<20} [{status}]\n"));
                }
                io.output("\n");
            } else {
                io.output(&format!("  Response: {raw}\n"));
            }
        }
        Err(_) => io.output(&format!("  Raw: {raw}\n")),
    }
}

fn cmd_sessions(io: &mut PluginIO, base_url: &str) {
    let resp = io.request(&OutboundMessage::Exec {
        id: next_id(),
        command: format!(
            "curl -s '{base_url}/api/sessions' | python3 -c \"\
import sys,json; data=json.load(sys.stdin); \
running=[s for s in data if s.get('status')=='running']; \
print(json.dumps(running[:20]))\""
        ),
        cwd: None,
        timeout: Some(60),
    });
    let raw = resp.value
        .as_ref()
        .and_then(|v| v.get("stdout"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if raw.is_empty() {
        io.output("  Server not reachable.\n");
        return;
    }

    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(data) => {
            if let Some(sessions) = data.as_array() {
                io.output(&format!("\n  Running Sessions ({})\n\n", sessions.len()));
                for s in sessions.iter().take(10) {
                    let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let short_id = &id[..8.min(id.len())];
                    let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let source = s.get("source").and_then(|v| v.as_str()).unwrap_or("?");
                    let cost = s.get("totalCostUsd").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    io.output(&format!("  {short_id}  [{source}]  {name}  (${cost:.2})\n"));
                }
                io.output("\n");
            } else {
                io.output("  Unexpected response format.\n");
            }
        }
        Err(_) => io.output("  Failed to parse sessions.\n"),
    }
}

fn cmd_work(io: &mut PluginIO, base_url: &str, args: &[String]) {
    let subcmd = args.first().map(|s| s.as_str()).unwrap_or("list");

    match subcmd {
        "list" => {
            let resp = io.request(&OutboundMessage::Exec {
                id: next_id(),
                command: format!(
                    "curl -s '{base_url}/api/work-tasks' | python3 -c \"\
import sys,json; data=json.load(sys.stdin); \
recent=sorted(data,key=lambda t:t.get('queuedAt') or '',reverse=True)[:20]; \
out=[dict(id=t.get('id','?'),status=t.get('status','?'),description=(t.get('description') or '')[:80],prUrl=t.get('prUrl')) for t in recent]; \
print(json.dumps(out))\""
                ),
                cwd: None,
                timeout: Some(60),
            });
            let raw = resp.value
                .as_ref()
                .and_then(|v| v.get("stdout"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();

            if raw.is_empty() {
                io.output("  Server not reachable.\n");
                return;
            }
            match serde_json::from_str::<serde_json::Value>(&raw) {
                Ok(data) => {
                    if let Some(tasks) = data.as_array() {
                        io.output(&format!("\n  Work Tasks (showing latest {})\n\n", tasks.len()));
                        for t in tasks {
                            let id = t.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                            let short = &id[..8.min(id.len())];
                            let status = t.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                            let desc = t.get("description").and_then(|v| v.as_str()).unwrap_or("(no description)");
                            let truncated = if desc.len() > 60 { &desc[..60] } else { desc };
                            io.output(&format!("  {short}  [{status:<10}]  {truncated}\n"));
                        }
                        io.output("\n");
                    } else {
                        io.output("  Unexpected response format.\n");
                    }
                }
                Err(_) => io.output("  Failed to parse work tasks.\n"),
            }
        }
        "create" => {
            let resp = io.request(&OutboundMessage::Prompt {
                id: next_id(),
                message: "Task description:".into(),
                default: None,
                validate: Some("non_empty".into()),
            });
            let desc = resp.value.as_ref().and_then(|v| v.as_str()).unwrap_or("").to_string();
            if desc.is_empty() {
                io.output("  Cancelled.\n");
                return;
            }

            let agents_raw = io.curl(&format!("{base_url}/api/agents"));
            let agent_names: Vec<String> = serde_json::from_str::<serde_json::Value>(&agents_raw)
                .ok()
                .and_then(|d| {
                    d.as_array().or_else(|| d.get("agents").and_then(|v| v.as_array())).map(|arr| {
                        arr.iter()
                            .filter_map(|a| a.get("name").and_then(|v| v.as_str()).map(String::from))
                            .collect()
                    })
                })
                .unwrap_or_default();

            if agent_names.is_empty() {
                io.output("  No agents available.\n");
                return;
            }

            let resp = io.request(&OutboundMessage::Select {
                id: next_id(),
                message: "Assign to agent:".into(),
                options: agent_names.clone(),
                default: Some(0),
            });
            let agent = resp.value.as_ref().and_then(|v| v.as_str()).unwrap_or(&agent_names[0]).to_string();

            let body = serde_json::json!({
                "description": desc,
                "agentName": agent,
            });

            io.send(&OutboundMessage::Progress {
                message: Some("Creating work task".into()),
                current: None,
                total: None,
                done: None,
            });

            let result = io.curl_post(&format!("{base_url}/api/work-tasks"), &body.to_string());

            io.send(&OutboundMessage::Progress {
                message: None,
                current: None,
                total: None,
                done: Some(true),
            });

            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&result) {
                let id = parsed.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                io.output(&format!("\n  Work task created: {id}\n  Agent: {agent}\n  Description: {desc}\n\n"));
            } else {
                io.output(&format!("  Response: {result}\n"));
            }
        }
        _ => {
            io.output(&format!("  Unknown work subcommand: {subcmd}\n  Usage: corvid-agent work [list|create]\n"));
        }
    }
}

fn cmd_chat(io: &mut PluginIO, base_url: &str, args: &[String]) {
    if args.is_empty() {
        let raw = io.curl(&format!("{base_url}/api/algochat/conversations"));
        if raw.is_empty() {
            io.output("  Server not reachable.\n");
            return;
        }
        match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(data) => {
                let convos = data.as_array().or_else(|| {
                    data.get("conversations").and_then(|v| v.as_array())
                });
                if let Some(convos) = convos {
                    io.output(&format!("\n  AlgoChat Conversations ({})\n\n", convos.len()));
                    for c in convos {
                        let addr = c.get("participantAddr").and_then(|v| v.as_str()).unwrap_or("?");
                        let short_addr = if addr.len() > 12 { &addr[..12] } else { addr };
                        let created = c.get("createdAt").and_then(|v| v.as_str()).unwrap_or("?");
                        io.output(&format!("  {short_addr}...  {created}\n"));
                    }
                    io.output("\n");
                } else {
                    io.output(&format!("  Response: {raw}\n"));
                }
            }
            Err(_) => io.output(&format!("  Raw: {raw}\n")),
        }
    } else {
        let message = args.join(" ");
        let body = serde_json::json!({ "message": message });
        let result = io.curl_post(&format!("{base_url}/api/algochat/send"), &body.to_string());
        if result.is_empty() {
            io.output("  Message sent.\n");
        } else {
            io.output(&format!("  {result}\n"));
        }
    }
}

fn cmd_config(io: &mut PluginIO) {
    let resp = io.request(&OutboundMessage::Prompt {
        id: next_id(),
        message: "CorvidAgent server URL:".into(),
        default: Some("http://localhost:3000".into()),
        validate: None,
    });
    let url = resp.value.as_ref().and_then(|v| v.as_str()).unwrap_or("http://localhost:3000").to_string();

    io.send(&OutboundMessage::Store {
        key: "corvid_base_url".into(),
        value: url.clone(),
    });

    io.output(&format!("\n  Saved server URL: {url}\n\n"));
}

fn cmd_restart(io: &mut PluginIO, base_url: &str) {
    let resp = io.request(&OutboundMessage::Confirm {
        id: next_id(),
        message: "Restart the CorvidAgent server?".into(),
        default: Some(false),
    });
    let confirmed = resp.value.as_ref().and_then(|v| v.as_bool()).unwrap_or(false);
    if !confirmed {
        io.output("  Cancelled.\n");
        return;
    }

    io.send(&OutboundMessage::Progress {
        message: Some("Restarting server".into()),
        current: None,
        total: None,
        done: None,
    });

    let result = io.curl_post(&format!("{base_url}/api/system/restart"), "{}");

    io.send(&OutboundMessage::Progress {
        message: None,
        current: None,
        total: None,
        done: Some(true),
    });

    if result.is_empty() {
        io.output("  Restart signal sent.\n");
    } else {
        io.output(&format!("  {result}\n"));
    }
}

fn show_help(io: &mut PluginIO) {
    io.output(
        "\n  fledge corvid-agent — CorvidAgent CLI\n\n  \
         Commands:\n    \
         health      Check server status and uptime\n    \
         agents      List registered agents\n    \
         sessions    List active sessions\n    \
         work        Work task management (list, create)\n    \
         chat        View or send AlgoChat messages\n    \
         restart     Restart the server\n    \
         config      Set server URL\n    \
         help        Show this help\n\n  \
         Examples:\n    \
         fledge corvid-agent health\n    \
         fledge corvid-agent agents\n    \
         fledge corvid-agent work create\n    \
         fledge corvid-agent chat Hello from Fledge!\n\n"
    );
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut io = PluginIO::new();
    let init = io.recv_init();

    let command = init.args.first().map(|s| s.as_str()).unwrap_or("help");
    let rest: Vec<String> = if init.args.len() > 1 {
        init.args[1..].to_vec()
    } else {
        vec![]
    };

    let base_url = get_base_url(&mut io);

    match command {
        "health" | "status" => cmd_health(&mut io, &base_url),
        "agents" => cmd_agents(&mut io, &base_url),
        "sessions" => cmd_sessions(&mut io, &base_url),
        "work" => cmd_work(&mut io, &base_url, &rest),
        "chat" => cmd_chat(&mut io, &base_url, &rest),
        "restart" => cmd_restart(&mut io, &base_url),
        "config" => cmd_config(&mut io),
        "help" | "--help" | "-h" => show_help(&mut io),
        _ => {
            io.output(&format!("  Unknown command: {command}\n"));
            show_help(&mut io);
        }
    }

    io.log("info", &format!("corvid-agent {command} complete"));
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("plugin error: {e}");
        std::process::exit(1);
    }
}
