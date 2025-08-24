#!/bin/bash
set -x

if [ -z "$1" ]; then
    echo "Usage: $0 <parameter>"
    exit 1
fi

sudo ./jailhouse disable
sudo rmmod jailhouse
sudo rmmod eqdriver
sudo insmod jailhouse.ko
sudo insmod eqdriver.ko
sudo chown $(whoami) /dev/jailhouse
sudo chown $(whoami) /dev/eqmanager
sudo ./jailhouse enable "$1"
