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

## Setup rootfs for VM2

### SATA disk

Burn the rootfs image to an **M.2 SATA** SSD with whatever tools you prefer (e.g. dd, rufus, Balena Etcher, or even a loop mount and rsync), and install it to the back of the board. Specify the proper partition identifier (e.g. `/dev/sda1`) in the DTS bootargs.

The kernel need built with SCSI disk, libata and AHCI platform support, or the corresponding kernel modules need to be put into the initramfs image. The default config in the Firefly SDK builds them as kernel modules but are not included in the initramfs image, hence the kernel failed to recognize the disk and mount the root partition.

### NFS-root (optional)

This works when directly attached storage is not available for VM2.

Setup an NFS server:

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

Before compiling the DTS, edit the bootargs in `aio-rk3588-jd4-vm2.dts` and specify an NFS root as `root=/dev/nfs nfsroot=<server_ip>:<root-dir>` where `<server_ip>:<root-dir>` is your own NFS server IP and rootfs export path setup in the previous step.

## Compile device tree

```bash
dtc -o configs/vms/aio-rk3588-jd4-vm1.dtb -O dtb -I dts configs/vms/aio-rk3588-jd4-vm1.dts
dtc -o configs/vms/aio-rk3588-jd4-vm2.dtb -O dtb -I dts configs/vms/aio-rk3588-jd4-vm2.dts
```

## Prepare Linux kernel binary

Prepare RK3588 SDK following manufacturer's instruction, checkout the Linux kernel repository to this branch: https://github.com/arceos-hypervisor/firefly-linux-bsp/tree/axvisor-rk3588-jd4-rt89, then build the kernel. This branch has PREEMPT_RT patches and native SCS, SATA and AHCI support, bootable on both VMs.

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
make ARCH=aarch64 PLATFORM=configs/platforms/aarch64-rk3588j-hv.toml SMP=4 defconfig
make ARCH=aarch64 PLATFORM=configs/platforms/aarch64-rk3588j-hv.toml SMP=4 VM_CONFIGS=configs/vms/linux-rk3588-aarch64-smp-vm1.toml:configs/vms/linux-rk3588-aarch64-smp-vm2.toml LOG=debug GICV3=y upload
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

* Resets of the ethernet in VM2 is not working, and reconfigure the NIC (e.g. with NetworkManager) may cause the VM2 to hang. Currently the initramfs will attempt to autoconfig the eth port when NFS-root is used. You may override the configuration with `ip=` kernel bootarg.
* Execute `reboot` in either VM would reset the whole board, which may be unexpected for the other VM. You may `shutdown` VM2 first, then do shutdown or reboot in VM1.
