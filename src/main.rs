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
            command: format!("curl -s -X POST -H 'Content-Type: application/json' -d '{escaped}' '{url}'"),
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

    fn exec_pipe(&mut self, command: &str) -> String {
        let resp = self.request(&OutboundMessage::Exec {
            id: next_id(),
            command: command.to_string(),
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
// Flag parsing helpers
// ---------------------------------------------------------------------------

struct Flags {
    yes: bool,
    json: bool,
    positional: Vec<String>,
}

fn parse_flags(args: &[String]) -> Flags {
    let mut yes = false;
    let mut json = false;
    let mut positional = Vec::new();

    for arg in args {
        match arg.as_str() {
            "--yes" | "-y" => yes = true,
            "--json" => json = true,
            _ => positional.push(arg.clone()),
        }
    }

    Flags { yes, json, positional }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

const DEFAULT_URL: &str = "http://localhost:3000";

fn get_base_url(io: &mut PluginIO) -> String {
    let resp = io.request(&OutboundMessage::Load {
        id: next_id(),
        key: "corvid_base_url".to_string(),
    });
    resp.value
        .as_ref()
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_URL)
        .to_string()
}

fn cmd_health(io: &mut PluginIO, base_url: &str, json_output: bool) {
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
        if json_output {
            io.output("{\"error\":\"server not reachable\"}\n");
        } else {
            io.output("  Server is not reachable.\n");
        }
        return;
    }

    if json_output {
        io.output(&format!("{raw}\n"));
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

fn cmd_agents(io: &mut PluginIO, base_url: &str, json_output: bool) {
    let raw = io.curl(&format!("{base_url}/api/agents"));
    if raw.is_empty() {
        if json_output {
            io.output("{\"error\":\"server not reachable\"}\n");
        } else {
            io.output("  Server not reachable.\n");
        }
        return;
    }

    if json_output {
        io.output(&format!("{raw}\n"));
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

fn cmd_sessions(io: &mut PluginIO, base_url: &str, json_output: bool) {
    let cmd = format!(
        "curl -s '{base_url}/api/sessions' | python3 -c \"\
import sys,json; \
data=json.load(sys.stdin); \
sessions=data if isinstance(data,list) else data.get('sessions',[]); \
running=[{{'id':s.get('id','')[:8],'agent':s.get('agentName','?'),'status':s.get('status','?'),'cost':s.get('totalCost',0)}} for s in sessions if s.get('status')=='running']; \
json.dump(running,sys.stdout)\""
    );
    let raw = io.exec_pipe(&cmd);

    if raw.is_empty() {
        if json_output {
            io.output("{\"error\":\"server not reachable\"}\n");
        } else {
            io.output("  Server not reachable.\n");
        }
        return;
    }

    if json_output {
        io.output(&format!("{raw}\n"));
        return;
    }

    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(data) => {
            if let Some(sessions) = data.as_array() {
                io.output(&format!("\n  Running Sessions ({})\n\n", sessions.len()));
                for s in sessions.iter().take(20) {
                    let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let agent = s.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
                    let cost = s.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    io.output(&format!("  {id}  {agent:<16} ${cost:.4}\n"));
                }
                io.output("\n");
            } else {
                io.output("  Unexpected response format.\n");
            }
        }
        Err(_) => io.output("  Failed to parse sessions.\n"),
    }
}

fn cmd_work(io: &mut PluginIO, base_url: &str, args: &[String], flags: &Flags) {
    let subcmd = args.first().map(|s| s.as_str()).unwrap_or("list");

    match subcmd {
        "list" => {
            let cmd = format!(
                "curl -s '{base_url}/api/work-tasks' | python3 -c \"\
import sys,json; \
data=json.load(sys.stdin); \
tasks=data if isinstance(data,list) else data.get('tasks',[]); \
tasks.sort(key=lambda t: t.get('queuedAt') or t.get('createdAt') or '', reverse=True); \
out=[{{'id':t.get('id','')[:8],'status':t.get('status','?'),'desc':(t.get('description','') or '')[:60],'agent':t.get('agentName','?')}} for t in tasks[:20]]; \
json.dump(out,sys.stdout)\""
            );
            let raw = io.exec_pipe(&cmd);

            if raw.is_empty() {
                if flags.json {
                    io.output("{\"error\":\"server not reachable\"}\n");
                } else {
                    io.output("  Server not reachable.\n");
                }
                return;
            }

            if flags.json {
                io.output(&format!("{raw}\n"));
                return;
            }

            match serde_json::from_str::<serde_json::Value>(&raw) {
                Ok(data) => {
                    if let Some(tasks) = data.as_array() {
                        io.output(&format!("\n  Work Tasks (showing up to 20)\n\n"));
                        for t in tasks {
                            let id = t.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                            let status = t.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                            let desc = t.get("desc").and_then(|v| v.as_str()).unwrap_or("");
                            let agent = t.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
                            io.output(&format!("  {id}  [{status:<10}]  {agent:<12}  {desc}\n"));
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
            // Non-interactive: fledge corvid-agent work create --agent Jackdaw "Fix the bug"
            // Interactive:     fledge corvid-agent work create
            let remaining = if args.len() > 1 { &args[1..] } else { &[] };
            let work_flags = parse_work_create_flags(remaining);

            let desc = if let Some(d) = work_flags.description {
                d
            } else {
                let resp = io.request(&OutboundMessage::Prompt {
                    id: next_id(),
                    message: "Task description:".into(),
                    default: None,
                    validate: Some("non_empty".into()),
                });
                resp.value.as_ref().and_then(|v| v.as_str()).unwrap_or("").to_string()
            };

            if desc.is_empty() {
                io.output("  Cancelled — no description provided.\n");
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

            let agent = if let Some(a) = work_flags.agent {
                if agent_names.iter().any(|n| n.eq_ignore_ascii_case(&a)) {
                    agent_names.iter().find(|n| n.eq_ignore_ascii_case(&a)).unwrap().clone()
                } else {
                    io.output(&format!("  Unknown agent: {a}\n  Available: {}\n", agent_names.join(", ")));
                    return;
                }
            } else {
                let resp = io.request(&OutboundMessage::Select {
                    id: next_id(),
                    message: "Assign to agent:".into(),
                    options: agent_names.clone(),
                    default: Some(0),
                });
                resp.value.as_ref().and_then(|v| v.as_str()).unwrap_or(&agent_names[0]).to_string()
            };

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
                if flags.json {
                    io.output(&format!("{result}\n"));
                } else {
                    let id = parsed.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    io.output(&format!("\n  Work task created: {id}\n  Agent: {agent}\n  Description: {desc}\n\n"));
                }
            } else {
                io.output(&format!("  Response: {result}\n"));
            }
        }
        _ => {
            io.output(&format!("  Unknown work subcommand: {subcmd}\n  Usage: corvid-agent work [list|create]\n"));
        }
    }
}

struct WorkCreateFlags {
    agent: Option<String>,
    description: Option<String>,
}

fn parse_work_create_flags(args: &[String]) -> WorkCreateFlags {
    let mut agent = None;
    let mut desc_parts: Vec<String> = Vec::new();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--agent" | "-a" => {
                if i + 1 < args.len() {
                    agent = Some(args[i + 1].clone());
                    i += 2;
                    continue;
                }
                i += 1;
            }
            s if !s.starts_with('-') => {
                desc_parts.push(args[i].clone());
                i += 1;
            }
            _ => { i += 1; }
        }
    }

    let description = if desc_parts.is_empty() {
        None
    } else {
        Some(desc_parts.join(" "))
    };

    WorkCreateFlags { agent, description }
}

fn cmd_chat(io: &mut PluginIO, base_url: &str, args: &[String], json_output: bool) {
    if args.is_empty() {
        let raw = io.curl(&format!("{base_url}/api/algochat/conversations"));
        if raw.is_empty() {
            if json_output {
                io.output("{\"error\":\"server not reachable\"}\n");
            } else {
                io.output("  Server not reachable.\n");
            }
            return;
        }

        if json_output {
            io.output(&format!("{raw}\n"));
            return;
        }

        match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(data) => {
                let conversations = data.as_array().or_else(|| {
                    data.get("conversations").and_then(|v| v.as_array())
                });
                if let Some(convos) = conversations {
                    io.output(&format!("\n  AlgoChat Conversations ({})\n\n", convos.len()));
                    for c in convos.iter().take(20) {
                        let contact = c.get("contactName").and_then(|v| v.as_str())
                            .or_else(|| c.get("from").and_then(|v| v.as_str()))
                            .unwrap_or("?");
                        let last_msg = c.get("lastMessage").and_then(|v| v.as_str())
                            .or_else(|| c.get("text").and_then(|v| v.as_str()))
                            .or_else(|| c.get("content").and_then(|v| v.as_str()))
                            .unwrap_or("");
                        let truncated = if last_msg.len() > 60 { &last_msg[..60] } else { last_msg };
                        io.output(&format!("  [{contact}] {truncated}\n"));
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
        if json_output {
            if result.is_empty() {
                io.output("{\"status\":\"sent\"}\n");
            } else {
                io.output(&format!("{result}\n"));
            }
        } else if result.is_empty() {
            io.output("  Message sent.\n");
        } else {
            io.output(&format!("  {result}\n"));
        }
    }
}

fn cmd_config(io: &mut PluginIO, args: &[String]) {
    let mut url: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--url" => {
                if i + 1 < args.len() {
                    url = Some(args[i + 1].clone());
                    i += 2;
                    continue;
                }
                i += 1;
            }
            s if !s.starts_with('-') && url.is_none() => {
                url = Some(args[i].clone());
                i += 1;
            }
            _ => { i += 1; }
        }
    }

    let url = if let Some(u) = url {
        u
    } else {
        let resp = io.request(&OutboundMessage::Prompt {
            id: next_id(),
            message: "CorvidAgent server URL:".into(),
            default: Some(DEFAULT_URL.into()),
            validate: None,
        });
        resp.value.as_ref().and_then(|v| v.as_str()).unwrap_or(DEFAULT_URL).to_string()
    };

    io.send(&OutboundMessage::Store {
        key: "corvid_base_url".into(),
        value: url.clone(),
    });

    io.output(&format!("\n  Saved server URL: {url}\n\n"));
}

fn cmd_restart(io: &mut PluginIO, base_url: &str, yes: bool) {
    if !yes {
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
         sessions    List running sessions\n    \
         work        Work task management (list, create)\n    \
         chat        View or send AlgoChat messages\n    \
         restart     Restart the server\n    \
         config      Set server URL\n    \
         help        Show this help\n\n  \
         Global Flags:\n    \
         --yes, -y   Skip confirmations (non-interactive mode)\n    \
         --json      Output raw JSON (machine-readable)\n\n  \
         Examples:\n    \
         fledge corvid-agent health\n    \
         fledge corvid-agent health --json\n    \
         fledge corvid-agent agents\n    \
         fledge corvid-agent work list\n    \
         fledge corvid-agent work create --agent Jackdaw \"Fix the login bug\"\n    \
         fledge corvid-agent chat Hello from Fledge!\n    \
         fledge corvid-agent restart --yes\n    \
         fledge corvid-agent config --url http://localhost:3000\n\n"
    );
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut io = PluginIO::new();
    let init = io.recv_init();

    let flags = parse_flags(&init.args);

    let command = flags.positional.first().map(|s| s.as_str()).unwrap_or("help");
    let rest: Vec<String> = if flags.positional.len() > 1 {
        flags.positional[1..].to_vec()
    } else {
        vec![]
    };

    let base_url = get_base_url(&mut io);

    match command {
        "health" | "status" => cmd_health(&mut io, &base_url, flags.json),
        "agents" => cmd_agents(&mut io, &base_url, flags.json),
        "sessions" => cmd_sessions(&mut io, &base_url, flags.json),
        "work" => cmd_work(&mut io, &base_url, &rest, &flags),
        "chat" => cmd_chat(&mut io, &base_url, &rest, flags.json),
        "restart" => cmd_restart(&mut io, &base_url, flags.yes),
        "config" => cmd_config(&mut io, &rest),
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
