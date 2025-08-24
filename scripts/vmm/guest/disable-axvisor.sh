#!/bin/bash
set -x

sudo rmmod eqdriver
sudo ./jailhouse disable
