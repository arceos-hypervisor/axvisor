# Boot Two Linux VMs on the Firefly AIO-3588JD4 Board

## Setup TFTP Server

```bash
sudo apt-get install tftpd-hpa tftp-hpa
sudo chmod 777 /srv/tftp
```

Check if TFTP works

```bash
echo "TFTP Server Test" > /srv/tftp/testfile.txt
tftp localhost
tftp> get testfile.txt
tftp> quit
cat testfile.txt
```

You should see `TFTP Server Test` on your screen.

## Setup an NFS Server for the rootfs of VM2

```bash
sudo apt install nfs-kernel-server
sudo mkdir -p /srv/nfs/firefly-rootfs
# Download rootfs image from firefly wiki, assume rootfs.img
# expand image and partition
sudo dd if=/dev/zero of=rootfs.img bs=1M count=0 seek=16384
# ... will show which loop device the image is mounted on, assume loopX
sudo losetup -f --show rootfs.img
sudo e2fsck -f /dev/loopX && sudo resize2fs /dev/loopX
sudo losetup -D /dev/loopX
# now mount the image file to rootfs path
sudo mount -t loop rootfs.img /srv/nfs/firefly-rootfs
# Add to NFS exports
sudo cat <<EOF >> /etc/exports
/srv/nfs        192.168.XXX.0/24(rw,async,no_subtree_check,fsid=0)
/srv/nfs/firefly-rootfs 192.168.XXX.0/24(rw,async,no_subtree_check,no_root_squash)
EOF
sudo exportfs -ar
```

## Compile device tree

Before compiling the DTS, edit the bootargs in `aio-rk3588-jd4-vm2.dts` and replace `<server_ip>:<root-dir>` with your own NFS server IP and rootfs export path setup in the previous step.

```bash
dtc -o configs/vms/aio-rk3588-jd4-vm1.dtb -O dtb -I dts configs/vms/aio-rk3588-jd4-vm1.dts
dtc -o configs/vms/aio-rk3588-jd4-vm2.dtb -O dtb -I dts configs/vms/aio-rk3588-jd4-vm2.dts
```

## Prepare Linux kernel bianry

Prepare RK3588 SDK following manufacturer's instruction, checkout the Linux kernel repository to this branch: https://github.com/arceos-hypervisor/firefly-linux-bsp/tree/axvisor-wip, then build the kernel.

Copy the kernel and ramdisk image to AxVisor directory:

```bash
scp xxx@192.168.xxx.xxx:/home/xxx/firefly_rk3588_SDK/kernel/arch/arm64/boot/Image configs/vms/Image.bin
scp xxx@192.168.xxx.xxx:/home/xxx/firefly_rk3588_SDK/kernel/ramdisk.img configs/vms/ramdisk.img
```

## Compile AxVisor

* get deps

```bash
./tool/dev_env.py
cd crates/arceos && git checkout rk3588_jd4
```

* compile

```bash
make ARCH=aarch64 PLATFORM=configs/platforms/aarch64-rk3588j-hv.toml SMP=2 defconfig
make ARCH=aarch64 PLATFORM=configs/platforms/aarch64-rk3588j-hv.toml VM_CONFIGS=configs/vms/linux-rk3588-aarch64-smp-vm1.toml:configs/vms/linux-rk3588-aarch64-smp-vm2.toml LOG=debug GICV3=y upload
```

* copy to tftp dir (make xxx upload will copy the image to `/srv/tftp/axvisor` automatically)

```bash
cp axvisor_aarch64-rk3588j.img /srv/tftp/axvisor
```

## rk3588 console

上电，在 uboot 中 ctrl+C

```bash
# 这是 tftp 服务器所在的主机 ip
setenv serverip 192.168.50.97
# 这是 rk3588 所在设备的 ip (Firefly Linux 自己 DHCP 拿到的地址)
setenv ipaddr 192.168.50.8
# 使用 tftp 加载镜像到指定内存地址并 boot
setenv serverip 192.168.50.97;setenv ipaddr 192.168.50.8;tftp 0x00480000 ${serverip}:axvisor;tftp 0x10000000 ${serverip}:rk3588_dtb.bin;bootm 0x00480000 - 0x10000000;
```

The VM2 will wait for several seconds before boot to allow VM1 to setup clocks of the whole SoC first.

The VM1 output goes to the RS232 on the board (ttyS1 in Linux and serial@feb40000 in the device tree), and the VM2 output goes to the USB Type-C (ttyS2/ttyFIQ0 in Linux and serial@feb5000 in the device tree).

## Known Issues

- Resets of the ethernet in VM2 is not working, and reconfigure the NIC (e.g. with NetworkManager) may cause the VM2 to hang. Currently the initramfs will attempt to autoconfig the eth port then mount NFS as the rootfs. You may override the configuration with `ip=` kernel bootarg.
