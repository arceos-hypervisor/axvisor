#!/bin/bash
guestos=$1
current_path=$(pwd)
arceos_config_path="./tmp/arceos-aarch64.toml"
arceos_kernel_path="$current_path/tmp/arceos_aarch64-dyn.bin"
arceos_img_path="https://raw.githubusercontent.com/arceos-hypervisor/axvisor-guest/main/IMAGES/arceos/arceos_aarch64-dyn.bin"

mkdir -p ./tmp
wget -O ./tmp/arceos_aarch64-dyn.bin $arceos_img_path
cp $current_path/configs/vms/arceos-aarch64.toml ./tmp
sed -i 's|^kernel_path =.*|kernel_path = $arceos_kernel_path|' "$arceos_config_path"
./axvisor.sh run --plat aarch64-generic --arceos-args "LOG=info,SMP=4" --features "ept-level-4"  --vmconfigs "$arceos_config_path"
rm -rf tmp