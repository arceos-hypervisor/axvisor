#!/bin/bash
set -x

JH_DIR=~/jailhouse-axvisor
JH=$JH_DIR/tools/jailhouse

sudo $JH disable
