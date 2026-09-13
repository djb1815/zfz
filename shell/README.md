# Shell integrations

This directory contains the initial Fish integration. Install or symlink
`functions/z.fish` and `conf.d/zfz.fish` into the corresponding directories
under `$XDG_CONFIG_HOME/fish` (normally `~/.config/fish`). The Rust core does
not depend on a particular shell.

The `z` function asks the executable for a NUL-terminated selection and passes
the result to `cd` as one quoted argument. This preserves spaces, newlines, and
wildcard characters without reparsing them. Output and administrative options
are passed through unchanged. The `$PWD` event handler uses the internal
`--track` operation; a tracking update may be dropped under write contention so
any history-related delay to a directory change remains tightly bounded.
