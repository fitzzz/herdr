use std::path::Path;

use serde::{Deserialize, Serialize};

use OptionArity::{Flag, OptionalValue, Value, Variadic};

const MAX_SESSION_ID_LEN: usize = 512;
const MAX_SESSION_PATH_LEN: usize = 4096;
const MAX_RESUME_ARGS: usize = 64;
const MAX_RESUME_ARGV_BYTES: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSessionRef {
    pub kind: AgentSessionRefKind,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionRefKind {
    Id,
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentResumePlan {
    pub agent: String,
    pub argv: Vec<String>,
    pub dedupe_key: String,
}

/// A resume command reported by the agent itself, run in the restored pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportedAgentResume {
    pub source: String,
    pub agent: String,
    pub argv: Vec<String>,
}

impl ReportedAgentResume {
    /// The same command can name different sessions in different directories,
    /// for example `agent --continue`, so the directory is part of its identity.
    pub fn plan(&self, cwd: &Path) -> AgentResumePlan {
        AgentResumePlan {
            agent: self.agent.clone(),
            argv: self.argv.clone(),
            dedupe_key: format!(
                "{}\u{0}{}\u{0}{}\u{0}argv\u{0}{}",
                self.source,
                self.agent,
                cwd.display(),
                self.argv.join("\u{0}")
            ),
        }
    }
}

/// Restore types the command into the pane's shell, so the executable must be a
/// bare command name: shells disagree on how to invoke a quoted path.
pub fn validate_resume_argv(argv: &[String]) -> Result<(), String> {
    let Some(command) = argv.first() else {
        return Err("resume_argv must not be empty".into());
    };
    if argv.len() > MAX_RESUME_ARGS {
        return Err(format!(
            "resume_argv allows at most {MAX_RESUME_ARGS} arguments"
        ));
    }
    if argv.iter().map(String::len).sum::<usize>() > MAX_RESUME_ARGV_BYTES {
        return Err(format!(
            "resume_argv allows at most {MAX_RESUME_ARGV_BYTES} bytes"
        ));
    }
    if argv.iter().any(|arg| arg.chars().any(char::is_control)) {
        return Err("resume_argv must not contain control characters".into());
    }
    // Restore quotes arguments POSIX-style, which PowerShell reads differently
    // only when an argument itself contains an apostrophe.
    if argv.iter().any(|arg| arg.contains('\'')) {
        return Err("resume_argv must not contain apostrophes".into());
    }
    let plain_command = !command.is_empty()
        && !command.starts_with('-')
        && command
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    if !plain_command {
        return Err("resume_argv must start with a plain command name, not a path".into());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedAgentSession {
    pub source: String,
    pub agent: String,
    pub session_ref: AgentSessionRef,
}

impl AgentSessionRef {
    pub fn id(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        valid_session_id(&value).then_some(Self {
            kind: AgentSessionRefKind::Id,
            value,
        })
    }

    pub fn path(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        valid_session_path(&value).then_some(Self {
            kind: AgentSessionRefKind::Path,
            value,
        })
    }
}

pub fn session_ref_from_report(
    source: &str,
    agent: &str,
    agent_session_id: Option<String>,
    _agent_session_path: Option<String>,
) -> Option<AgentSessionRef> {
    if !is_official_agent_source(source, agent) {
        return None;
    }

    if agent == "pi" || agent == "omp" {
        return _agent_session_path
            .and_then(AgentSessionRef::path)
            .or_else(|| agent_session_id.and_then(AgentSessionRef::id));
    }

    agent_session_id.and_then(AgentSessionRef::id)
}

/// The Codex conversation named by `herdr agent start --kind codex -- <args>`.
/// `--last` and `--remote` leave the conversation unknown until the hook
/// reports it.
pub fn persisted_session_from_launch_args(
    agent: crate::detect::Agent,
    args: &[String],
) -> Option<PersistedAgentSession> {
    if agent != crate::detect::Agent::Codex {
        return None;
    }
    let launch = parse_codex_args(args).ok()?;
    if !launch.resume || launch.last || launch.remote {
        return None;
    }

    Some(PersistedAgentSession {
        source: "herdr:codex".into(),
        agent: "codex".into(),
        session_ref: AgentSessionRef::id(launch.session_id?)?,
    })
}

/// The options a Claude or Codex process was launched with, without the parts
/// that pick or start a conversation, a one-time prompt, or print-and-exit mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentLaunchOptions {
    Known(Vec<String>),
    Unavailable(LaunchOptionsUnavailable),
}

/// Why restore cannot repeat an agent's launch options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchOptionsUnavailable {
    /// Nothing recorded them, for example a session file from an older Herdr.
    NotRecorded,
    /// Herdr does not read this process's command line, for example inside WSL.
    CommandLineUnreadable,
    /// The command line does not have a shape Herdr knows for this agent.
    UnrecognizedCommand,
    /// An option Herdr cannot classify, so it cannot tell its value from a prompt.
    UnsupportedOption,
    /// The options do not fit the restore command limits.
    ExceedsResumeLimits,
}

/// Whether a Claude or Codex restore repeated the agent's launch options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchOptionsRestore {
    Kept,
    NotKept(LaunchOptionsUnavailable),
}

/// Launch options from a running process's full command line (`argv[0]`
/// included). `None` for agents whose restore does not carry launch options.
pub fn launch_options_from_process_argv(
    agent: crate::detect::Agent,
    argv: Option<&[String]>,
) -> Option<AgentLaunchOptions> {
    if !carries_launch_options(agent) {
        return None;
    }
    let Some(argv) = argv else {
        return Some(AgentLaunchOptions::Unavailable(
            LaunchOptionsUnavailable::CommandLineUnreadable,
        ));
    };
    match agent_args_start(argv) {
        Some(start) => launch_options_from_agent_args(agent, &argv[start..]),
        None => Some(AgentLaunchOptions::Unavailable(
            LaunchOptionsUnavailable::UnrecognizedCommand,
        )),
    }
}

/// Launch options from the arguments after the agent executable, as given to
/// `herdr agent start -- <args>`. `None` for agents whose restore does not
/// carry launch options.
pub fn launch_options_from_agent_args(
    agent: crate::detect::Agent,
    args: &[String],
) -> Option<AgentLaunchOptions> {
    let parsed = match agent {
        crate::detect::Agent::Claude => parse_claude_args(args),
        crate::detect::Agent::Codex => parse_codex_args(args).map(|launch| launch.options),
        _ => return None,
    };
    Some(match parsed {
        Ok(options) => AgentLaunchOptions::Known(options),
        Err(reason) => AgentLaunchOptions::Unavailable(reason),
    })
}

/// The restore plan for a saved session together with its launch options.
/// The outcome is `None` for agents whose restore does not carry launch options.
pub fn plan_with_launch_options(
    source: &str,
    agent: &str,
    session_ref: &AgentSessionRef,
    launch_options: Option<&[String]>,
) -> Option<(AgentResumePlan, Option<LaunchOptionsRestore>)> {
    let bare = plan(source, agent, session_ref)?;
    let claude = match (source, agent, session_ref.kind) {
        ("herdr:claude", "claude", AgentSessionRefKind::Id) => true,
        ("herdr:codex", "codex", AgentSessionRefKind::Id) => false,
        _ => return Some((bare, None)),
    };
    let Some(options) = launch_options else {
        return Some((
            bare,
            Some(LaunchOptionsRestore::NotKept(
                LaunchOptionsUnavailable::NotRecorded,
            )),
        ));
    };
    if options.is_empty() {
        return Some((bare, Some(LaunchOptionsRestore::Kept)));
    }

    // Codex takes the id directly after `resume` so no option value can
    // swallow it; Claude's `--resume` starts with a dash, which ends any
    // variadic option before it.
    let id = session_ref.value.clone();
    let argv: Vec<String> = if claude {
        std::iter::once("claude".to_string())
            .chain(options.iter().cloned())
            .chain(["--resume".to_string(), id])
            .collect()
    } else {
        ["codex".to_string(), "resume".to_string(), id]
            .into_iter()
            .chain(options.iter().cloned())
            .collect()
    };
    if validate_resume_argv(&argv).is_err() {
        return Some((
            bare,
            Some(LaunchOptionsRestore::NotKept(
                LaunchOptionsUnavailable::ExceedsResumeLimits,
            )),
        ));
    }
    Some((
        AgentResumePlan { argv, ..bare },
        Some(LaunchOptionsRestore::Kept),
    ))
}

fn carries_launch_options(agent: crate::detect::Agent) -> bool {
    matches!(
        agent,
        crate::detect::Agent::Claude | crate::detect::Agent::Codex
    )
}

/// Where the agent's own arguments start in a process command line: after a
/// native executable, or after a JavaScript runtime, its flags and the script.
fn agent_args_start(argv: &[String]) -> Option<usize> {
    let executable = argv.first()?;
    let name = executable
        .rsplit(['/', '\\'])
        .find(|component| !component.is_empty())
        .unwrap_or(executable)
        .to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    match name {
        "node" | "nodejs" | "bun" | "deno" => {}
        // A shell or another interpreter runs something Herdr cannot place.
        "sh" | "bash" | "zsh" | "fish" | "dash" | "env" | "tmux" | "cmd" | "powershell"
        | "pwsh" => return None,
        name if name.starts_with("python") => return None,
        _ => return Some(1),
    }

    let mut index = 1;
    loop {
        let token = argv.get(index)?;
        match token.as_str() {
            // No script: the agent's code is not in this command line.
            "-e" | "--eval" | "-p" | "--print" | "-i" | "--interactive" => return None,
            // Runtime flags whose value is the next argument.
            "-r"
            | "--require"
            | "--import"
            | "--loader"
            | "--experimental-loader"
            | "-C"
            | "--conditions" => index += 2,
            // `bun run <script>` and `deno run [flags] <script>`.
            "run" if matches!(name, "bun" | "deno") => index += 1,
            "--" => index += 1,
            token if token.starts_with('-') => index += 1,
            _ => return Some(index + 1),
        }
    }
}

/// How many arguments a launch option takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OptionArity {
    Flag,
    /// One value, as the next argument, after `=`, or attached to a short name.
    Value,
    /// Commander `[value]`: the next argument only when it does not start with `-`.
    OptionalValue,
    /// Commander `<x...>` and clap `num_args(1..)`: one value, then every
    /// following argument that does not start with `-`.
    Variadic,
}

#[derive(Debug)]
struct LaunchOptionSpec {
    names: &'static [&'static str],
    arity: OptionArity,
    /// Repeated on restore; otherwise dropped.
    keep: bool,
    /// Codex only: accepted before a subcommand. `codex resume` accepts
    /// every option.
    top_level: bool,
}

const fn kept(names: &'static [&'static str], arity: OptionArity) -> LaunchOptionSpec {
    LaunchOptionSpec {
        names,
        arity,
        keep: true,
        top_level: true,
    }
}

const fn dropped(names: &'static [&'static str], arity: OptionArity) -> LaunchOptionSpec {
    LaunchOptionSpec {
        names,
        arity,
        keep: false,
        top_level: true,
    }
}

const fn dropped_resume_only(
    names: &'static [&'static str],
    arity: OptionArity,
) -> LaunchOptionSpec {
    LaunchOptionSpec {
        names,
        arity,
        keep: false,
        top_level: false,
    }
}

/// Options of `claude --help` 2.1.302, plus the `-file` system prompt forms
/// that help text names.
const CLAUDE_OPTIONS: &[LaunchOptionSpec] = &[
    kept(&["--add-dir"], Variadic),
    kept(&["--agent"], Value),
    kept(&["--agents"], Value),
    kept(&["--allow-dangerously-skip-permissions"], Flag),
    kept(&["--allowedTools", "--allowed-tools"], Variadic),
    kept(&["--append-system-prompt"], Value),
    kept(&["--append-system-prompt-file"], Value),
    kept(&["--autocompact"], Value),
    kept(&["--ax-screen-reader"], Flag),
    kept(&["--bare"], Flag),
    kept(&["--betas"], Variadic),
    kept(&["--brief"], Flag),
    kept(&["--chrome"], Flag),
    kept(&["--dangerously-skip-permissions"], Flag),
    kept(&["-d", "--debug"], OptionalValue),
    kept(&["--debug-file"], Value),
    kept(&["--disable-slash-commands"], Flag),
    kept(&["--disallowedTools", "--disallowed-tools"], Variadic),
    kept(&["--effort"], Value),
    kept(&["--exclude-dynamic-system-prompt-sections"], Flag),
    kept(&["--fallback-model"], Value),
    kept(&["--ide"], Flag),
    kept(&["--mcp-config"], Variadic),
    kept(&["--model"], Value),
    kept(&["-n", "--name"], Value),
    kept(&["--no-chrome"], Flag),
    kept(&["--permission-mode"], Value),
    kept(&["--plugin-dir"], Value),
    kept(&["--plugin-url"], Value),
    kept(&["--prompt-suggestions"], OptionalValue),
    kept(&["--remote-control"], OptionalValue),
    kept(&["--remote-control-session-name-prefix"], Value),
    kept(&["--restricted"], Flag),
    kept(&["--safe-mode"], Flag),
    kept(&["--setting-sources"], Value),
    kept(&["--settings"], Value),
    kept(&["--strict-mcp-config"], Flag),
    kept(&["--system-prompt"], Value),
    kept(&["--system-prompt-file"], Value),
    kept(&["--system-prompt-snapshot"], Value),
    kept(&["--tools"], Variadic),
    kept(&["--verbose"], Flag),
    // They pick or start a conversation; the saved session id takes their place.
    dropped(&["-r", "--resume"], OptionalValue),
    dropped(&["-c", "--continue"], Flag),
    dropped(&["--session-id"], Value),
    dropped(&["--fork-session"], Flag),
    dropped(&["--from-pr"], OptionalValue),
    // Print-and-exit mode and the options that only work with it.
    dropped(&["-p", "--print"], Flag),
    dropped(&["--output-format"], Value),
    dropped(&["--input-format"], Value),
    dropped(&["--include-partial-messages"], Flag),
    dropped(&["--include-hook-events"], Flag),
    dropped(&["--replay-user-messages"], Flag),
    dropped(&["--forward-subagent-text"], Flag),
    dropped(&["--json-schema"], Value),
    dropped(&["--max-budget-usd"], Value),
    dropped(&["--no-session-persistence"], Flag),
    dropped(&["--permission-prompts"], Value),
    // They run the session somewhere other than this pane or in a new worktree.
    dropped(&["--bg", "--background"], Flag),
    dropped(&["--cloud"], OptionalValue),
    dropped(&["--desktop"], Flag),
    dropped(&["--environment"], Value),
    dropped(&["--teleport"], OptionalValue),
    dropped(&["-w", "--worktree"], OptionalValue),
    dropped(&["--tmux"], Flag),
    // Downloads files once at startup, like a prompt attachment.
    dropped(&["--file"], Variadic),
    // Print help or the version and exit.
    dropped(&["-h", "--help"], Flag),
    dropped(&["-v", "--version"], Flag),
];

const CLAUDE_SUBCOMMANDS: &[&str] = &[
    "agents",
    "attach",
    "auth",
    "auto-mode",
    "doctor",
    "gateway",
    "help",
    "import",
    "install",
    "kill",
    "logs",
    "mcp",
    "plugin",
    "plugins",
    "purge",
    "respawn",
    "rm",
    "setup-token",
    "stop",
    "ultrareview",
    "update",
    "upgrade",
];

/// Options of `codex --help` and `codex resume --help` 0.163.0. A kept
/// option must be accepted by `codex resume`, since restore runs that.
const CODEX_OPTIONS: &[LaunchOptionSpec] = &[
    kept(&["-c", "--config"], Value),
    kept(&["--enable"], Value),
    kept(&["--disable"], Value),
    kept(&["--remote"], Value),
    kept(&["--remote-auth-token-env"], Value),
    kept(&["--strict-config"], Flag),
    kept(&["-m", "--model"], Value),
    kept(&["--oss"], Flag),
    kept(&["--local-provider"], Value),
    kept(&["-p", "--profile"], Value),
    kept(&["-s", "--sandbox"], Value),
    kept(&["--approve-for-me"], Flag),
    kept(&["--dangerously-bypass-approvals-and-sandbox"], Flag),
    kept(&["--dangerously-bypass-hook-trust"], Flag),
    kept(&["-C", "--cd"], Value),
    kept(&["--add-dir"], Value),
    kept(&["-a", "--ask-for-approval"], Value),
    kept(&["--search"], Flag),
    kept(&["--no-alt-screen"], Flag),
    kept(&["--no-daemon"], Flag),
    // They pick a conversation; the saved session id takes their place.
    dropped_resume_only(&["--last"], Flag),
    dropped_resume_only(&["--all"], Flag),
    dropped_resume_only(&["--include-non-interactive"], Flag),
    // Attaches images to the one-time initial prompt.
    dropped(&["-i", "--image"], Variadic),
    // Starts the session in a new managed Git worktree.
    dropped(&["--worktree"], Flag),
    // Print help or the version and exit.
    dropped(&["-h", "--help"], Flag),
    dropped(&["-V", "--version"], Flag),
];

const CODEX_SUBCOMMANDS: &[&str] = &[
    "a",
    "agents",
    "app-server",
    "apply",
    "archive",
    "cloud",
    "completion",
    "debug",
    "delete",
    "doctor",
    "e",
    "exec",
    "exec-server",
    "features",
    "fork",
    "help",
    "login",
    "logout",
    "mcp",
    "migrate-rollouts",
    "plugin",
    "queue",
    "remote-control",
    "review",
    "sandbox",
    "unarchive",
    "update",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParserStyle {
    Commander,
    Clap,
}

/// One argument from a launch command line.
enum LaunchArg {
    /// An option and the number of arguments it used, itself included.
    Option {
        spec: &'static LaunchOptionSpec,
        len: usize,
    },
    Operand,
    /// `--`: every later argument is an operand.
    EndOfOptions,
}

fn find_option(
    specs: &'static [LaunchOptionSpec],
    name: &str,
) -> Option<&'static LaunchOptionSpec> {
    specs.iter().find(|spec| spec.names.contains(&name))
}

fn looks_like_option(arg: &str) -> bool {
    arg.len() > 1 && arg.starts_with('-')
}

/// The extra arguments a variadic option takes after its first value.
fn variadic_tail(args: &[String]) -> usize {
    args.iter()
        .take_while(|arg| !looks_like_option(arg))
        .count()
}

/// Classifies `args[0]`, following commander's `parseOptions` (Claude) and
/// clap (Codex).
fn next_launch_arg(
    specs: &'static [LaunchOptionSpec],
    style: ParserStyle,
    args: &[String],
) -> Result<LaunchArg, LaunchOptionsUnavailable> {
    use LaunchOptionsUnavailable::UnsupportedOption;
    let Some(arg) = args.first() else {
        return Err(UnsupportedOption);
    };
    if arg == "--" {
        return Ok(LaunchArg::EndOfOptions);
    }
    if !looks_like_option(arg) {
        return Ok(LaunchArg::Operand);
    }
    let rest = &args[1..];
    // A value given inline cannot continue into the next arguments in
    // commander; in clap it is unclear, and they might be the session id.
    let inline_value = |spec: &'static LaunchOptionSpec| {
        if spec.arity == Variadic && style == ParserStyle::Clap && variadic_tail(rest) > 0 {
            return Err(UnsupportedOption);
        }
        Ok(LaunchArg::Option { spec, len: 1 })
    };

    if let Some(spec) = find_option(specs, arg) {
        let len = match spec.arity {
            Flag => 1,
            Value if rest.is_empty() => return Err(UnsupportedOption),
            Value => 2,
            OptionalValue => match rest.first() {
                Some(value) if !looks_like_option(value) => 2,
                _ => 1,
            },
            Variadic if rest.is_empty() => return Err(UnsupportedOption),
            Variadic => 2 + variadic_tail(&rest[1..]),
        };
        return Ok(LaunchArg::Option { spec, len });
    }

    if let Some((name, _)) = arg.split_once('=').filter(|_| arg.starts_with("--")) {
        let spec = find_option(specs, name).ok_or(UnsupportedOption)?;
        return match spec.arity {
            // Neither parser accepts a value for a flag; a dropped one is
            // harmless to drop with it (Claude's `--tmux=classic`).
            Flag if spec.keep => Err(UnsupportedOption),
            Flag => Ok(LaunchArg::Option { spec, len: 1 }),
            _ => inline_value(spec),
        };
    }

    if arg.starts_with("--") {
        return Err(UnsupportedOption);
    }

    // A cluster of short options such as `-mVALUE` or commander's `-cd`.
    // Every option in one argument must agree on whether it is kept.
    let mut cluster = &arg[1..];
    let mut keep = None;
    loop {
        let Some(short) = cluster.chars().next() else {
            return Err(UnsupportedOption);
        };
        let spec = find_option(specs, &format!("-{short}")).ok_or(UnsupportedOption)?;
        if keep.is_some_and(|keep| keep != spec.keep) {
            return Err(UnsupportedOption);
        }
        keep = Some(spec.keep);
        let after = &cluster[short.len_utf8()..];
        if after.is_empty() {
            // The last short option reads the next arguments like `-x` alone.
            let next = std::iter::once(format!("-{short}"))
                .chain(rest.iter().cloned())
                .collect::<Vec<_>>();
            return next_launch_arg(specs, style, &next);
        }
        if spec.arity != Flag {
            return inline_value(spec);
        }
        cluster = after;
    }
}

/// The launch options in a Claude command line. Commander parses options
/// anywhere; the first operand is a subcommand or the one-time prompt, and
/// every later operand is dropped with it. A variadic option such as
/// `--add-dir a b` takes every following argument up to the next one that
/// starts with `-`, so a prompt typed after it is one of its values to Claude
/// too, and is kept as such.
fn parse_claude_args(args: &[String]) -> Result<Vec<String>, LaunchOptionsUnavailable> {
    let mut options = Vec::new();
    let mut operands = 0;
    let mut index = 0;
    while index < args.len() {
        match next_launch_arg(CLAUDE_OPTIONS, ParserStyle::Commander, &args[index..])? {
            LaunchArg::Option { spec, len } => {
                if spec.keep {
                    options.extend_from_slice(&args[index..index + len]);
                }
                index += len;
            }
            LaunchArg::Operand => {
                if operands == 0 && CLAUDE_SUBCOMMANDS.contains(&args[index].as_str()) {
                    return Err(LaunchOptionsUnavailable::UnrecognizedCommand);
                }
                operands += 1;
                index += 1;
            }
            LaunchArg::EndOfOptions => break,
        }
    }
    Ok(options)
}

#[derive(Debug, Default)]
struct CodexLaunch {
    options: Vec<String>,
    resume: bool,
    session_id: Option<String>,
    last: bool,
    remote: bool,
}

/// The parts of a Codex command line: top-level options, then either a
/// one-time prompt or `resume [SESSION_ID] [PROMPT]` with its options.
fn parse_codex_args(args: &[String]) -> Result<CodexLaunch, LaunchOptionsUnavailable> {
    use LaunchOptionsUnavailable::{UnrecognizedCommand, UnsupportedOption};
    let mut launch = CodexLaunch::default();
    let mut operands = 0;
    let mut options_ended = false;
    let mut index = 0;
    while index < args.len() {
        let arg = if options_ended {
            LaunchArg::Operand
        } else {
            next_launch_arg(CODEX_OPTIONS, ParserStyle::Clap, &args[index..])?
        };
        match arg {
            LaunchArg::Option { spec, len } => {
                if !launch.resume && !spec.top_level {
                    return Err(UnsupportedOption);
                }
                match spec.names[0] {
                    "--last" => launch.last = true,
                    "--remote" => launch.remote = true,
                    _ => {}
                }
                if spec.keep {
                    launch.options.extend_from_slice(&args[index..index + len]);
                }
                index += len;
            }
            LaunchArg::Operand => {
                let operand = &args[index];
                if launch.resume {
                    if operands == 0 {
                        launch.session_id = Some(operand.clone());
                    }
                    operands += 1;
                } else if operands == 0 && !options_ended && operand == "resume" {
                    launch.resume = true;
                } else if operands == 0
                    && !options_ended
                    && CODEX_SUBCOMMANDS.contains(&operand.as_str())
                {
                    return Err(UnrecognizedCommand);
                } else {
                    operands += 1;
                }
                index += 1;
            }
            LaunchArg::EndOfOptions => {
                options_ended = true;
                index += 1;
            }
        }
    }
    Ok(launch)
}

pub fn normalize_session_start_source(value: Option<String>) -> Option<String> {
    match value.as_deref().map(str::trim) {
        Some(
            source @ ("startup" | "resume" | "clear" | "compact" | "branch" | "new" | "fork"
            | "select"),
        ) => Some(source.to_string()),
        _ => None,
    }
}

pub fn is_reserved_native_state_source(source: &str, agent: &str) -> bool {
    matches!(
        (source, agent),
        ("herdr:claude", "claude")
            | ("herdr:codex", "codex")
            | ("herdr:copilot", "copilot")
            | ("herdr:devin", "devin")
            | ("herdr:droid", "droid")
            | ("herdr:qodercli", "qodercli")
            | ("herdr:qwen", "qwen")
            | ("herdr:cursor", "cursor")
            | ("herdr:grok", "grok")
    )
}

pub fn session_ref_from_snapshot(
    source: &str,
    agent: &str,
    kind: AgentSessionRefKind,
    value: &str,
) -> Option<PersistedAgentSession> {
    if !is_official_agent_source(source, agent) {
        return None;
    }
    let session_ref = match (agent, kind) {
        ("pi" | "omp", AgentSessionRefKind::Path) => AgentSessionRef::path(value)?,
        (_, AgentSessionRefKind::Id) => AgentSessionRef::id(value)?,
        _ => return None,
    };
    Some(PersistedAgentSession {
        source: source.to_string(),
        agent: agent.to_string(),
        session_ref,
    })
}

pub fn plan(source: &str, agent: &str, session_ref: &AgentSessionRef) -> Option<AgentResumePlan> {
    if !is_official_agent_source(source, agent) {
        return None;
    }

    let argv = match (source, agent, session_ref.kind) {
        ("herdr:claude", "claude", AgentSessionRefKind::Id) => {
            vec![
                "claude".into(),
                "--resume".into(),
                session_ref.value.clone(),
            ]
        }
        ("herdr:codex", "codex", AgentSessionRefKind::Id) => {
            vec!["codex".into(), "resume".into(), session_ref.value.clone()]
        }
        ("herdr:copilot", "copilot", AgentSessionRefKind::Id) => {
            vec!["copilot".into(), format!("--resume={}", session_ref.value)]
        }
        ("herdr:devin", "devin", AgentSessionRefKind::Id) => {
            vec!["devin".into(), "--resume".into(), session_ref.value.clone()]
        }
        ("herdr:droid", "droid", AgentSessionRefKind::Id) => {
            vec!["droid".into(), "--resume".into(), session_ref.value.clone()]
        }
        ("herdr:kimi", "kimi", AgentSessionRefKind::Id) => {
            vec!["kimi".into(), "--session".into(), session_ref.value.clone()]
        }
        ("herdr:mastracode", "mastracode", AgentSessionRefKind::Id) => {
            vec![
                "mastracode".into(),
                "--thread".into(),
                session_ref.value.clone(),
            ]
        }
        ("herdr:pi", "pi", AgentSessionRefKind::Path | AgentSessionRefKind::Id) => {
            vec!["pi".into(), "--session".into(), session_ref.value.clone()]
        }
        ("herdr:omp", "omp", AgentSessionRefKind::Path | AgentSessionRefKind::Id) => {
            // omp resume is `-r, --resume=<value>` (ID prefix or path); it has no
            // `--session` flag, unlike pi.
            vec!["omp".into(), format!("--resume={}", session_ref.value)]
        }
        ("herdr:hermes", "hermes", AgentSessionRefKind::Id) => {
            vec![
                "hermes".into(),
                "--resume".into(),
                session_ref.value.clone(),
            ]
        }
        ("herdr:opencode", "opencode", AgentSessionRefKind::Id) => {
            vec![
                "opencode".into(),
                "--session".into(),
                session_ref.value.clone(),
            ]
        }
        ("herdr:qodercli", "qodercli", AgentSessionRefKind::Id) => {
            vec![
                "qodercli".into(),
                "--resume".into(),
                session_ref.value.clone(),
            ]
        }
        ("herdr:qwen", "qwen", AgentSessionRefKind::Id) => {
            vec!["qwen".into(), "--resume".into(), session_ref.value.clone()]
        }
        ("herdr:kilo", "kilo", AgentSessionRefKind::Id) => {
            vec!["kilo".into(), "--session".into(), session_ref.value.clone()]
        }
        ("herdr:cursor", "cursor", AgentSessionRefKind::Id) => {
            vec![
                if cfg!(windows) {
                    "cursor-agent.cmd"
                } else {
                    "cursor-agent"
                }
                .into(),
                "--resume".into(),
                session_ref.value.clone(),
            ]
        }
        ("herdr:antigravity_cli", "agy", AgentSessionRefKind::Id) => {
            vec![
                "agy".into(),
                "--conversation".into(),
                session_ref.value.clone(),
            ]
        }
        ("herdr:grok", "grok", AgentSessionRefKind::Id) => {
            vec!["grok".into(), "--resume".into(), session_ref.value.clone()]
        }
        ("herdr:letta", "letta", AgentSessionRefKind::Id) => {
            if let Some(agent_id) = session_ref.value.strip_prefix("default:") {
                if agent_id.is_empty() {
                    return None;
                }
                vec![
                    "letta".into(),
                    "--conversation".into(),
                    "default".into(),
                    "--agent".into(),
                    agent_id.into(),
                ]
            } else {
                vec![
                    "letta".into(),
                    "--conversation".into(),
                    session_ref.value.clone(),
                ]
            }
        }
        _ => return None,
    };

    Some(AgentResumePlan {
        agent: agent.to_string(),
        argv,
        dedupe_key: dedupe_key(source, agent, session_ref),
    })
}

pub fn dedupe_key(source: &str, agent: &str, session_ref: &AgentSessionRef) -> String {
    format!(
        "{source}\u{0}{agent}\u{0}{:?}\u{0}{}",
        session_ref.kind, session_ref.value
    )
}

pub(crate) fn is_official_agent_source(source: &str, agent: &str) -> bool {
    matches!(
        (source, agent),
        ("herdr:claude", "claude")
            | ("herdr:codex", "codex")
            | ("herdr:copilot", "copilot")
            | ("herdr:devin", "devin")
            | ("herdr:droid", "droid")
            | ("herdr:kimi", "kimi")
            | ("herdr:omp", "omp")
            | ("herdr:mastracode", "mastracode")
            | ("herdr:pi", "pi")
            | ("herdr:hermes", "hermes")
            | ("herdr:opencode", "opencode")
            | ("herdr:qodercli", "qodercli")
            | ("herdr:qwen", "qwen")
            | ("herdr:kilo", "kilo")
            | ("herdr:cursor", "cursor")
            | ("herdr:antigravity_cli", "agy")
            | ("herdr:grok", "grok")
            | ("herdr:letta", "letta")
    )
}

fn valid_session_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_SESSION_ID_LEN && !value.chars().any(char::is_control)
}

fn valid_session_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SESSION_PATH_LEN
        && !value.chars().any(char::is_control)
        && Path::new(value).is_absolute()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn absolute_test_path(name: &str) -> String {
        std::env::current_dir()
            .unwrap()
            .join(name)
            .display()
            .to_string()
    }

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| part.to_string()).collect()
    }

    #[test]
    fn reported_resume_argv_requires_a_plain_command_name() {
        assert!(validate_resume_argv(&argv(&[
            "prime-agent",
            "--resume",
            "01a0de21",
            "--model",
            "gpt 6"
        ]))
        .is_ok());
        assert!(validate_resume_argv(&argv(&["cursor-agent.cmd", "--resume", "id"])).is_ok());

        for invalid in [
            argv(&[]),
            argv(&[""]),
            argv(&["/usr/bin/prime-agent", "--resume", "id"]),
            argv(&["C:\\Program Files\\Prime\\prime.exe"]),
            argv(&["prime agent"]),
            argv(&["-prime"]),
            argv(&["prime-agent", "bad\nline"]),
            argv(&["prime-agent", "--name", "can's session"]),
            vec!["prime-agent".to_string(); MAX_RESUME_ARGS + 1],
            argv(&["prime-agent", &"x".repeat(MAX_RESUME_ARGV_BYTES)]),
        ] {
            assert!(validate_resume_argv(&invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn native_state_reservation_excludes_full_lifecycle_sources() {
        assert!(is_reserved_native_state_source("herdr:claude", "claude"));
        assert!(is_reserved_native_state_source("herdr:codex", "codex"));
        assert!(is_reserved_native_state_source("herdr:devin", "devin"));
        assert!(!is_reserved_native_state_source("herdr:kimi", "kimi"));
        assert!(!is_reserved_native_state_source(
            "herdr:opencode",
            "opencode"
        ));
    }

    #[test]
    fn codex_noncanonical_resume_launch_has_no_explicit_session() {
        assert_eq!(
            persisted_session_from_launch_args(
                crate::detect::Agent::Codex,
                &["resume".into(), "codex-session".into()]
            )
            .unwrap()
            .session_ref
            .value,
            "codex-session"
        );
        assert!(persisted_session_from_launch_args(
            crate::detect::Agent::Codex,
            &["resume".into(), "--last".into()]
        )
        .is_none());
        assert!(persisted_session_from_launch_args(
            crate::detect::Agent::Codex,
            &["resume".into(), "not-a-session".into(), "--last".into()]
        )
        .is_none());
        assert!(persisted_session_from_launch_args(
            crate::detect::Agent::Codex,
            &[
                "--remote".into(),
                "ws://example.test".into(),
                "resume".into(),
                "remote-session".into(),
            ]
        )
        .is_none());
    }

    #[test]
    fn planner_allows_supported_agents() {
        let pi_session = absolute_test_path("pi-session.jsonl");
        let omp_session = absolute_test_path("omp-session.jsonl");
        assert_eq!(
            plan(
                "herdr:claude",
                "claude",
                &AgentSessionRef::id("claude-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["claude", "--resume", "claude-session"]
        );
        assert_eq!(
            plan(
                "herdr:codex",
                "codex",
                &AgentSessionRef::id("codex-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["codex", "resume", "codex-session"]
        );
        assert_eq!(
            plan(
                "herdr:copilot",
                "copilot",
                &AgentSessionRef::id("copilot-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["copilot", "--resume=copilot-session"]
        );
        assert_eq!(
            plan(
                "herdr:devin",
                "devin",
                &AgentSessionRef::id("devin-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["devin", "--resume", "devin-session"]
        );
        assert_eq!(
            plan(
                "herdr:droid",
                "droid",
                &AgentSessionRef::id("droid-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["droid", "--resume", "droid-session"]
        );
        assert_eq!(
            plan(
                "herdr:kimi",
                "kimi",
                &AgentSessionRef::id("kimi-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["kimi", "--session", "kimi-session"]
        );
        assert_eq!(
            plan(
                "herdr:mastracode",
                "mastracode",
                &AgentSessionRef::id("mastracode-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["mastracode", "--thread", "mastracode-session"]
        );
        assert_eq!(
            plan(
                "herdr:pi",
                "pi",
                &AgentSessionRef::path(&pi_session).unwrap()
            )
            .unwrap()
            .argv,
            vec!["pi", "--session", pi_session.as_str()]
        );
        assert_eq!(
            plan(
                "herdr:omp",
                "omp",
                &AgentSessionRef::path(&omp_session).unwrap()
            )
            .unwrap()
            .argv,
            vec!["omp", format!("--resume={omp_session}").as_str()]
        );
        assert_eq!(
            plan(
                "herdr:hermes",
                "hermes",
                &AgentSessionRef::id("hermes-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["hermes", "--resume", "hermes-session"]
        );
        assert_eq!(
            plan(
                "herdr:opencode",
                "opencode",
                &AgentSessionRef::id("opencode-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["opencode", "--session", "opencode-session"]
        );
        assert_eq!(
            plan(
                "herdr:qodercli",
                "qodercli",
                &AgentSessionRef::id("qoder-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["qodercli", "--resume", "qoder-session"]
        );
        assert_eq!(
            plan(
                "herdr:qwen",
                "qwen",
                &AgentSessionRef::id("qwen-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["qwen", "--resume", "qwen-session"]
        );
        assert_eq!(
            plan(
                "herdr:kilo",
                "kilo",
                &AgentSessionRef::id("kilo-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["kilo", "--session", "kilo-session"]
        );
        assert_eq!(
            plan(
                "herdr:cursor",
                "cursor",
                &AgentSessionRef::id("cursor-session").unwrap()
            )
            .unwrap()
            .argv,
            vec![
                if cfg!(windows) {
                    "cursor-agent.cmd"
                } else {
                    "cursor-agent"
                },
                "--resume",
                "cursor-session",
            ]
        );
        assert_eq!(
            plan(
                "herdr:antigravity_cli",
                "agy",
                &AgentSessionRef::id("agy-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["agy", "--conversation", "agy-session"]
        );
        assert_eq!(
            plan(
                "herdr:grok",
                "grok",
                &AgentSessionRef::id("grok-session").unwrap()
            )
            .unwrap()
            .argv,
            vec!["grok", "--resume", "grok-session"]
        );
        assert_eq!(
            plan(
                "herdr:letta",
                "letta",
                &AgentSessionRef::id("conversation-123").unwrap()
            )
            .unwrap()
            .argv,
            vec!["letta", "--conversation", "conversation-123"]
        );
        assert_eq!(
            plan(
                "herdr:letta",
                "letta",
                &AgentSessionRef::id("default:agent-123").unwrap()
            )
            .unwrap()
            .argv,
            vec!["letta", "--conversation", "default", "--agent", "agent-123"]
        );
        assert!(plan(
            "herdr:letta",
            "letta",
            &AgentSessionRef::id("default:").unwrap()
        )
        .is_none());
    }

    #[test]
    fn planner_rejects_custom_and_unsupported_path_refs() {
        let claude_session = absolute_test_path("claude-session");
        assert!(plan(
            "custom:claude",
            "claude",
            &AgentSessionRef::id("session").unwrap()
        )
        .is_none());
        assert!(plan(
            "herdr:claude",
            "claude",
            &AgentSessionRef::path(&claude_session).unwrap()
        )
        .is_none());
    }

    #[test]
    fn report_ref_prefers_pi_and_omp_paths_and_validates_values() {
        let pi_session = absolute_test_path("pi-session.jsonl");
        let omp_session = absolute_test_path("omp-session.jsonl");
        let claude_session = absolute_test_path("claude-session");
        let copilot_session = absolute_test_path("copilot-session");
        let session_ref = session_ref_from_report(
            "herdr:pi",
            "pi",
            Some("pi-id".into()),
            Some(pi_session.clone()),
        )
        .unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Path);
        assert_eq!(session_ref.value, pi_session);

        assert!(session_ref_from_report("herdr:pi", "pi", Some("bad\nid".into()), None).is_none());
        assert!(
            session_ref_from_report("herdr:pi", "pi", None, Some("relative.jsonl".into()))
                .is_none()
        );
        assert!(session_ref_from_report("custom:pi", "pi", Some("pi-id".into()), None).is_none());

        let session_ref = session_ref_from_report(
            "herdr:omp",
            "omp",
            Some("omp-id".into()),
            Some(omp_session.clone()),
        )
        .unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Path);
        assert_eq!(session_ref.value, omp_session);

        let session_ref =
            session_ref_from_report("herdr:omp", "omp", Some("omp-id".into()), None).unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Id);
        assert_eq!(session_ref.value, "omp-id");
        let session_ref = session_ref_from_report(
            "herdr:omp",
            "omp",
            Some("omp-id".into()),
            Some("relative.jsonl".into()),
        )
        .unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Id);
        assert_eq!(session_ref.value, "omp-id");
        assert!(
            session_ref_from_report("herdr:omp", "omp", None, Some("relative.jsonl".into()))
                .is_none()
        );

        assert!(
            session_ref_from_report("herdr:claude", "claude", None, Some(claude_session)).is_none()
        );

        let session_ref =
            session_ref_from_report("herdr:copilot", "copilot", Some("copilot-id".into()), None)
                .unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Id);
        assert_eq!(session_ref.value, "copilot-id");
        assert!(
            session_ref_from_report("herdr:copilot", "copilot", None, Some(copilot_session))
                .is_none()
        );

        let session_ref =
            session_ref_from_report("herdr:devin", "devin", Some("devin-id".into()), None).unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Id);
        assert_eq!(session_ref.value, "devin-id");

        let session_ref =
            session_ref_from_report("herdr:droid", "droid", Some("droid-id".into()), None).unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Id);
        assert_eq!(session_ref.value, "droid-id");
        assert!(session_ref_from_report(
            "herdr:droid",
            "droid",
            None,
            Some("/tmp/droid-session".into())
        )
        .is_none());

        let session_ref =
            session_ref_from_report("herdr:kimi", "kimi", Some("kimi-id".into()), None).unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Id);
        assert_eq!(session_ref.value, "kimi-id");

        let session_ref = session_ref_from_report(
            "herdr:mastracode",
            "mastracode",
            Some("mastracode-id".into()),
            None,
        )
        .unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Id);
        assert_eq!(session_ref.value, "mastracode-id");

        let session_ref =
            session_ref_from_report("herdr:kilo", "kilo", Some("kilo-id".into()), None).unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Id);
        assert_eq!(session_ref.value, "kilo-id");

        let session_ref =
            session_ref_from_report("herdr:qodercli", "qodercli", Some("qoder-id".into()), None)
                .unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Id);
        assert_eq!(session_ref.value, "qoder-id");

        let session_ref =
            session_ref_from_report("herdr:qwen", "qwen", Some("qwen-id".into()), None).unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Id);
        assert_eq!(session_ref.value, "qwen-id");

        let session_ref =
            session_ref_from_report("herdr:antigravity_cli", "agy", Some("agy-id".into()), None)
                .unwrap();
        assert_eq!(session_ref.kind, AgentSessionRefKind::Id);
        assert_eq!(session_ref.value, "agy-id");
    }

    #[test]
    fn normalize_session_start_source_allows_known_values() {
        assert_eq!(
            normalize_session_start_source(Some("startup".into())),
            Some("startup".into())
        );
        assert_eq!(
            normalize_session_start_source(Some("resume".into())),
            Some("resume".into())
        );
        assert_eq!(
            normalize_session_start_source(Some("clear".into())),
            Some("clear".into())
        );
        assert_eq!(
            normalize_session_start_source(Some("compact".into())),
            Some("compact".into())
        );
        assert_eq!(
            normalize_session_start_source(Some("branch".into())),
            Some("branch".into())
        );
        assert_eq!(
            normalize_session_start_source(Some("new".into())),
            Some("new".into())
        );
        assert_eq!(
            normalize_session_start_source(Some("fork".into())),
            Some("fork".into())
        );
        assert_eq!(
            normalize_session_start_source(Some("select".into())),
            Some("select".into())
        );
        assert_eq!(
            normalize_session_start_source(Some(" resume ".into())),
            Some("resume".into())
        );
        assert_eq!(normalize_session_start_source(Some("other".into())), None);
        assert_eq!(normalize_session_start_source(None), None);
    }

    #[test]
    fn ids_are_data_not_shell_text() {
        let id = "abc; rm -rf /";
        let codex_plan = plan("herdr:codex", "codex", &AgentSessionRef::id(id).unwrap()).unwrap();
        assert_eq!(codex_plan.argv, vec!["codex", "resume", id]);

        let copilot_plan = plan(
            "herdr:copilot",
            "copilot",
            &AgentSessionRef::id(id).unwrap(),
        )
        .unwrap();
        assert_eq!(copilot_plan.argv, vec!["copilot", "--resume=abc; rm -rf /"]);

        let devin_plan = plan("herdr:devin", "devin", &AgentSessionRef::id(id).unwrap()).unwrap();
        assert_eq!(devin_plan.argv, vec!["devin", "--resume", id]);
    }

    #[test]
    fn planner_rejects_path_refs_for_id_only_agents() {
        let hermes_session = absolute_test_path("hermes-session");
        let opencode_session = absolute_test_path("opencode-session");
        let kilo_session = absolute_test_path("kilo-session");
        let copilot_session = absolute_test_path("copilot-session");
        let devin_session = absolute_test_path("devin-session");
        assert!(plan(
            "herdr:hermes",
            "hermes",
            &AgentSessionRef::path(&hermes_session).unwrap()
        )
        .is_none());
        assert!(plan(
            "herdr:opencode",
            "opencode",
            &AgentSessionRef::path(&opencode_session).unwrap()
        )
        .is_none());
        assert!(plan(
            "herdr:kilo",
            "kilo",
            &AgentSessionRef::path(&kilo_session).unwrap()
        )
        .is_none());
        assert!(plan(
            "herdr:copilot",
            "copilot",
            &AgentSessionRef::path(&copilot_session).unwrap()
        )
        .is_none());
        assert!(plan(
            "herdr:devin",
            "devin",
            &AgentSessionRef::path(&devin_session).unwrap()
        )
        .is_none());
        assert!(session_ref_from_snapshot(
            "herdr:mastracode",
            "mastracode",
            AgentSessionRefKind::Id,
            "mastracode-session"
        )
        .is_some());
        assert!(session_ref_from_snapshot(
            "herdr:hermes",
            "hermes",
            AgentSessionRefKind::Id,
            "hermes-session"
        )
        .is_some());
        assert!(session_ref_from_snapshot(
            "herdr:opencode",
            "opencode",
            AgentSessionRefKind::Id,
            "opencode-session"
        )
        .is_some());
        assert!(session_ref_from_snapshot(
            "herdr:kilo",
            "kilo",
            AgentSessionRefKind::Id,
            "kilo-session"
        )
        .is_some());
        assert!(session_ref_from_snapshot(
            "herdr:copilot",
            "copilot",
            AgentSessionRefKind::Id,
            "copilot-session"
        )
        .is_some());
        assert!(session_ref_from_snapshot(
            "herdr:devin",
            "devin",
            AgentSessionRefKind::Id,
            "devin-session"
        )
        .is_some());
        assert!(session_ref_from_snapshot(
            "herdr:antigravity_cli",
            "agy",
            AgentSessionRefKind::Id,
            "agy-session"
        )
        .is_some());
        let agy_session = absolute_test_path("agy-session");
        assert!(plan(
            "herdr:antigravity_cli",
            "agy",
            &AgentSessionRef::path(&agy_session).unwrap()
        )
        .is_none());
    }

    use crate::detect::Agent;

    fn claude_options(args: &[&str]) -> AgentLaunchOptions {
        launch_options_from_agent_args(Agent::Claude, &argv(args)).unwrap()
    }

    fn codex_options(args: &[&str]) -> AgentLaunchOptions {
        launch_options_from_agent_args(Agent::Codex, &argv(args)).unwrap()
    }

    fn known(args: &[&str]) -> AgentLaunchOptions {
        AgentLaunchOptions::Known(argv(args))
    }

    fn unavailable(reason: LaunchOptionsUnavailable) -> AgentLaunchOptions {
        AgentLaunchOptions::Unavailable(reason)
    }

    fn restore(
        source: &str,
        agent: &str,
        id: &str,
        options: Option<&[&str]>,
    ) -> (AgentResumePlan, Option<LaunchOptionsRestore>) {
        let options = options.map(argv);
        plan_with_launch_options(
            source,
            agent,
            &AgentSessionRef::id(id).unwrap(),
            options.as_deref(),
        )
        .unwrap()
    }

    #[test]
    fn claude_bypass_launch_options_are_kept_on_restore() {
        assert_eq!(
            claude_options(&["--dangerously-skip-permissions"]),
            known(&["--dangerously-skip-permissions"])
        );
        assert_eq!(
            claude_options(&["--permission-mode", "bypassPermissions"]),
            known(&["--permission-mode", "bypassPermissions"])
        );
        assert_eq!(
            claude_options(&["--permission-mode=bypassPermissions"]),
            known(&["--permission-mode=bypassPermissions"])
        );
        let (plan, outcome) = restore(
            "herdr:claude",
            "claude",
            "claude-session",
            Some(&["--dangerously-skip-permissions"]),
        );
        assert_eq!(
            plan.argv,
            vec![
                "claude",
                "--dangerously-skip-permissions",
                "--resume",
                "claude-session"
            ]
        );
        assert_eq!(outcome, Some(LaunchOptionsRestore::Kept));
    }

    #[test]
    fn codex_bypass_launch_options_are_kept_on_restore() {
        assert_eq!(
            codex_options(&["--dangerously-bypass-approvals-and-sandbox"]),
            known(&["--dangerously-bypass-approvals-and-sandbox"])
        );
        let (plan, outcome) = restore(
            "herdr:codex",
            "codex",
            "codex-session",
            Some(&["--dangerously-bypass-approvals-and-sandbox"]),
        );
        assert_eq!(
            plan.argv,
            vec![
                "codex",
                "resume",
                "codex-session",
                "--dangerously-bypass-approvals-and-sandbox"
            ]
        );
        assert_eq!(outcome, Some(LaunchOptionsRestore::Kept));
    }

    #[test]
    fn restrictive_launch_options_are_kept_on_restore() {
        for mode in ["plan", "default", "acceptEdits"] {
            assert_eq!(
                claude_options(&["--permission-mode", mode]),
                known(&["--permission-mode", mode])
            );
        }
        assert_eq!(
            codex_options(&["-s", "read-only", "-a", "on-request"]),
            known(&["-s", "read-only", "-a", "on-request"])
        );
        assert_eq!(
            codex_options(&["--sandbox=read-only", "-aon-request"]),
            known(&["--sandbox=read-only", "-aon-request"])
        );
        let (plan, _) = restore(
            "herdr:codex",
            "codex",
            "codex-session",
            Some(&["-s", "read-only", "-a", "on-request"]),
        );
        assert_eq!(
            plan.argv,
            vec![
                "codex",
                "resume",
                "codex-session",
                "-s",
                "read-only",
                "-a",
                "on-request"
            ]
        );
    }

    #[test]
    fn launch_without_permission_options_restores_without_them() {
        assert_eq!(
            claude_options(&["--model", "opus"]),
            known(&["--model", "opus"])
        );
        assert_eq!(claude_options(&[]), known(&[]));
        assert_eq!(codex_options(&[]), known(&[]));
        let (plan, outcome) = restore("herdr:claude", "claude", "claude-session", Some(&[]));
        assert_eq!(plan.argv, vec!["claude", "--resume", "claude-session"]);
        assert_eq!(outcome, Some(LaunchOptionsRestore::Kept));
        let (plan, outcome) = restore("herdr:codex", "codex", "codex-session", Some(&[]));
        assert_eq!(plan.argv, vec!["codex", "resume", "codex-session"]);
        assert_eq!(outcome, Some(LaunchOptionsRestore::Kept));
        let (plan, _) = restore(
            "herdr:claude",
            "claude",
            "claude-session",
            Some(&["--model", "opus"]),
        );
        assert!(!plan.argv.iter().any(|arg| arg.contains("permission")));
    }

    #[test]
    fn model_and_effort_launch_options_are_kept_on_restore() {
        assert_eq!(
            claude_options(&["--model", "claude-placeholder", "--effort", "xhigh"]),
            known(&["--model", "claude-placeholder", "--effort", "xhigh"])
        );
        assert_eq!(
            codex_options(&["-m", "gpt-placeholder", "-c", "model_reasoning_effort=high"]),
            known(&["-m", "gpt-placeholder", "-c", "model_reasoning_effort=high"])
        );
        assert_eq!(
            codex_options(&["--model=gpt-placeholder", "-mgpt-other"]),
            known(&["--model=gpt-placeholder", "-mgpt-other"])
        );
        let (plan, outcome) = restore(
            "herdr:claude",
            "claude",
            "claude-session",
            Some(&["--model", "claude-placeholder", "--effort", "xhigh"]),
        );
        assert_eq!(
            plan.argv,
            vec![
                "claude",
                "--model",
                "claude-placeholder",
                "--effort",
                "xhigh",
                "--resume",
                "claude-session"
            ]
        );
        assert_eq!(outcome, Some(LaunchOptionsRestore::Kept));
        let (plan, _) = restore(
            "herdr:codex",
            "codex",
            "codex-session",
            Some(&["-m", "gpt-placeholder", "-c", "model_reasoning_effort=high"]),
        );
        assert_eq!(
            plan.argv,
            vec![
                "codex",
                "resume",
                "codex-session",
                "-m",
                "gpt-placeholder",
                "-c",
                "model_reasoning_effort=high"
            ]
        );
    }

    #[test]
    fn other_claude_launch_options_are_kept_exactly() {
        let args = [
            "--add-dir",
            "/placeholder/a",
            "/placeholder/b",
            "--allowedTools",
            "Bash(git *)",
            "Edit",
            "--disallowed-tools",
            "WebFetch",
            "--settings",
            r#"{"k":"placeholder"}"#,
            "--mcp-config",
            r#"{"mcpServers":{"placeholder":{"command":"placeholder"}}}"#,
            "--append-system-prompt",
            "placeholder text",
            "--system-prompt=placeholder system",
            "--setting-sources",
            "user,project",
            "--allow-dangerously-skip-permissions",
            "-d",
            "api,hooks",
            "--verbose",
            "-nplaceholder-name",
        ];
        assert_eq!(claude_options(&args), known(&args));
    }

    #[test]
    fn other_codex_launch_options_are_kept_exactly() {
        let args = [
            "--no-daemon",
            "--add-dir",
            "/placeholder/a",
            "-C",
            "/placeholder/work",
            "-p",
            "placeholder-profile",
            "--enable",
            "placeholder_feature",
            "--disable",
            "other_feature",
            "-c",
            r#"mcp_servers.placeholder.env={TOKEN="placeholder"}"#,
            "-c",
            "model_reasoning_effort=high",
            "--search",
        ];
        assert_eq!(codex_options(&args), known(&args));
    }

    #[test]
    fn claude_conversation_prompt_and_print_parts_are_dropped() {
        assert_eq!(
            claude_options(&[
                "--resume",
                "old-session",
                "--model",
                "opus",
                "-c",
                "--session-id",
                "00000000-0000-0000-0000-000000000000",
                "--fork-session",
                "--from-pr",
                "42",
                "-p",
                "--output-format",
                "json",
                "--input-format=text",
                "--include-partial-messages",
                "--max-budget-usd",
                "1",
                "--json-schema",
                "{}",
                "--no-session-persistence",
                "--bg",
                "--cloud",
                "--desktop",
                "--environment",
                "env-placeholder",
                "--teleport",
                "-w",
                "tree-name",
                "--tmux=classic",
                "--file",
                "file_placeholder:doc.txt",
                "one-time prompt",
            ]),
            known(&["--model", "opus"])
        );
        // The prompt comes first, then more options.
        assert_eq!(
            claude_options(&["fix the placeholder", "--effort", "high"]),
            known(&["--effort", "high"])
        );
        // `-r` without a value, then an option.
        assert_eq!(claude_options(&["-r", "--verbose"]), known(&["--verbose"]));
        // Everything after `--` is the prompt.
        assert_eq!(
            claude_options(&["--verbose", "--", "--model", "not-an-option"]),
            known(&["--verbose"])
        );
        // Short options combined in one argument, all dropped.
        assert_eq!(claude_options(&["-pc"]), known(&[]));
    }

    #[test]
    fn claude_prompt_after_variadic_option_is_one_of_its_values() {
        // Commander gives every argument after `--add-dir` that does not start
        // with `-` to `--add-dir`, so Claude itself reads this "prompt" as a
        // directory. Restore repeats exactly what Claude was given.
        assert_eq!(
            claude_options(&["--add-dir", "/placeholder", "fix the placeholder"]),
            known(&["--add-dir", "/placeholder", "fix the placeholder"])
        );
        // An inline value does not continue, so the prompt is dropped.
        assert_eq!(
            claude_options(&["--add-dir=/placeholder", "fix the placeholder"]),
            known(&["--add-dir=/placeholder"])
        );
    }

    #[test]
    fn codex_conversation_prompt_and_image_parts_are_dropped() {
        assert_eq!(
            codex_options(&[
                "--no-daemon",
                "resume",
                "codex-session",
                "one-time prompt",
                "--cd",
                "/placeholder"
            ]),
            known(&["--no-daemon", "--cd", "/placeholder"])
        );
        assert_eq!(
            codex_options(&["resume", "--no-daemon", "codex-session"]),
            known(&["--no-daemon"])
        );
        assert_eq!(
            codex_options(&[
                "resume",
                "--last",
                "--all",
                "--include-non-interactive",
                "-m",
                "x"
            ]),
            known(&["-m", "x"])
        );
        assert_eq!(
            codex_options(&[
                "-i",
                "/placeholder/a.png",
                "/placeholder/b.png",
                "--worktree",
                "-s",
                "read-only",
                "prompt"
            ]),
            known(&["-s", "read-only"])
        );
        assert_eq!(
            codex_options(&["one-time prompt", "-m", "x"]),
            known(&["-m", "x"])
        );
        // An inline image value followed by an argument clap might read as
        // another image or as the session id.
        assert_eq!(
            codex_options(&["resume", "--image=/placeholder/a.png", "codex-session"]),
            unavailable(LaunchOptionsUnavailable::UnsupportedOption)
        );
    }

    #[test]
    fn unknown_launch_option_makes_options_unavailable() {
        for args in [
            &["--model", "opus", "--not-a-claude-option"][..],
            &["--chrome=yes"][..],
            &["-x"][..],
            &["-cx"][..],
            &["--model"][..],
            // Kept and dropped short options in one argument.
            &["-cd"][..],
        ] {
            assert_eq!(
                claude_options(args),
                unavailable(LaunchOptionsUnavailable::UnsupportedOption),
                "{args:?}"
            );
        }
        for args in [
            &["--not-a-codex-option"][..],
            &["resume", "codex-session", "--bogus"][..],
            &["--no-daemon=1"][..],
            &["-m"][..],
            // `--last` is a `codex resume` option only.
            &["--last", "resume"][..],
        ] {
            assert_eq!(
                codex_options(args),
                unavailable(LaunchOptionsUnavailable::UnsupportedOption),
                "{args:?}"
            );
        }
    }

    #[test]
    fn launch_subcommands_make_options_unavailable() {
        for args in [
            &["mcp", "list"][..],
            &["--verbose", "doctor"][..],
            &["agents"][..],
        ] {
            assert_eq!(
                claude_options(args),
                unavailable(LaunchOptionsUnavailable::UnrecognizedCommand),
                "{args:?}"
            );
        }
        // A later operand is part of the prompt, not a subcommand.
        assert_eq!(claude_options(&["fix it", "doctor"]), known(&[]));
        for args in [
            &["exec", "prompt"][..],
            &["--no-daemon", "review"][..],
            &["fork", "codex-session"][..],
            &["e"][..],
        ] {
            assert_eq!(
                codex_options(args),
                unavailable(LaunchOptionsUnavailable::UnrecognizedCommand),
                "{args:?}"
            );
        }
        // After `--`, `exec` is the prompt.
        assert_eq!(codex_options(&["--", "exec"]), known(&[]));
    }

    #[test]
    fn launch_options_from_process_argv_handles_install_shapes() {
        for argv0 in [
            "claude",
            "/home/placeholder/.local/share/mise/installs/claude/2.1.300/claude",
            "/home/placeholder/.local/share/claude/versions/2.1.302",
            r"C:\Users\placeholder\.local\bin\claude.exe",
        ] {
            assert_eq!(
                launch_options_from_process_argv(
                    Agent::Claude,
                    Some(&argv(&[argv0, "--model", "opus", "--resume", "id"]))
                ),
                Some(known(&["--model", "opus"])),
                "{argv0}"
            );
        }
        assert_eq!(
            launch_options_from_process_argv(
                Agent::Claude,
                Some(&argv(&[
                    "node",
                    "--no-warnings",
                    "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js",
                    "--model",
                    "opus"
                ]))
            ),
            Some(known(&["--model", "opus"]))
        );
        for argv0 in [
            "/home/placeholder/.local/share/mise/installs/codex/0.162.1/bin/codex",
            "/usr/lib/node_modules/@openai/codex/vendor/x86_64-unknown-linux-musl/codex/codex",
            "codex.exe",
        ] {
            assert_eq!(
                launch_options_from_process_argv(
                    Agent::Codex,
                    Some(&argv(&[
                        argv0,
                        "--no-daemon",
                        "resume",
                        "id",
                        "--cd",
                        "/placeholder"
                    ]))
                ),
                Some(known(&["--no-daemon", "--cd", "/placeholder"])),
                "{argv0}"
            );
        }
        for runtime in [
            &["node", "/usr/lib/node_modules/@openai/codex/bin/codex.js"][..],
            &[
                "node.exe",
                "--require",
                "/placeholder/hook.js",
                r"C:\npm\codex.js",
            ][..],
            &["bun", "run", "/placeholder/codex.js"][..],
            &["deno", "run", "-A", "/placeholder/codex.js"][..],
            &["nodejs", "/placeholder/codex.js"][..],
        ] {
            let mut full = argv(runtime);
            full.extend(argv(&["resume", "id", "-m", "x"]));
            assert_eq!(
                launch_options_from_process_argv(Agent::Codex, Some(&full)),
                Some(known(&["-m", "x"])),
                "{runtime:?}"
            );
        }
        for shape in [
            &[][..],
            &["node"][..],
            &["node", "--no-warnings"][..],
            &["node", "-e", "placeholder()"][..],
            &["bash", "-c", "claude --model opus"][..],
            &["python3", "/placeholder/claude.py"][..],
        ] {
            assert_eq!(
                launch_options_from_process_argv(Agent::Claude, Some(&argv(shape))),
                Some(unavailable(LaunchOptionsUnavailable::UnrecognizedCommand)),
                "{shape:?}"
            );
        }
    }

    #[test]
    fn unreadable_command_line_makes_options_unavailable() {
        for agent in [Agent::Claude, Agent::Codex] {
            assert_eq!(
                launch_options_from_process_argv(agent, None),
                Some(unavailable(LaunchOptionsUnavailable::CommandLineUnreadable))
            );
        }
    }

    #[test]
    fn launch_options_exceeding_restore_limits_use_the_bare_command() {
        let apostrophe = ["--append-system-prompt", "the placeholder's text"];
        let many = vec!["--verbose"; MAX_RESUME_ARGS];
        let long = "x".repeat(MAX_RESUME_ARGV_BYTES);
        let large = ["--system-prompt", long.as_str()];
        let control = ["--system-prompt", "line\nbreak"];
        for options in [&apostrophe[..], &many[..], &large[..], &control[..]] {
            let (plan, outcome) =
                restore("herdr:claude", "claude", "claude-session", Some(options));
            assert_eq!(plan.argv, vec!["claude", "--resume", "claude-session"]);
            assert_eq!(
                outcome,
                Some(LaunchOptionsRestore::NotKept(
                    LaunchOptionsUnavailable::ExceedsResumeLimits
                ))
            );
            let (plan, outcome) = restore("herdr:codex", "codex", "codex-session", Some(options));
            assert_eq!(plan.argv, vec!["codex", "resume", "codex-session"]);
            assert_eq!(
                outcome,
                Some(LaunchOptionsRestore::NotKept(
                    LaunchOptionsUnavailable::ExceedsResumeLimits
                ))
            );
        }
    }

    #[test]
    fn unrecorded_launch_options_use_the_bare_command_with_notice() {
        let (plan, outcome) = restore("herdr:claude", "claude", "claude-session", None);
        assert_eq!(plan.argv, vec!["claude", "--resume", "claude-session"]);
        assert_eq!(
            outcome,
            Some(LaunchOptionsRestore::NotKept(
                LaunchOptionsUnavailable::NotRecorded
            ))
        );
        let (plan, outcome) = restore("herdr:codex", "codex", "codex-session", None);
        assert_eq!(plan.argv, vec!["codex", "resume", "codex-session"]);
        assert_eq!(
            outcome,
            Some(LaunchOptionsRestore::NotKept(
                LaunchOptionsUnavailable::NotRecorded
            ))
        );
    }

    #[test]
    fn launch_options_do_not_change_the_dedupe_key() {
        for (source, agent) in [("herdr:claude", "claude"), ("herdr:codex", "codex")] {
            let session_ref = AgentSessionRef::id("shared-session").unwrap();
            let (with_options, _) = restore(source, agent, "shared-session", Some(&["--verbose"]));
            let (without, _) = restore(source, agent, "shared-session", None);
            assert_eq!(
                with_options.dedupe_key,
                dedupe_key(source, agent, &session_ref)
            );
            assert_eq!(with_options.dedupe_key, without.dedupe_key);
        }
    }

    #[test]
    fn other_agents_restore_is_unchanged_by_launch_options() {
        let pi_session = absolute_test_path("pi-session.jsonl");
        for (source, agent, session_ref) in [
            (
                "herdr:copilot",
                "copilot",
                AgentSessionRef::id("s").unwrap(),
            ),
            ("herdr:devin", "devin", AgentSessionRef::id("s").unwrap()),
            (
                "herdr:pi",
                "pi",
                AgentSessionRef::path(&pi_session).unwrap(),
            ),
            (
                "herdr:letta",
                "letta",
                AgentSessionRef::id("default:a").unwrap(),
            ),
        ] {
            let options = argv(&["--model", "x"]);
            let (with_options, outcome) =
                plan_with_launch_options(source, agent, &session_ref, Some(&options)).unwrap();
            assert_eq!(with_options, plan(source, agent, &session_ref).unwrap());
            assert_eq!(outcome, None);
        }
        assert!(plan_with_launch_options(
            "custom:claude",
            "claude",
            &AgentSessionRef::id("s").unwrap(),
            Some(&[])
        )
        .is_none());
        for agent in [Agent::Pi, Agent::GithubCopilot, Agent::Gemini] {
            assert_eq!(
                launch_options_from_agent_args(agent, &argv(&["--model", "x"])),
                None
            );
            assert_eq!(launch_options_from_process_argv(agent, None), None);
        }
    }

    #[test]
    fn restored_command_line_yields_the_original_launch_options() {
        let claude = [
            "--dangerously-skip-permissions",
            "--add-dir",
            "/a",
            "/b",
            "--model",
            "opus",
        ];
        let (plan, _) = restore("herdr:claude", "claude", "claude-session", Some(&claude));
        assert_eq!(
            launch_options_from_process_argv(Agent::Claude, Some(&plan.argv)),
            Some(known(&claude))
        );
        let codex = ["--no-daemon", "-c", "model_reasoning_effort=high"];
        let (plan, _) = restore("herdr:codex", "codex", "codex-session", Some(&codex));
        assert_eq!(
            launch_options_from_process_argv(Agent::Codex, Some(&plan.argv)),
            Some(known(&codex))
        );
        assert_eq!(
            persisted_session_from_launch_args(Agent::Codex, &plan.argv[1..])
                .unwrap()
                .session_ref
                .value,
            "codex-session"
        );
    }

    #[test]
    fn codex_launch_with_options_records_its_session() {
        for args in [
            &["--no-daemon", "resume", "codex-session"][..],
            &["resume", "--no-daemon", "codex-session"][..],
            &[
                "-m",
                "x",
                "resume",
                "codex-session",
                "-c",
                "k=v",
                "a prompt",
            ][..],
            &[
                "resume",
                "codex-session",
                "--dangerously-bypass-approvals-and-sandbox",
            ][..],
        ] {
            assert_eq!(
                persisted_session_from_launch_args(Agent::Codex, &argv(args))
                    .map(|session| session.session_ref.value),
                Some("codex-session".to_string()),
                "{args:?}"
            );
        }
        for args in [
            &["resume"][..],
            &["resume", "--no-daemon"][..],
            &["resume", "--bogus", "codex-session"][..],
            &["resume", "--remote=unix://", "codex-session"][..],
            &["exec", "resume", "codex-session"][..],
            &["a prompt"][..],
            &["--", "resume", "codex-session"][..],
        ] {
            assert!(
                persisted_session_from_launch_args(Agent::Codex, &argv(args)).is_none(),
                "{args:?}"
            );
        }
        assert!(persisted_session_from_launch_args(
            Agent::Claude,
            &argv(&["resume", "codex-session"])
        )
        .is_none());
    }
}
