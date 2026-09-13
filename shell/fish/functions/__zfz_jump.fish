function __zfz_jump --description 'Jump to a directory from zfz history'
    if test (count $argv) -eq 0
        echo 'zfz: interactive selection is not implemented yet' >&2
        return 2
    end

    if test (count $argv) -eq 1
        switch $argv[1]
            case -h --help
                command zfz --help
                return $status
        end
    end

    set -l target (command zfz --jump $argv | string split0)
    set -l zfz_status $pipestatus[1]
    if test $zfz_status -ne 0
        return $zfz_status
    end
    if test (count $target) -ne 1
        echo 'zfz: executable returned an invalid selection' >&2
        return 2
    end

    builtin cd -- "$target"
end
