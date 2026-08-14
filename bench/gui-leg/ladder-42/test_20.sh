#!/bin/sh
# Feature 20: incr non-numeric
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_20): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set s abc || fail set
sh ./minidb incr s >/dev/null 2>&1
[ $? -eq 2 ] || fail "incr non-numeric should exit 2"
[ "$(sh ./minidb get s)" = "abc" ] || fail "value should be unchanged"
exit 0
