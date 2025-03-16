#!/bin/bash

# Install packages
sudo sed -i "s/http:\/\/archive.ubuntu.com/http:\/\/mirrors.tuna.tsinghua.edu.cn/g" /etc/apt/sources.list
sudo apt-get update
sudo apt-get install -y build-essential python3-mako

# Create a hypervisor image link to /lib/firmware/evm-intel.bin
sudo mkdir -p /lib/firmware
sudo ln -sf ~/evm-intel.bin /lib/firmware

# Clone jailhouse-equation, apply patches and build
# git clone https://github.com/EquationOS/jailhouse-equation
cd jailhouse-equation
make

# Update grub config file to update kernel cmdline
./update-cmdline.sh
sudo update-grub

echo
echo "Setup OK!"
echo "Press ENTER to reboot..."
read
sudo reboot
