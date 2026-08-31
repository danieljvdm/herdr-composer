# Herdr Composer plan

Build a task composer for Herdr. Describe the work, attach context, choose a
repository, workspace provider, and agent, then launch. The same resolver and
launcher serve the terminal editor and CLI.

Use `Herdr Composer` as the product name, `composer` as the plugin ID, and
`herdr-composer` as the executable and repository slug. Start the independent
package at `0.1.0`; installation examples must point to the published fork.
Native Herdr is the default workspace provider. Worktrunk is an explicit choice
for its checkout conventions and hooks.

This is the implementation contract for the replacement, implemented on
2026-08-31. The root Rust crate owns both entry points; the shell runtime,
branch browser, PR subsystem, old wrappers, and legacy tab mode are removed.
The editor/media modules retain their existing editing and rendering behavior.
The manifest action invokes the binary directly and adds its package directory
to the editor pane's PATH, as required by Herdr 0.8.2's PTY executable lookup.
The pane's working directory is the plugin root; a pinned environment value
preserves the invoking checkout independently. The live compose action passed.

Verification includes resolver/catalog/storage tests, fake-tool acceptance
tests with real disposable Git repositories, a focused editor PTY flow, import
tests, and live native Herdr and Worktrunk launches in the isolated named
`composer-integration-20260831` session. Herdr 0.8.2 and Worktrunk 0.74.0 both
passed real Codex startup, structured prompt acknowledgement, dirty-checkout
refusal, safe removal, and unmerged-branch retention. Native removal retains
branches; Worktrunk removes merged branches and retains unmerged branches.
Worktrunk's configured blocking pre-switch hook also demonstrated failure
before startup and successful completion with a local disposable remote.

The public package is published independently from herdr-worktrunk. Installation
uses the owner-qualified repository in README. The source license and Git
history are retained. Import remains an explicit user action.

The audit covers all 29 tracked files at `607a38b`, plus this plan, on
2026-08-31. Generated binaries, build caches, and Git history are outside the
source redesign. Preserve existing user state and the license notice.

## Requirements and scope decisions

| Requirement | Source | Consequence |
| --- | --- | --- |
| Task editing, repository selection, attachments, and recoverable drafts | Original plan, points 1 and 4 | Retain these behaviors through the rewrite. |
| One launch path for editor and CLI | Original plan, point 1 | Parse and resolve once, then pass a typed request to launch. |
| Native Herdr, Worktrunk, and a custom executable option | Original plan, point 2 | Implement the two concrete providers first; derive the executable contract from them. |
| Configurable agents, model catalogs, and supported effort/speed controls | Original plan, point 3 | Remove model policy from UI code and shell routing. |
| Focus, launch feedback, and cleanup through the provider that created the session | Original plan, point 4 | Persist launch ownership and keep partial failures inspectable. |
| A product with its own identity and architecture | Latest user instruction | Every source file needs a composer-specific reason to remain. |
| Literal task text, durable originals, pinned removal targets, and provider safeguards | Existing behavior and data-safety boundaries | Carry these into acceptance tests before deleting their implementations. |

Two scope decisions supersede the original migration and optional-helper
language:

- Install alongside the old plugin. Do not automatically rewrite its config,
  keybindings, wrappers, or state. Offer an explicit copy/import of useful config,
  drafts, and their attachment originals, plus a documented keybinding and
  command mapping. Preserve source data and newer destination files. Imported
  users retain Worktrunk; fresh installations default to Herdr.
- Remove branch browsing, PR reporting/opening, and legacy tab presentation from
  this product. Users can keep the old plugin or use the underlying tools for
  those workflows. Do not keep dormant implementations behind feature flags.

The configurable launch-mode follow-up adds existing-checkout tabs. The built-in
default remains `worktree`; `defaults.launch_mode = "tab"`, the editor's Launch in
picker, or CLI `--launch-mode tab` explicitly opts into a shared checkout. The
selected checkout is frozen before handoff. Tab sessions own their tab and never
claim checkout ownership. Their cleanup validates tab/pane bindings and closes
only that tab. Worktree provider safeguards remain unchanged for worktree mode.

## One executable and a shared core

Delete the shell runtime once its necessary behavior lives in the shared Rust
implementation. Today `app.rs`, `composer.sh`, and `dispatch.sh` each interpret
parts of the request, then `composer.sh` generates another shell program for
handoff. Replace that translation chain with one resolved request.

Move the Rust crate to the repository root. The executable owns configuration,
catalog resolution, draft storage, workspace preparation, agent startup, and
session cleanup. The manifest invokes that executable directly. An internal
action entry point pins the invoking context and asks Herdr to open the editor
or removal pane; the pane then runs the normal command. Keep a small build script
for Herdr's locked build and atomic binary replacement.

Public commands:

```text
herdr-composer                         open the editor
herdr-composer launch [options] -      read task text from stdin and launch
herdr-composer catalog --json          inspect the catalog used by both paths
herdr-composer remove --session ID     remove a recorded composer session
herdr-composer remove --current        resolve and pin the caller's session
herdr-composer import-worktrunk PATH   explicitly copy supported old state
```

`launch` also accepts task text as an argument. Keep explicit repository,
provider, branch/base, agent, model, effort, speed, and focus options. `sow` and
`reap` become optional user-installed aliases to these commands. The new
installation does not take those names over.

Separate request/config/catalog resolution, external tool calls, session
storage, and editor/media code with ordinary modules. Create file boundaries
when they acquire code. An enum is enough for built-in workspace providers.
Keep CLI calls as argument arrays. The pane handoff contains a quoted executable
and request-file path; task text stays out of shell commands. Add no daemon,
workflow engine, provider registry, or graphics extension framework.

### Request and resolution

Keep three data structures with separate jobs:

1. `Draft` holds editable task text, attachment references, repository, provider,
   branch/base, agent/model/effort/speed choices, focus, and a revision. Unset
   choices mean Automatic. Key it by invoking checkout or global context;
   changing provider or branch does not hide a draft.
2. `TaskRequest` is the immutable resolved form, with a launch ID, canonical
   repository, chosen provider, new branch, base specification, exact agent/model
   IDs, validated settings, and retained attachment paths. Here an absent setting
   intentionally omits its native flag. The runner never re-resolves it.
3. `SessionRecord` owns that request, provider receipt, resource IDs, last
   completed step, and any error. Cleanup follows this record. Store it privately;
   successful session removal discards the task payload and retains a minimal
   removed receipt to prevent replay.

Version persisted and executable JSON boundaries and reject unsupported versions.
Internal Rust types do not each need their own protocol or schema service.

Resolve fields in this order: explicit editor/CLI selection, explicit inline
directive, configured prose-resolver suggestion, configured default. Support
`@agent`, `>repo`, and `branch:` through one parser for argument and stdin input.
Consume only recognized leading directives and preserve the remaining task
literally. An explicit invalid choice is an error, never a reason to fall back.

Resolve repository tokens to an existing path or a unique exact repository name.
Remove arbitrary substring matching inside task prose. An ambiguous global
invocation asks for a repository in the editor; the CLI reports the candidates.
Keep the invoking checkout separate from the repository's common Git directory.
Base choices are Provider default, Current checkout, and an explicit Git ref.
Current is available only when the invoking checkout belongs to the selected
repository. Pin Current/ref to a commit before handoff. Provider default is a
frozen instruction to omit the override. Record the resulting `prepared_head`;
hooks may have changed HEAD, so do not call that the base commit. Label the
choice Provider default and preserve the provider's branch conventions.

Generate branch names locally with a collision-resistant suffix. Validate with
Git and let the creation operation arbitrate a race. An explicit existing name
fails with an explanation; do not reinterpret it as permission to reuse a task.

Prose resolution is off by default. A configured command receives task text and
the available catalog as JSON on stdin, and returns suggestions. It cannot
override explicit fields, enable a disabled agent, or invent capabilities.
Invoke it once with a bounded timeout. Invalid suggestions leave fields open
for configured defaults and produce a visible diagnostic. Remove the implicit
Claude naming call and its fallback chain.

### Catalog and configuration

Use one user-owned `config.toml`, resolved identically by CLI and plugin actions.
Honor Herdr's supplied config/state directories; CLI directory discovery lives
in the binary. Do not automatically load executable configuration from a task's
repository. Worktrunk continues to own its project-hook approval rules.

Each agent has one selected catalog source: built-in discovery, a shipped
curated list, or a configured executable. Normalize the source once, then apply
user overrides by exact model ID. Commands use argv arrays and return one
versioned JSON model list on stdout, with diagnostics on stderr.

Catalog data includes agent kind, model ID, label, aliases, display order,
visibility, enabled state, defaults, supported effort values, and supported speed
values. `enabled = false` rejects all selection paths. `visible = false` hides a
choice from pickers while allowing explicit selection. Keep these meanings
distinct and remove the separate `disabled_agents` mechanism.

An illustrative override, with a placeholder model ID:

```toml
[defaults]
workspace = "herdr"
agent = "codex"
focus = true

[agents.codex]
catalog = "curated"
allow_custom_model = true
default_model = "model-id-from-your-agent"

[[agents.codex.models]]
id = "model-id-from-your-agent"
label = "Daily work"
aliases = ["daily"]
order = 10
efforts = ["low", "medium", "high"]
speeds = []
```

Replace the placeholder and capabilities with values your agent accepts.
Catalog data and launch adapters must agree on supported settings. Remove
embedded model-family guesses and fallback versions from code.

Resolution rules:

- Resolve aliases within the selected agent. Without an agent, infer one only
  from a unique catalog match. Ambiguous aliases require an agent selection.
  Reject duplicate IDs and conflicting aliases within one agent.
- Automatic permits the configured default, otherwise omits the native flag.
  An explicit Normal speed is distinct from Automatic. Keep native tier and
  effort translations inside the agent adapter. If no default agent exists,
  use the sole available enabled agent or require an explicit selection.
- Changing agent/model revalidates effort and speed. Show an incompatible saved
  choice and ask for correction; do not silently downgrade it.
- Custom model entry requires an explicit agent and `allow_custom_model`.
  Unknown models get no invented capabilities. Explicit effort/speed needs
  configured or discovered support, or validation fails before preparation.
- Herdr-supported agent kinds remain usable with automatic settings. Explicit
  settings require a supported adapter. Remove the special OpenCode preview
  launch path unless a concrete supported launch contract justifies it.
- Discovery runs outside the editor event loop, with bounded runtime and output.
  While loading, show user-configured entries and Automatic. A failed source
  leaves these entries and a visible error. Curated entries appear only when
  that source is selected. Offer Refresh; add no persistent discovery cache or
  silent substitution of a selected model.
- Freeze the resolved catalog selections for a submitted request. Later config
  changes apply to new launches, not an already queued task.

### Workspace providers

Providers prepare and remove workspaces. Their input is the resolved workspace
portion of the request, launch ID, and pinned Herdr source workspace. Task text,
attachments, and model settings stay with the launcher. The result contains the
canonical checkout, branch, Herdr workspace/pane IDs, resource ownership, and
cleanup information.

| Provider | Prepare | Remove |
| --- | --- | --- |
| `herdr` | Create through native Herdr worktree support using an explicit source repository and the requested base choice. Read returned IDs. | Validate the recorded workspace/checkout binding, then use native removal without force. |
| `worktrunk` | Run `wt switch --create ... --no-cd --format=json`, preserve user configuration and hooks, then register the returned checkout with Herdr. | Run Worktrunk removal against the recorded checkout from outside it. Close only its verified Herdr workspace after provider success. |
| Configured executable | Receive a versioned `prepare` request on stdin and return the same prepared-workspace result. | Receive a versioned `remove` request with the recorded provider receipt and return a removal outcome. |

Create/open in the background and apply the user's focus choice deliberately.
The preparation result means the provider's blocking setup is complete. Do not
start an agent merely because a worktree path has appeared while hooks run.
Worktrunk background hooks retain their normal semantics.

No automatic hook approval, `--no-hooks`, force, clobber, or provider fallback.
A provider failure reports its output and any known created resources. A custom
command must return one valid JSON object, use stderr for progress, honor the
launch ID, and report partial preparation through its receipt where possible.
Timeout or malformed output after a mutation leaves the launch needing attention.
It never triggers another prepare call automatically.

Persist Worktrunk's returned checkout and ownership before calling Herdr open,
then add the Herdr IDs. A failed open still leaves a usable cleanup receipt.
Accept valid partial receipts from custom providers too. Without a validated
receipt, uncertain preparation requires inspection rather than automatic cleanup.

Store the provider ID, protocol version, cleanup options, and custom command
argv used at launch. Changing defaults or redefining an entry must not redirect
cleanup. If the recorded provider is unavailable or cannot read its receipt,
stop with a recovery instruction. Do not fall back to `git worktree remove`.

The executable interface is a first-release requirement, but implement it only
after both concrete providers use the same result. Ship one fixture provider
and a short example; no provider discovery, installation, or templating system.

### Launch, recovery, and removal

Validate repository, explicit base, provider availability, model settings,
attachments, and Herdr connectivity before creating a checkout. Opening or
cancelling the editor performs no workspace mutation.

Both entry points assign a launch ID and atomically save the resolved request
before the first mutation. Resolve the selected repository's source workspace
or create it in the background, record its identity, and open a runner tab there.
The runner uses the same binary with only the session ID as input. It reads the
frozen record. Keep exclusive ownership of that record while launching, so
duplicate runner invocations report its state instead of starting work again.

The source workspace survives task cleanup. Only the target workspace belongs
to the task. Closing the editor must not kill preparation. Show Preparing while
setup runs, and leave failures visible in the runner and session record. Clear
only the submitted draft revision after confirmed delivery. Protect revision
comparisons and writes against a concurrent editor; an older save must not
overwrite a newer draft.

Use these checkpoints rather than a generic workflow engine:

| Boundary | What must be durable or observable |
| --- | --- |
| Before preparation | Launch ID, resolved request, provider, and submitted draft revision. |
| Checkout created, then workspace opened | Persist checkout ownership/receipt first, then Herdr IDs, before the next mutation. |
| Agent started | Returned live identity and the exact requested settings. |
| Before sending the prompt | A delivery-attempt marker, so a crash cannot cause an automatic resend. |
| Prompt result | Herdr's command result and any structured lifecycle transition. Claim delivery only when the supported API establishes it; otherwise mark it uncertain. |
| Removal | Provider outcome and any remaining workspace closure, so a partial cleanup is visible. |

Call Herdr start and prompt once, using its timeout and lifecycle checks. Record
a documented rejection before input as Not sent. A stalled prompt, timeout, or
lost response is Unknown. Only documented post-submission acknowledgement marks
delivery Confirmed and permits draft clearing. Blocked dialogs require user
action. Remove blind Enter nudges and transcript substring checks.

Keep failed drafts and prepared resources for inspection. Offer the exact
workspace to open and a concrete next action. Do not automatically roll back a
checkout that hooks or an agent may have modified. Do not build a general resume
engine in this release.

Report settings as requested unless a structured runtime response confirms the
effective values. Delete TUI footer scraping. If an adapter can establish a real
mismatch, report it without claiming verified settings for the other adapters.

Removal applies only to recorded composer sessions. Pin the caller's target
before showing confirmation, then recheck canonical repository/checkout
identity and the live workspace binding immediately before mutation. Reject the
primary checkout, missing ownership, changed bindings, and ambiguous targets.
Changing focus must never change the target. Show live agents and pending work.

The editor confirms removal of the pinned target. CLI callers authorize it by
naming a session or explicitly requesting `--current`, which must resolve one
unique record from the caller's context and print the target. Both paths retain
provider safeguards and hook approvals. Run cleanup outside the workspace it
may close. Preserve provider-specific branch retention behavior and report
whether a branch was kept or deleted. Never sweep arbitrary panes by cwd or
delete paths merely because they resemble a Worktrunk trash directory.

### Editor and attachments

Keep the task prominent. Make Repository, Workspace provider, Agent, and Model
available without a separate workflow. Show effort/speed only when the selected
model supports them. Keep branch/base under additional options. Automatic
choices show the configured fallback where known. Launch errors return to the
same editable task or point to the recorded prepared session.

Retain keyboard navigation, multiline paste, Unicode, undo/redo, responsive
layout, draft recovery, image-only tasks, local clipboard import, remote file
paste, and preview/removal controls. The stored task and attachment paths are
data, including quotes, spaces, and shell metacharacters.

Keep content-addressed copies of original images, private file modes, decode
limits, and atomic writes. Removing an attachment from a draft must not delete
an original a running agent uses. Session cleanup does not sweep the attachment
store. Defer automatic garbage collection until retention has a real requirement.

Keep Herdr pixel previews as a narrow optional rendering path with terminal-cell
fallback. Both workspace providers still run inside Herdr, so this integration
has a purpose. Release streams when previews close or the editor exits. Graphics
availability must not affect request validation or attachment delivery.

## Every-file disposition

Delete a runtime file only when its necessary behavior is covered by the new
path, or when this plan explicitly removes that behavior. Temporary coexistence
is for implementation only; it must not become a shipped compatibility layer.

| Current file | Disposition and remaining necessity |
| --- | --- |
| `.gitignore` | Replace old artifact paths with root Cargo target and the new binary's build outputs. |
| `LICENSE.md` | Retain existing notice and attribution. |
| `README.md` | Rewrite around composing, configuring, launching, and cleanup. Include native and Worktrunk setups and explicit import. Remove old worktree/PR manual. |
| `herdr-plugin.toml` | Replace identity, actions, and panes. Keep compose and remove-current; invoke the binary directly with pinned context. Delete picker and PR actions. |
| `build-composer.sh` | Replace with a short root build script for the new executable; retain locked builds and atomic installation. |
| `composer/Cargo.toml` | Move to root, rename package/binary, add a real TOML parser. Reassess every dependency against the new code. |
| `composer/Cargo.lock` | Move/regenerate with Cargo after dependency changes; keep reproducible builds. |
| `composer/src/main.rs` | Replace with root CLI entry points; move terminal restoration and draft handling to their actual owners. Retain headless rendering for tests. |
| `composer/src/app.rs` | Move and rewrite around shared request/catalog types. Retain editor and picker behavior; remove hardcoded model/effort/speed policy. |
| `composer/src/ui.rs` | Move and reshape for provider/model selection and pending/error feedback. Retain responsive rendering. |
| `composer/src/images.rs` | Move and retain bounded import, durable originals, quoted path parsing, and previews. Remove only duplication created by the new state owner. |
| `composer/src/graphics.rs` | Move and keep the small Herdr stream adapter and fallback. Remove old product labels; do not generalize it. |
| `composer.sh` | Delete. Discovery, validation, draft clearing, task rewriting, and generated runner commands move into the shared core. |
| `dispatch.sh` | Delete. Replace routing, workspace creation, startup, and delivery with the typed launch path; discard naming/model guesses, retries, and footer scraping. |
| `config.sh` | Delete. Use one TOML parser and typed validation. |
| `helpers.sh` | Delete. Keep only necessary Git/Herdr targeting and grammar behavior in their owning modules. Remove list normalization and fzf helpers. |
| `picker.sh` | Delete with its manifest entries, `open_mode`, remote-branch options, PR shortcuts, and composer-promotion bridge. |
| `remove.sh` | Delete. Replace with receipt-driven provider cleanup; remove picker, trash deletion, and legacy pane sweep. |
| `pr-status.sh` | Delete with PR actions and hook documentation. |
| `bin/sow` | Delete cached plugin lookup. Document an optional alias to the new launch command. |
| `bin/reap` | Delete cached plugin lookup. Document an optional alias to recorded-session removal. |
| `skills/sow/SKILL.md` | Replace with `skills/composer/SKILL.md` after the CLI exists. Document the actual launch/cleanup contract; do not modify installed global skills. |
| `tests/composer_test.py` | Replace shell-handoff fixtures with shared resolver and launch acceptance tests. Retain literal input, failed/newer draft, missing attachment, and pre-mutation rejection cases. |
| `tests/composer_pty_test.py` | Retain and update a focused PTY test for editing, catalog/provider selection, draft recovery, and attachment lifetime. Remove static effort assumptions. |
| `tests/config_test.sh` | Replace with TOML/catalog validation tests. Delete legacy presentation and remote-branch cases. |
| `tests/dispatch_test.sh` | Replace with request-resolution and adapter-contract tests. Delete tests whose purpose is freezing old guesses or footer scraping. |
| `tests/helpers_test.sh` | Move necessary Git identity and grammar cases to their modules. Delete shortcut, list-schema, and picker cases. |
| `tests/remove_test.sh` | Replace with receipt/ownership cleanup tests, including changing focus/defaults and partial provider failure. |
| `tests/pr_status_test.sh` | Delete with PR status support. |
| `COMPOSER_PLAN.md` | Keep this implementation contract current; remove speculative sections as decisions become code. |

Dependency review starts with the existing set: Ratatui, Crossterm, and textarea
serve terminal editing; serde/JSON serve persistence and executable boundaries;
image serves decoding and previews; SHA-256 serves durable deduplication; shlex
serves literal file-drop parsing. Keep them only while those uses remain.
Remove runtime requirements for fzf and jq. Require `wt` only for Worktrunk
sessions, and remove the PR-related `gh` requirement. Add no new graphics or
async runtime dependency without a concrete need.

## Implementation order and gates

1. **Establish the shared request and catalog.** Move the crate, introduce typed
   config and request resolution, and port the editor onto them. Feed both UI
   and CLI the same fixtures. Gate: changing catalog data changes labels,
   aliases, defaults, and supported controls without a source edit; invalid
   selections fail before workspace creation.
2. **Complete native Herdr launch.** Add durable submission, one background
   runner, preparation, startup, and prompt delivery. Gate: with `wt`, fzf, and
   jq absent from PATH, a configured agent receives the exact task and retained
   images; closing the editor leaves setup running; failed or uncertain launch
   keeps its draft and identifies any created resources.
3. **Complete Worktrunk and removal.** Add the second preparation/removal path
   and provider receipts. Gate: approved blocking hooks finish before startup,
   hook failures do not launch an agent, and cleanup still uses Worktrunk after
   the default changes to Herdr. Check the reverse direction too. Verify dirty
   files, unmerged branch retention, primary-checkout refusal, and focus races.
4. **Add the executable provider.** Reuse the concrete prepared-workspace and
   cleanup contracts. Gate: a fixture command can prepare and remove through
   the same flow; malformed output, missing command, timeout, and partial
   preparation produce useful errors without fallback or duplicate launch.
5. **Finish the clean fork.** Delete superseded files and tests, replace the
   manifest/README/agent skill, and implement explicit import. Preview supported
   mappings and ignored keys; copy drafts and attachment originals without
   modifying the source or overwriting a newer destination. Make re-import
   idempotent; never adopt old checkouts or invent cleanup receipts. Write a
   keybinding/alias mapping for user review. Gate: the old plugin can coexist,
   imported defaults use Worktrunk, and a fresh install uses Herdr.

Test behavior at the boundary that owns it. Use disposable Git repositories and
fake Herdr/provider commands for failure and race cases. Keep one focused PTY
flow and layout checks for small and wide terminals. Do not retain tests merely
to preserve deleted implementation details.

Repeat the disposable named-session integration after changes to provider or
agent contracts. The tested minimum versions are Herdr 0.8.2 and Worktrunk
0.74.0, recorded in README and the Herdr manifest. The initial integration
result is recorded above; tests/live_integration.py reproduces the flow.

Done when all five gates pass, both launch entry points use the same resolver,
configuration alone controls the model choices, cleanup follows recorded
ownership, and every retained file has the necessity listed above. The shipped
runtime must have no required Worktrunk path, duplicate shell launch pipeline,
legacy tab mode, branch browser, PR subsystem, or silent compatibility fallback.
