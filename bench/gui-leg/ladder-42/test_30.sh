#!/bin/sh
# Feature 30: import command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_30): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
printf 'x\t9\ny\t8\n' > imp.tsv
sh ./minidb import imp.tsv || fail "import should exit 0"
[ "$(sh ./minidb get x)" = "9" ] || fail x
[ "$(sh ./minidb get y)" = "8" ] || fail y
rm -f imp.tsv
exit 0
