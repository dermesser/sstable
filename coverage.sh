#!/bin/bash

KCOV=kcov
KCOV_OPTS="--verify --exclude-pattern=/.cargo"
KCOV_OUT="./kcov-out/"

TEST_BIN=$(cargo test 2>&1 >/dev/null | awk '/^     Running target\/debug\// { print $2 }')

${KCOV} ${KCOV_OPTS} ${KCOV_OUT} ${TEST_BIN} && xdg-open ${KCOV_OUT}/index.html
