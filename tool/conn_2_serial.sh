#!/bin/bash

# Print usage information
if [[ "$1" == "--help" || "$1" == "-h" ]]; then
	echo "Usage: $0 [FIRST_PORT] [SECOND_PORT]"
	echo "Default ports are 4321 and 4322 if not specified."
	exit 0
fi

# Determine the first and second ports to use
FIRST_PORT=${1:-4321}
SECOND_PORT=${2:-4322}

# Create tmux session if it doesn't exist
if ! tmux has-session -t "mysession"; then
	tmux new-session -d -s mysession
	tmux split-window -h
fi

# Send C-c to both panes to clear any previous commands
tmux send-keys -t mysession:0.0 C-c
tmux send-keys -t mysession:0.1 C-c

# Send telnet commands to both panes
tmux send-keys -t mysession:0.0 "telnet localhost ${FIRST_PORT}" C-m
tmux send-keys -t mysession:0.1 "sleep 2; telnet localhost ${SECOND_PORT}" C-m

# Attach to the tmux session if not already attached
session=$(tmux list-sessions 2>/dev/null | grep 'mysession')
if [[ -n "$session" && "$session" == *"attached"* ]]; then
	echo "Session 'mysession' is already attached."
	exit 0
fi

tmux attach-session -t mysession
