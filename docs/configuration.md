# Configuration and model catalogs

One user-owned TOML file configures both entry points. Plugin actions honor
`HERDR_PLUGIN_CONFIG_DIR` and `HERDR_PLUGIN_STATE_DIR`. The CLI uses:

| Data | Default directory |
| --- | --- |
| `config.toml` | `$XDG_CONFIG_HOME/herdr/plugins/config/composer`, normally `~/.config/herdr/plugins/config/composer` |
| Drafts, originals, sessions | `$XDG_STATE_HOME/herdr/plugins/composer`, normally `~/.local/state/herdr/plugins/composer` |

`COMPOSER_CONFIG_DIR` and `COMPOSER_STATE_DIR` can override CLI directories.
Herdr-supplied plugin directories take precedence. Executable configuration is
never loaded from a task repository. Worktrunk owns project-hook approvals.

```toml
repositories = ["/path/to/repository"]

[defaults]
launch_mode = "worktree"
workspace = "herdr"
agent = "codex"
focus = true

[agents.codex]
catalog = "discovery"
allow_custom_model = true
```

`defaults.launch_mode` selects `"worktree"` or `"tab"`. New worktrees remain the
built-in default. The editor's **Launch in** picker and CLI `--launch-mode` override
the default for one task; a saved explicit choice survives reopening the draft.
Automatic follows the current config. The worktree provider applies only to
worktree mode. Switching to a tab keeps that provider preference for later use.

Tab mode uses the resolved repository checkout as it is. An invocation from a
linked checkout keeps that checkout unless you select another repository path.
Branch and base overrides require worktree mode; clear saved overrides or switch
back before launching. Tab cleanup closes its recorded tab without removing Git
worktrees or changing branches.

Codex and Claude use local model discovery by default. Other agents default to the curated
catalog. An explicit `catalog` setting overrides this choice:

| `catalog` | Source |
| --- | --- |
| `curated` | Shipped [`catalogs/curated.json`](../catalogs/curated.json). Currently includes Claude's native model aliases without assumed effort/speed capabilities. |
| `discovery` | Codex queries `codex debug models` with a five-second timeout and 1 MiB output limit. Falls back to `models_cache.json` under CODEX_HOME (normally `~/.codex`) with a diagnostic if the query fails. Claude queries its SDK initialization control response for selectable models and effort levels, with the same bounds. It sends no task or inference request and disables hooks, plugins, tools, MCP, and session persistence. Claude discovery failures report a diagnostic and retain configured entries without substituting a curated list. Other kinds report that built-in discovery is unavailable. |
| `command` | `command = ["/absolute/catalog-program", "arg"]`, with versioned JSON stdin/stdout. Five-second timeout and 1 MiB output limits. |

The catalog command receives `{"version":1,"agent":"id","kind":"codex"}` and
returns `{"version":1,"models":[...]}`. Diagnostics belong on stderr. Failed
discovery leaves configured entries and Automatic available, with a visible
diagnostic. Refresh never silently substitutes a selected model.

User model overrides merge by exact ID. For example, replace this placeholder
with an ID and capabilities accepted by your agent:

```toml
[agents.codex]
catalog = "curated"
default_model = "model-id-from-your-agent"

[[agents.codex.models]]
id = "model-id-from-your-agent"
label = "Daily work"
aliases = ["daily"]
order = 10
enabled = true
visible = true
efforts = ["low", "medium", "high"]
speeds = ["normal", "fast"]
default_effort = "high"
```

Agent entries also accept `kind`, `label`, `order`, `enabled`, and `visible`.
Disabled choices fail through every selection path. Hidden choices stay usable
when explicitly named. Duplicate IDs or conflicting aliases within an agent
are errors. Without an explicit agent, a model or alias must match one agent.
Without any agent selection/default, the sole installed enabled agent is used.

Automatic applies configured defaults or omits the native flag. Normal speed
is an explicit setting. Codex maps it to `service_tier="default"`, and Fast to
`service_tier="fast"`; effort uses `model_reasoning_effort`. Claude supports
`--model` and `--effort`. Other Herdr kinds work with Automatic settings.
Unknown custom models require an explicit agent and `allow_custom_model=true`;
they receive no invented effort or speed support.

`sow` inherits `defaults.workspace` like the editor. It supplies
`--default-agent codex` to prefer Codex even when Composer's editor has another
configured default. Explicit agent flags, inline `@agent` directives,
and model choices retain their usual precedence. This override is scoped to the
launch and does not change the editor's saved settings.

## Shared Codex

Shared mode is optional and has been tested with unmodified Codex **0.159.2**.
It connects new Composer task agents to an existing local app-server. Model,
reasoning effort, speed, and checkout are selected separately for each task.
Without this setting, Composer continues to use `--strict-config`. Branch
naming remains a separate ephemeral call.

Stock Codex shares its backend environment with hooks and legacy `notify`.
Composer registers each session's Herdr routing fields before opening an
explicit `resume <session-id>` client. Its adapter supplies the correct pane
environment to the original hook or notification command, keeping the original
input, output, and exit status. Only seven Herdr routing fields are stored, in
private files; credentials are not copied into them. Shell tools receive the
same fields through the session's shell environment policy.

After building, configure the adapter against your already running Codex server:

```sh
python3 scripts/setup-shared-codex.py \
  --socket unix:///absolute/path/to/app-server.sock \
  --contexts /absolute/path/to/private/composer-contexts
```

Setup requires Python 3.11+, an installed Composer binary, enabled Codex hooks
(`features.hooks = true`), and trusted user
command hooks in `$CODEX_HOME/hooks.json`. It wraps those commands and the
existing `notify` argv, preserves hook options, and adds a context-only
`SubagentStart` hook. It updates trust only for already-trusted commands and
this known adapter. Original configuration files are backed up under the
context directory. Setup verifies the running server sees the updates without
restarting it. Enabled plugin or project command hooks need separate routing
support; shared launches currently refuse such configurations.

If another file generates `config.toml`, supply its notify source with
`--notify-policy /path/to/config.shared.toml` and its existing render command
with `--apply-policy /path/to/codex-config-apply`. This keeps later renders from
removing the notify adapter. Other configuration managers should preserve
the adapter's `notify` argv and updated machine-local hook trust records.

Add the verified settings printed by setup to Composer's `config.toml`:

```toml
[codex.shared]
socket = "unix:///absolute/path/to/app-server.sock"
contexts_dir = "/absolute/path/to/private/composer-contexts"
```

When `config.toml` is synced between machines, put just `socket` and
`contexts_dir` (without the table header) in the machine-local Composer state
file `codex-shared.toml` instead. Its default location is
`~/.local/state/herdr/plugins/composer/codex-shared.toml`; it follows
`HERDR_PLUGIN_STATE_DIR`, `COMPOSER_STATE_DIR`, or `XDG_STATE_HOME` like other
Composer state. Configuring both locations is an error. A machine without
either opt-in continues to use embedded mode.

A stable executable launcher can be supplied as `--binary`; it must forward
Composer's `__codex-hook` and `__codex-notify` arguments. To migrate an existing
trusted adapter, also pass `--replace-binary /exact/old/binary`. Setup only
unwraps adapters at that exact path, with the same context directory, and
still requires their current commands to be trusted.

The endpoint and context directory are frozen in the saved launch request.
Composer checks effective hooks and notify before creating a session. A
changed adapter, unavailable server, or preparation failure stops the launch
before task delivery. Any created Codex session ID remains in the session
record as `codex_thread`; failures never cause automatic prompt replay.

Remove `[codex.shared]` or the local `codex-shared.toml` to use embedded mode for future submissions. Queued
requests retain their chosen backend. Wrappers pass unrelated embedded
sessions through with their inherited environment. Keep the adapters and
private routing records while shared sessions may still run or be resumed;
they also route native children and a root session resumed in another Herdr
pane. Opening the same session in multiple Herdr clients refuses ambiguous
routing. Restore the original configuration backup when no shared sessions
need the adapter. No Codex executable or source changes are required.

## Branch naming

Model-generated branch names are optional and use a separate Codex call. Configure
the naming model independently of the agent that performs the task:

```toml
[branch_naming]
enabled = true
model = "model-id-from-your-codex"
effort = "medium"
speed = "fast"
prefix = ""
```

Naming is disabled by default. `model` is required when enabled; omitted effort
and speed use Codex defaults. Speed accepts `fast` or `normal`. `prefix` is added
to the generated name, for example `team/`. These settings never change the task
agent's model, effort, or speed.

An explicit branch name or a valid prose-resolver branch suggestion takes
precedence. Tab launches and tasks without text skip naming. For other worktree
launches, Composer sends the task text to `codex exec` on stdin after validating
the launch settings. This requires an authenticated Codex CLI with support for
ephemeral execution and `--ignore-user-config`. The naming call uses read-only
mode with shell tools, agent delegation, and web search disabled. It runs outside
the repository and does not load project instructions or the user's Codex config.

Naming has a 20-second timeout. Failure, invalid output, or an existing branch
name falls back to a unique `task-<id>` name. The reason appears in CLI/runner
output and the saved session record. To choose a name yourself, use the editor's
Branch name field or CLI `--branch`.

## Workspace titles

Workspace naming is independent of branch naming and off by default:

```toml
[workspace_naming]
enabled = true
model = "model-id-from-your-codex"
effort = "medium"
speed = "fast"
```

Composer's asynchronous Herdr hook observes agent status changes in linked
worktree workspaces. It works with native Herdr creation and other worktree
providers; the task's agent does not have to be Codex. The naming model still
requires an authenticated Codex CLI, with the same isolated, ephemeral execution
and 20-second timeout as branch naming.

Only workspaces still displaying their checkout folder's name are eligible.
The hook sends the last 100 lines of that agent pane (at most 6,000 characters)
to the configured model. Once a concrete task is visible, it applies a short
lowercase kebab-case title such as `shared-alarm-wakeup` (at most 36 characters).
Startup screens can return no title; later status changes can try again, with
at most three calls per workspace and no concurrent calls for that workspace.
Successful naming stops further calls. Existing workspaces become eligible on
their next agent status change.

Before applying a title, Composer rechecks the workspace label and pane binding.
A manual label or a moved/replaced agent cancels the update. Herdr 0.9.0 does not
expose whether a label was manually set, so an explicit label identical to the
checkout folder name is indistinguishable from the default. Branch names, paths,
focus, and task delivery are unaffected. Naming attempts retain only counters
and a content digest under the plugin state directory, never terminal excerpts.
Failures appear in `herdr plugin log list --plugin composer`.

## Prose suggestions

Prose suggestions are off by default. Set top-level
`prose_resolver = ["/absolute/program", "arg"]` to enable one invocation per
submission, bounded to five seconds. Input contains `version`, literal `task`,
the normalized `catalog`, and `repositories`. Return
`{"version":1,"suggestions":{"agent":"codex","branch":"new-name"}}`.
Suggestions cannot enable disabled agents or replace explicit choices.
