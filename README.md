# Herdr Composer

Launch a coding agent in a new worktree or a tab in your existing checkout from a task editor in [Herdr](https://herdr.dev).
Choose a repository, attach screenshots, and set the agent, model, and effort before launching.

Drafts survive closing the editor. Launching clears the task and images from the
editor once setup is handed off, so you can compose another task immediately.
Setup runs in the background. When you're done,
Composer removes the session through the same tool that created it.

## Install

Requires Herdr **0.8.2+**, Git, Rust/Cargo, and an installed coding agent.
The plugin builds its Rust executable during installation.

```sh
herdr plugin install danieljvdm/herdr-composer
herdr plugin action invoke composer.compose
```

Choose an agent in the editor, or set a default in
`~/.config/herdr/plugins/config/composer/config.toml`:

```toml
[defaults]
agent = "codex"
launch_mode = "worktree" # or "tab" to reuse the selected checkout
```

## Use the editor

Write the task, check the settings, then press **Ctrl+S** to launch.
The **Launch in** selector overrides your default for this task. Choose **New worktree**
or **Tab in selected checkout**; Automatic follows the configured default.
Repository, agent, and model are available in the same pane.
Effort and speed appear when the selected model supports them.

| Key | Action |
| --- | --- |
| Tab / Shift+Tab | Move between fields |
| Enter | Open a picker or add a line to the task |
| Ctrl+V | Attach an image from the local clipboard |
| Ctrl+R | Refresh model choices |
| Esc | Save the draft and close |

You can also paste an image's file path. Composer keeps its own copy, so deleting
the source image won't break the task. Image-only tasks work too.

To bind a key, add this to your Herdr config and reload it:

```toml
[[keys.command]]
key = "prefix+shift+c"
type = "plugin_action"
command = "composer.compose"
description = "Compose a task"
```

Pick an unused key if `prefix+shift+c` already has a job in your setup.

## Use the CLI

Install the standalone command if you want to launch from scripts or a shell:

```sh
cargo install --git https://github.com/danieljvdm/herdr-composer --locked
```

Run it inside a Herdr pane:

```sh
herdr-composer launch --agent codex 'Fix the login redirect'
herdr-composer launch --launch-mode tab 'Review my current changes'
herdr-composer launch --repo /path/to/repo --attach screenshot.png - < task.txt
herdr-composer remove --current
```

Claude and Codex workspace trust dialogs keep your task pending. Approve trust
in the task pane printed by Composer; it continues once the agent is ready.
If startup times out, recover the prompt or continue the unsent session:

```sh
herdr-composer task --session ID
herdr-composer resume --session ID
```

The CLI and editor use the same settings. In worktree mode, `--branch` must name a new branch;
`--base current` starts from the invoking checkout. See `herdr-composer --help`
for all options.

The optional `bin/sow` wrapper launches Worktrunk worktrees with the same model
settings. Use `sow catalog --json` (or `sow catalog`) to inspect available models
and efforts without creating a task. `sow --help` lists the wrapper's options.
Put launch options before task text; use stdin or `--` when the task contains
literal option arguments, for example `sow -- 'catalog --json'`.
Sow defaults to Codex; explicit agents and model choices take precedence.
Claude and Codex model choices are discovered from their installed CLIs.

By default, Composer launches Codex with `--strict-config`, keeping its backend
in the pane's process and preserving the pane's environment. You can opt into
[shared Codex](docs/configuration.md#shared-codex) while keeping per-task model,
reasoning, and speed choices. This uses stock Codex and a Composer hook adapter.

To generate branch names with a separate model, enable
[`branch_naming`](docs/configuration.md#branch-naming). Its model, effort, speed,
and prefix are configurable. Explicit branch names always take precedence.
Naming runs in the background after handoff, so the editor closes without
waiting for the model.

## Native worktrees or Worktrunk

New-worktree mode uses native Herdr worktrees by default. To use Worktrunk's checkout
layout and hooks, install `wt` **0.74.0+** and choose Worktrunk in the editor or
pass `--provider worktrunk`. You can also set `defaults.workspace = "worktrunk"`.

Cleanup checks the recorded checkout and workspace before removing anything.
It keeps provider safeguards, including dirty-file checks and hook approvals.
Failed launches retain their submitted task, images, and any prepared workspace
in the session record for inspection.
Tab mode shares the selected checkout, including uncommitted work. Its cleanup
closes only the recorded tab and keeps the checkout, files, and other tabs.

## More

- [Configuration and model catalogs](docs/configuration.md)
- [Workspace providers and custom commands](docs/providers.md)
- [Recovery and importing old drafts](docs/recovery-and-import.md)
- [Building and testing](CONTRIBUTING.md)

[MIT license](LICENSE.md). Derived from herdr-worktrunk; original attribution retained.
