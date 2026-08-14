#!/bin/sh
# Feature 4: get missing key
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_04): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
out=$(sh ./minidb get nope 2>/dev/null)
rc=$?
[ $rc -eq 1 ] || fail "exit code should be 1, got $rc"
[ -z "$out" ] || fail "should print nothing, got: $out"
exit 0
