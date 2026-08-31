---
name: composer
description: Launch a task with Herdr Composer, inspect its recorded outcome, or remove a Composer-owned session when the user requests these actions.
---

# Herdr Composer

Use the installed `herdr-composer` executable. Read `herdr-composer --help` for
CLI options and `herdr-composer catalog --json` for available settings.

For a requested launch, pass task text as one literal argv value or on stdin:

```sh
herdr-composer launch --repo /absolute/repository --agent codex - < task.txt
```

Select only settings the user requested or the catalog supports. Automatic
omits native flags unless configured defaults apply. Add retained images with
repeated `--attach PATH`. The provider defaults to native Herdr; use
`--provider worktrunk` when requested. Explicit branches must be new.

Keep the returned session ID and record path. Submission queues a durable
background runner; it does not claim prompt delivery. Inspect the session
record for Confirmed, NotSent, or Unknown. If delivery is Unknown, inspect the
recorded agent before sending any more input. Failed preparation stays available
for inspection. Hook approval belongs to the user.

For user-authorized cleanup, use `herdr-composer remove --session ID`. Use
`--current` only when the user's intended target is the calling workspace. The
command prints and pins that target, then follows its recorded provider. It
removes recorded Composer sessions only. A failed cleanup needs inspection of
the record and provider outcome; preserve safeguards and retained originals.

Configuration, import, and executable-provider details are in the repository's
README.md. This skill does not install aliases or modify global skills.
