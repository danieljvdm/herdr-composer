# Workspace providers

Providers create and remove worktrees when `launch_mode = "worktree"`. Tab mode
uses Herdr's tab API directly and does not require or run the configured provider.
It records ownership of the new tab only; the selected checkout remains shared.
`remove --current` identifies a tab session by the caller's tab ID, so another
task in the same checkout is not selected by its path alone.

Native Herdr creates with a pinned source workspace, then removes through native
safe worktree removal. It retains the branch. Worktrunk uses
`wt switch --create ... --no-cd --format=json`, waits for blocking setup, then
opens the returned checkout with Herdr. Removal runs `wt remove CHECKOUT` from
the primary repository and closes only the recorded target workspace after
provider success. Worktrunk decides whether to retain the branch. Hooks and
approval requirements remain in force.

```sh
herdr-composer remove --session SESSION_ID
herdr-composer remove --current
```

These CLI commands authorize removal and print the pinned target. The plugin
asks for confirmation. Removal checks ownership, canonical Git identity, and
the live workspace binding again immediately before mutation. It rejects the
primary checkout. Cleanup invoked inside the target runs from a source pane,
so closing the target cannot kill the cleanup process. The source workspace
survives. There is no force option, Git fallback, pane sweep, or trash sweep.

For an executable provider:

```toml
[providers.example]
command = ["python3", "/absolute/path/to/provider.py"]
```

Use `--provider example`. [The fixture](../examples/provider.py) demonstrates the
version-1 protocol with disposable repositories. Preparation receives only
`version`, `operation="prepare"`, `launch_id`, `workspace`, and `cleanup`.
Workspace contains repository/common-dir identity, branch, pinned base commit
or null, and source workspace ID. Return one JSON object with `version=1`,
`status="prepared"`, and `receipt`:

```json
{"version":1,"launch_id":"same-id","checkout":"/canonical/path","branch":"new-branch","owned":true,"workspace":null,"pane":null,"prepared_head":null,"cleanup":{}}
```

Null workspace/pane IDs ask Composer to register the checkout. Supplied IDs
must bind to that checkout. A failed prepare may still return a valid receipt
with `status="partial"`. Removal receives the original receipt and cleanup
options and returns `{"version":1,"status":"removed"}`. Commands have a
five-minute timeout and bounded output. They must honor the launch ID and use
stderr for progress. Timeout/malformed responses never cause another prepare
call. Cleanup uses the recorded command argv even after configuration changes.
