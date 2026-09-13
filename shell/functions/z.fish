function z --description 'Jump to a directory from zfz history'
    # Output and administrative operations belong to the executable. All
    # remaining invocations request one NUL-terminated path for cd.
    set -l parsing_options 1
    for argument in $argv
        if test $parsing_options -eq 0
            continue
        end
        switch $argument
            case --
                set parsing_options 0
            case --echo --list --add '--add=*' --track '--track=*' --remove '--remove=*' --remove-recursive '--remove-recursive=*' --help
                command zfz $argv
                return $status
            case '-*'
                if string match --quiet --regex '[elaxXh]' -- (string replace --regex '^-' '' -- $argument)
                    command zfz $argv
                    return $status
                end
        end
    end

    set -l target (command zfz --null $argv | string split0)
    set -l zfz_status $pipestatus[1]
    if test $zfz_status -ne 0
        return $zfz_status
    end
    if test (count $target) -ne 1
        echo 'z: zfz returned an invalid selection' >&2
        return 2
    end

    builtin cd -- "$target"
end
