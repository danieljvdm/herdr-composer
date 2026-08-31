# Recovery

Each submission atomically saves a private, versioned session record before
workspace mutation. A source-workspace runner reads the frozen request and
holds an exclusive record lock. Duplicate runners report state instead of
replaying. Closing the editor cannot stop setup.

Records distinguish NotSent, Unknown, and Confirmed delivery. Herdr startup runs
once and waits for readiness. The prompt runs once with a bounded lifecycle
wait. A delivery-attempt marker precedes input. Only `agent_prompted` confirms
delivery and clears the submitted draft revision. `agent_blocked` rejects input;
stalls, timeouts, and lost responses remain Unknown. The record keeps the
structured response. Settings are reported as requested, without footer
scraping or an unsupported claim that runtime settings were verified.

On failure, open the workspace named in the record and inspect its runner/agent.
For an approval dialog, resolve it yourself. For Unknown delivery, inspect
before manually sending the task. Composer has no automatic resume, resend, or
rollback. If cleanup was refused because of dirty work, save that work and run
removal again. A timeout during removal requires provider inspection. If
provider removal succeeded but workspace closure failed, rerun removal from
the source workspace to finish closure.

Concurrent editors cannot overwrite a newer draft. Closing a stale editor saves
its text and attachments under `draft-conflicts/` and prints that recovery path.

Successful removal replaces the task payload with a minimal removal receipt.
Retained images remain available to other drafts and sessions.

## Import old drafts and configuration

```sh
herdr-composer import-worktrunk /path/to/old-plugin-data --preview
herdr-composer import-worktrunk /path/to/old-plugin-data
```

The supplied directory may contain `config.toml`, `drafts/`, and `attachments/`,
or separate `config/` and `state/` subdirectories. You can also import the old
config directory and old state directory in separate invocations. Preview
prints supported mappings and ignored keys without writing. Import copies
originals, retains all imported drafts under `imports/worktrunk/`, and restores
the newest source draft for a context when no destination draft exists. It
keeps all existing destination config/drafts, making repeat imports idempotent.
Imported drafts explicitly select new worktrees. Imported defaults use Worktrunk;
fresh configuration uses Herdr worktrees.
Source files, bindings, wrappers, and existing checkouts are untouched. No
cleanup receipts are invented for old workspaces.

| Old choice | Composer mapping to review |
| --- | --- |
| `default_agent` | `defaults.agent` |
| `dispatch_focus` | `defaults.focus` |
| `disabled_agents` | `agents.ID.enabled=false` |
| `worktrunk.dispatch` action | `composer.compose` |
| `worktrunk.remove-current` action | Keep for general worktree cleanup |
| `sow` | Optional `bin/sow` wrapper forwards launches to Composer |
| `reap` | Keep the Worktrunk cleanup command |
| Branch browser, PR actions, worktree tabs | Keep herdr-worktrunk installed |
