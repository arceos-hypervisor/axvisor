#!/bin/bash

JH_DIR=~/jailhouse-equation
JH=$JH_DIR/tools/jailhouse

sudo rmmod eqdriver

sudo $JH disable
