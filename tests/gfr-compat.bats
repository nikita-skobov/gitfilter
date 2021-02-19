function setup() {
    test_folder="$BATS_TMPDIR/gfr-compat"
    mkdir -p "$test_folder"
    BATS_TMPDIR="$test_folder"
    cd $test_folder
}

function teardown() {
    cd $BATS_TMPDIR
    cd ..
    if [[ -d gfr-compat ]]; then
        rm -rf gfr-compat/
    fi
}

@test 'simple path include' {
    echo "$GITFILTERCLI"
    echo "$PATHTOREACTROOT"

    cd "$PATHTOREACTROOT"
    # this will generate the fast export comparison file:
    # .git/filter-repo/fast-export.filtered
    git filter-repo --force --dry-run --refs master --path packages/react-dom/
    COMPARE_PATH="$PATHTOREACTROOT/.git/filter-repo/fast-export.filtered"
    [[ -f $COMPARE_PATH ]]
    COMPARE_PATH_ACTUAL="$BATS_TMPDIR/gitfiltercli.output"
    "$GITFILTERCLI" > "$COMPARE_PATH_ACTUAL"

    echo "Comparing $COMPARE_PATH to $COMPARE_PATH_ACTUAL"
    echo "File sizes:"
    echo "$(wc -c $COMPARE_PATH)"
    echo "$(wc -c $COMPARE_PATH_ACTUAL)"
    cmp -s "$COMPARE_PATH" "$COMPARE_PATH_ACTUAL"
}
