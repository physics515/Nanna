#!/bin/sh
# Feature 38: repair command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_38): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set a 1 || fail set
printf 'brokenlinewithnotab\n' >> "$MINIDB_FILE"
sh ./minidb repair || fail "repair should exit 0"
sh ./minidb validate || fail "validate should pass after repair"
[ "$(sh ./minidb get a)" = "1" ] || fail "valid record should survive repair"
[ "$(sh ./minidb count)" = "1" ] || fail "count should be 1"
exit 0
