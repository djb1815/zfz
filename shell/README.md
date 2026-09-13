# Shell integrations

This directory contains shell-specific integrations. For Fish, install or
symlink `fish/functions/__zfz_jump.fish` and `fish/conf.d/zfz.fish` into the
corresponding directories under `$XDG_CONFIG_HOME/fish` (normally
`~/.config/fish`). The Rust core does not depend on a particular shell.

The private `__zfz_jump` function asks the executable for a NUL-terminated
selection and passes the result to `cd` as one quoted argument. The startup
script aliases `z` to that function by default; set `ZFZ_CMD` before the script
loads to choose a different command name. Setting `ZFZ_CMD=zfz` intentionally
shadows the executable with the navigation alias; `command zfz` remains
available. Setting `ZFZ_CMD` to an empty string creates no alias.

This preserves spaces, newlines, and wildcard characters without reparsing
them. The alias is navigation-only: use `z docs` to jump, and use the executable
directly for output and administration, for example `zfz --list docs`,
`zfz --echo docs`, and `zfz --remove /old/path`. A no-argument alias invocation
is reserved for the future interactive workflow and currently returns status 2
with a clear error. A sole `-h` or `--help` displays executable help.

The `$PWD` event handler uses the internal `--track` operation; a tracking
update may be dropped under write contention so any history-related delay to a
directory change remains tightly bounded.
