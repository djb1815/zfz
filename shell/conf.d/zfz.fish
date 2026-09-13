function __zfz_on_pwd --on-variable PWD
    # Tracking is intentionally best-effort when another writer holds the
    # database. It must never reject a successful directory change.
    command zfz --track "$PWD" >/dev/null 2>/dev/null
end
