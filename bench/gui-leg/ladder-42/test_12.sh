#!/bin/sh
# Feature 12: clear command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_12): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set a 1 && sh ./minidb set b 2 || fail set
sh ./minidb clear || fail "clear should exit 0"
[ "$(sh ./minidb count)" = "0" ] || fail "count after clear should be 0"
exit 0
