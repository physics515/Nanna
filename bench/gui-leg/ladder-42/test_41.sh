#!/bin/sh
# Feature 41: jexport command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_41): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set b 2 && sh ./minidb set a 1 || fail set
got=$(sh ./minidb jexport)
[ "$got" = '{"a":"1","b":"2"}' ] || fail "jexport mismatch, got: $got"
exit 0
