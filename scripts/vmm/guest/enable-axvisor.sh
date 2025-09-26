#!/bin/bash
set -x

JH_DIR=~/jailhouse-axvisor
JH=$JH_DIR/tools/jailhouse

if [ -z "$1" ]; then
    echo "Usage: $0 <parameter>"
    exit 1
fi

sudo $JH enable
sudo rmmod jailhouse
sudo insmod $JH_DIR/driver/jailhouse.ko
sudo chown $(whoami) /dev/jailhouse
sudo $JH enable "$1"
