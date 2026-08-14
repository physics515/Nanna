#!/bin/sh
# Feature 32: sum command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_32): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb mset a 1 b 2 c abc || fail mset
[ "$(sh ./minidb sum)" = "3" ] || fail "sum should be 3"
exit 0
