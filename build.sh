#!/bin/bash

YELLOW='\033[1;33m'
RESET='\e[0m'

if [ "$1" == "debug" ]; then
    printf "\nCompiling code in debug mode\n"
    cargo build | exit 1
    printf "\nMoving the debug version of clanix to bin..\n"
    sudo cp target/debug/clanix /usr/local/bin/clanix | exit 1
    printf "\n${YELLOW}Success${RESET} Successfully moved debug version to bin\n\n"
else
    printf "\nNo arguments found..\n"
    printf "\nCompiling code on release mode..\n"
    cargo build --release | exit 1
    printf "Trying to move the release version of clanix to bin..\n"
    sudo cp target/release/clanix /usr/local/bin/clanix | exit 1
    printf "\n${YELLOW}Success:${RESET} Successfully moved release version to bin\n\n"
fi
