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

Each agent selects exactly one source:

| `catalog` | Source |
| --- | --- |
| `curated` | Shipped [`catalogs/curated.json`](../catalogs/curated.json). Currently includes Claude's native model aliases without assumed effort/speed capabilities. |
| `discovery` | Codex's local `models_cache.json` under CODEX_HOME. No Composer cache or network fallback. Other kinds report that built-in discovery is unavailable. |
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

Prose suggestions are off by default. Set top-level
`prose_resolver = ["/absolute/program", "arg"]` to enable one invocation per
submission, bounded to five seconds. Input contains `version`, literal `task`,
the normalized `catalog`, and `repositories`. Return
`{"version":1,"suggestions":{"agent":"codex","branch":"new-name"}}`.
Suggestions cannot enable disabled agents or replace explicit choices.
