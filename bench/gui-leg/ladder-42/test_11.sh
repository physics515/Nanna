#!/bin/sh
# Feature 11: list command
export LC_ALL=C
export MINIDB_FILE=./minidb_data
rm -f ./minidb_data
fail() { echo "FAIL(test_11): $1"; exit 1; }
[ -f ./minidb ] || fail "./minidb does not exist"
sh ./minidb set b 2 && sh ./minidb set a 1 && sh ./minidb set c 3 || fail set
got=$(sh ./minidb list)
want=$(printf 'a\nb\nc')
[ "$got" = "$want" ] || fail "list should print sorted keys, got: $got"
exit 0
