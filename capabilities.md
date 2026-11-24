# herdr capabilities

herdr is a terminal workspace manager and runtime for AI coding agents. It ships
as one Rust binary that runs a background server, which owns the terminals and
their processes, plus one or more terminal UI clients that attach to it. Agents
such as Claude Code, Codex or OpenCode run unchanged in real terminal panes;
herdr tracks whether each one is working, blocked or idle, and exposes
everything through a CLI and a JSON socket API so scripts and agents can drive
it.

> This file describes the source tree on `main`, which can include unreleased
> work. Claims are based on `src/` and the unreleased docs drafts in
> [`docs/next/website/src/content/docs/`](docs/next/website/src/content/docs/),
> not on the published stable docs.

## 1. Workspaces, tabs and panes

- Organize work as **workspaces** (one per repo, task or investigation), each
  holding **tabs** (layouts), each holding **panes** (real terminals).
- Split panes right or down, resize, zoom, swap, move a pane to another tab or
  workspace, rename and close panes, tabs and workspaces.
- Agent state rolls up from panes to tabs and workspaces in the sidebar.
- Mouse-first UI: click to focus, drag split borders, drag-select to copy,
  right-click menus. Mouse capture can be turned off.
- tmux-style prefix keys (default `ctrl+b`): terminal mode, prefix mode and
  navigate mode, a Goto picker (`prefix+g`), keybind help (`prefix+?`), and
  optional prefix-free chords.
- CLI: `herdr workspace list|create|get|focus|rename|report-metadata|close`,
  `herdr tab list|create|get|focus|rename|close`,
  `herdr pane list|get|split|resize|zoom|swap|move|rename|close|...`.
- Source: `src/workspace.rs`, `src/workspace/`, `src/layout.rs`, `src/app/`,
  `src/ui.rs`, `src/ui/`. Docs:
  [concepts](docs/next/website/src/content/docs/concepts.mdx),
  [keyboard](docs/next/website/src/content/docs/keyboard.mdx).

## 2. Background server, sessions, detach and reattach

- The server owns panes and processes; the TUI is a client. Detach with
  `ctrl+b q` and everything keeps running; run `herdr` to reattach.
- Several clients can attach at once, and each can view its own workspace and
  tab.
- Named sessions are separate runtime namespaces with their own panes, sockets
  and saved state: `herdr session list|attach|stop|delete`, or
  `herdr --session <name>`.
- Server control: `herdr server` (headless), `herdr server stop`,
  `herdr server reload-config`; status with `herdr status [server|client]`.
- Source: `src/server/`, `src/client/`, `src/session.rs`. Docs:
  [concepts](docs/next/website/src/content/docs/concepts.mdx) ("Client and
  server"),
  [persistence and remote](docs/next/website/src/content/docs/persistence-remote.mdx).

## 3. Persistence and restore

- **Live persistence:** detaching never stops processes.
- **Snapshot restore:** after a server or machine restart, herdr restores
  workspaces, tabs, panes, working directories, layout and focus. It also keeps
  up to 48 recovery layout snapshots in `session-snapshots/`.
- **Pane screen history replay** (opt-in, `[experimental] pane_history = true`)
  brings back recent terminal contents after a restart.
- **Native agent session resume** (on by default): supported agents that
  reported a session through an official integration, or that report their own
  resume command, are restarted into the same conversation.
- **Live handoff** (experimental, opt-in, Unix only): `herdr update --handoff`
  or `herdr --remote <host> --handoff` moves running panes to a replacement
  server.
- Limit: the original processes do **not** survive a server or machine
  restart. Without a stronger restore path a pane comes back as a fresh shell in
  its saved directory.
- Source: `src/persist.rs`, `src/persist/`, `src/agent_resume.rs`,
  `src/handoff_runtime.rs`. Docs:
  [session state](docs/next/website/src/content/docs/session-state.mdx).

## 4. Agent status detection

- Each agent pane is classified as `working`, `blocked`, `done`, `idle` or
  `unknown`, from the agent's foreground process plus the live bottom of the
  screen (not the scrolled viewport), using per-agent TOML detection manifests.
- Bundled manifests can be updated from herdr.dev without a restart, and local
  overrides in `~/.config/herdr/agent-detection/<agent>.toml` always win.
- Commands: `herdr server agent-manifests`,
  `herdr server update-agent-manifests`, `herdr server reload-agent-manifests`,
  and `herdr agent explain <target>` (or `--file PATH --agent LABEL`) to see why
  a pane got its state.
- Notifications (in-app toast, outer terminal, or OS) and sounds when a
  background agent finishes or needs input.
- Agents with a bundled detection manifest in `src/detect/manifests/`:

  | Manifest | Agent |
  | --- | --- |
  | `amp` | Amp |
  | `antigravity` | Antigravity CLI |
  | `claude` | Claude Code |
  | `cline` | Cline |
  | `codex` | Codex |
  | `cursor` | Cursor Agent CLI |
  | `devin` | Devin CLI |
  | `droid` | Droid |
  | `gemini` | Gemini CLI |
  | `github-copilot` | GitHub Copilot CLI |
  | `grok` | Grok CLI |
  | `hermes` | Hermes Agent |
  | `kilo` | Kilo Code CLI |
  | `kimi` | Kimi Code CLI |
  | `kiro` | Kiro CLI |
  | `letta` | Letta Code |
  | `maki` | Maki |
  | `muse` | Muse |
  | `opencode` | OpenCode |
  | `pi` | Pi |
  | `qodercli` | Qoder CLI |
  | `qwen` | Qwen Code |

- Some agents report their own state to herdr instead (for example Crush,
  Command Code, Muse and Prime Agent); other programs run as plain terminals.
- Source: `src/detect/`. Docs:
  [agents](docs/next/website/src/content/docs/agents.mdx).

## 5. Agent integrations

- `herdr integration install|uninstall <agent>` installs or removes hooks that
  tell herdr which session an agent is in, so it can resume that session after a
  restart. `herdr integration status [--outdated-only]` checks versions.
  Integrations can also be installed from the integrations tab in settings.
- Integration targets (assets under `src/integration/assets/`): `pi`, `omp`,
  `claude`, `codex`, `copilot`, `devin`, `droid`, `kimi`, `opencode`, `kilo`,
  `hermes`, `qodercli`, `qwen`, `letta`, `cursor`, `mastracode`,
  `antigravity-cli`, `grok`.
- This list differs from the detection list above: `omp` and `mastracode` have
  integrations but no screen manifest (their state requires the integration),
  while `amp`, `cline`, `gemini`, `kiro`, `maki` and `muse` have manifests but no
  integration.
- The Pi, OMP, Kimi Code CLI, OpenCode, Kilo Code CLI and MastraCode
  integrations also report state, which then replaces screen reading.
- Custom hooks and tools can report state and display metadata themselves:
  `herdr pane report-agent`, `report-agent-session`, `release-agent`,
  `report-metadata` (custom status labels and sidebar tokens).
- Source: `src/integration/`. Docs:
  [integrations](docs/next/website/src/content/docs/integrations.mdx),
  [add herdr support](docs/next/website/src/content/docs/add-herdr-support.mdx).

## 6. Agent automation

- Scripts and agents can build layouts, start agents, prompt them, wait for
  them and read their output, so one agent can hand work to others.
- Agents: `herdr agent list|get|read|send-keys|prompt|rename|focus|wait|attach|start|explain`.
  `agent start <name> --kind KIND --pane ID` launches a supported agent in an
  idle shell pane and returns once it is ready; `agent prompt --wait` submits a
  prompt and waits for a settled state; `agent wait --until STATUS` blocks until
  an agent is idle, done or blocked.
- Panes: `herdr pane read` (visible, recent, recent-unwrapped or detection
  source, text or ANSI), `send-text`, `send-keys`, `run`, `wait-output`
  (literal or regex), `layout`, `neighbor`, `edges`, `process-info`, `input`.
- In-app or toast notifications from scripts: `herdr notification show`.
- An agent skill teaches coding agents to use herdr from inside a pane:
  [`skills/herdr/SKILL.md`](skills/herdr/SKILL.md).
- Source: `src/cli/agent.rs`, `src/cli/pane.rs`, `src/agent_launcher.rs`,
  `src/api/wait.rs`. Docs:
  [agent automation](docs/next/website/src/content/docs/agent-automation.mdx),
  [CLI reference](docs/next/website/src/content/docs/cli-reference.mdx).

## 7. Socket / JSON API and events

- A local socket API (Unix socket, Windows named pipe) with the same control
  surface as the CLI: workspaces, tabs, panes, agents, state reporting,
  integrations, server control and plugins.
- Long-lived event subscriptions (`events.subscribe`), for example on
  `pane.agent_status_changed`.
- A bundled JSON Schema: `herdr api schema [--json|--output PATH]`; a copy is
  at [`docs/next/api/herdr-api.schema.json`](docs/next/api/herdr-api.schema.json).
- Agent view queries (`agent.view.set`) to customize the built-in Agents view.
- Pane graphics go through the standard Kitty graphics protocol in pane output;
  the old socket pane-overlay image API has been removed.
- Protocol stability: a stable client endpoint generation lets clients and
  servers of different builds work together; unsupported methods fail only the
  action that needs them.
- Source: `src/api/`, `src/protocol/`, `src/cli/api.rs`. Docs:
  [socket API](docs/next/website/src/content/docs/socket-api.mdx).

## 8. Remote machines over SSH

- Save SSH machines with `herdr machine list|add|rename|disable|enable|remove|status|reconnect`
  and see Local and saved machines in one window with a combined agent list.
  Each machine keeps its own server, and losing one connection does not
  disconnect the others.
- `herdr --remote <host>` attaches to a remote server over SSH while drawing the
  UI locally (with local keybindings by default), and can install or update
  herdr on the remote host after confirmation.
- `herdr --machine <label-or-id> <command>` routes API commands such as
  `agent`, `pane` or `worktree` to a saved machine.
- Local image clipboard paste is bridged into remote sessions.
- Source: `src/remote.rs`, `src/remote/`, `src/cli/machine.rs`,
  `src/platform/remote_bridge.rs`. Docs:
  [connecting machines](docs/next/website/src/content/docs/connecting-machines.mdx),
  [persistence and remote](docs/next/website/src/content/docs/persistence-remote.mdx).

## 9. Git worktrees

- Create, open and remove Git worktree checkouts as herdr workspaces, grouped
  under their parent repo workspace, from the sidebar or the CLI:
  `herdr worktree list|create|open|remove`.
- `worktree remove` runs `git worktree remove` and never deletes the branch.
- Checkouts go under `[worktrees] directory` by default.
- Source: `src/worktree.rs`, `src/app/worktrees.rs`, `src/cli/worktree.rs`.
  Docs: [configuration](docs/next/website/src/content/docs/configuration.mdx)
  ("Worktrees"), [CLI reference](docs/next/website/src/content/docs/cli-reference.mdx).

## 10. Plugins

- A plugin is a directory with a `herdr-plugin.toml` manifest and commands in
  any language; the whole herdr CLI is the plugin API.
- Manifests declare actions, event hooks, startup hooks, terminal panes
  (overlay, popup, split, tab or zoomed), link handlers for modified URL clicks,
  build commands, and per-plugin config and state directories. Plugin actions
  can be bound to keys.
- CLI: `herdr plugin install|list|uninstall|enable|disable|link|unlink|config-dir`,
  `herdr plugin action list|invoke`, `herdr plugin log list`,
  `herdr plugin pane open|focus|close`.
- Installs come from GitHub shorthand and show a trust preview first. There is
  a plugin marketplace on herdr.dev.
- Source: `src/cli/plugin.rs`, `src/app/api/plugins/`, `src/plugin_command.rs`,
  `src/plugin_paths.rs`, `src/persist/plugin_registry.rs`. Docs:
  [plugins](docs/next/website/src/content/docs/plugins.mdx),
  [marketplace](docs/next/website/src/content/docs/marketplace.mdx).

## 11. Terminal emulation

- Terminal emulation is built on a vendored libghostty-vt
  (`crates/ghostty-vt`, `vendor/libghostty-vt`), with local patches tracked in
  [`vendor/libghostty-vt.patches.md`](vendor/libghostty-vt.patches.md).
- Kitty graphics rendering in compatible outer terminals, on by default
  (`[terminal] kitty_graphics`).
- Copy mode (`prefix+[`) with vim-style movement and search, mouse selection,
  and clipboard copy; clipboard data set by the server is forwarded to the
  client's outer terminal.
- Notifications and sounds, outer window title control (`ui.window_title`),
  and themes, including one that follows the host terminal's palette and
  light/dark auto-switching.
- Direct terminal attach without the full UI: `herdr terminal attach <id>`,
  `herdr agent attach <target>`, and JSON-stream
  `herdr terminal session control|observe`; `herdr terminal title set|clear`.
- Source: `src/terminal/`, `src/pane.rs`, `src/pane/`, `src/pty/`,
  `src/kitty_graphics.rs`, `src/copy_mode.rs`, `src/selection.rs`,
  `src/terminal_notify.rs`, `src/sound.rs`, `src/terminal_theme.rs`,
  `src/client/clipboard_forwarding.rs`, `src/server/terminal_attach.rs`.

## 12. Configuration

- Optional TOML config at `~/.config/herdr/config.toml` (Linux, macOS) or
  `%APPDATA%\herdr\config.toml` (Windows). Invalid values fall back to safe
  defaults with a warning.
- `herdr --default-config` prints the full default config,
  `herdr config check` reports config problems, `herdr config reset-keys` backs
  up the config and restores default keybindings, and
  `herdr server reload-config` applies changes without restarting panes.
- Configurable keybindings (including the prefix), custom command bindings,
  themes, sidebar row layouts, notifications, sounds, worktree directory,
  restore behavior and more.
- Source: `src/config.rs`, `src/config/`. Docs:
  [configuration](docs/next/website/src/content/docs/configuration.mdx),
  [config reference](docs/next/website/src/content/docs/config-reference.mdx).

## 13. Updates and release channels

- `herdr update` downloads and installs from the configured channel (Homebrew,
  mise and Nix installs update through their package manager instead).
- `herdr channel show` and `herdr channel set stable|preview` switch between
  stable releases and preview builds from `master`.
- Remote detection manifest updates are separate and can be turned off with
  `[update] manifest_check = false`.
- Source: `src/update.rs`, `src/cli.rs`. Docs:
  [install](docs/next/website/src/content/docs/install.mdx).

## 14. Platforms and install

- Release binaries: Linux x86_64 and aarch64, macOS x86_64 and aarch64, and
  Windows x86_64 (a zip with `herdr.exe` and its ConPTY runtime).
- Windows runs natively through ConPTY. The current draft calls it generally
  available, with listed gaps: plugins are in preview, and direct terminal
  attach and live handoff are not supported.
- Install with `curl -fsSL https://herdr.dev/install.sh | sh`,
  `brew install herdr`, `mise use -g herdr`, the PowerShell installer on
  Windows, Nix, or a manual binary download.
- Optional `cuda` Cargo feature for GPU row damage in server frame encoding
  (loads the NVIDIA driver at run time; no CUDA toolkit needed to build).
- Source: `src/platform/` (`linux`, `macos`, `windows`, shared Unix code),
  [`Cargo.toml`](Cargo.toml), `crates/cell-damage`. Docs:
  [install](docs/next/website/src/content/docs/install.mdx),
  [Windows](docs/next/website/src/content/docs/windows-beta.mdx).

## 15. Shell completions

- `herdr completion <shell>` (alias `completions`) prints completion scripts for
  zsh, bash, fish, PowerShell and elvish.
- Source: `src/cli/completion.rs`.

## 16. For developers

- Build: `cargo build --release`.
- Test: `just test` (cargo nextest and maintenance script tests); full check:
  `just check` (formatting, tests and maintenance checks).
- Benchmarks: `just bench-render-scale`, `bench-terminal-targets`,
  `bench-bsp-layout`, `bench-retained-graphics`, `bench-api-fairness`,
  `bench-release-smoke`, `bench-responsiveness` (see the
  [`justfile`](justfile)).
- Before changing anything, read [`AGENTS.md`](AGENTS.md); before opening issues
  or pull requests, read [`CONTRIBUTING.md`](CONTRIBUTING.md).
