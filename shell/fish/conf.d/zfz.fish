function __zfz_on_pwd --on-variable PWD
    # Tracking is intentionally best-effort when another writer holds the
    # database. It must never reject a successful directory change.
    command zfz --track "$PWD" >/dev/null 2>/dev/null
end

set -l __zfz_cmd
if set -q ZFZ_CMD
    set __zfz_cmd "$ZFZ_CMD"
else
    set __zfz_cmd z
end

# Keep the executable interface available as `zfz`; only the configured alias
# performs navigation. `ZFZ_CMD=zfz` intentionally shadows the executable, and
# `command zfz` remains available for direct access.
if test -n "$__zfz_cmd"
    alias -- "$__zfz_cmd" __zfz_jump
end
