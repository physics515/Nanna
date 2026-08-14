#!/bin/sh
# Feature 37: validate command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_37): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set k v || fail set
sh ./minidb validate || fail "validate should exit 0 on clean db"
printf 'brokenlinewithnotab\n' >> "$MINIDB_FILE"
sh ./minidb validate >/dev/null 2>&1
[ $? -eq 3 ] || fail "validate should exit 3 on malformed db"
exit 0
