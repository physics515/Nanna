#!/bin/sh
# Feature 5: set overwrites
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_05): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set k first || fail set1
sh ./minidb set k second || fail set2
v=$(sh ./minidb get k)
[ "$v" = "second" ] || fail "get should print second, got: $v"
n=$(grep -c "^k	" "$MINIDB_FILE")
[ "$n" -eq 1 ] || fail "db should hold exactly one record for k, got $n"
exit 0
