## Setup TFTP Server

```bash
sudo apt-get install tftpd-hpa tftp-hpa
sudo chmod 777 /srv/tftp
```

judge if TFTP works

```bash
echo "TFTP Server Test" > /srv/tftp/testfile.txt
tftp localhost
tftp> get testfile.txt
tftp> quit
cat testfile.txt 
```

You should see `TFTP Server Test` on your screen.

## Compile device tree

```bash
dtc -o aio-rk3588-jd4.dtb -O dtb -I dts aio-rk3588-jd4.dts
```

## Compile AxVisor

* compile

```bash
make ARCH=aarch64 PLATFORM=configs/platforms/aarch64-rk3588j-hv.toml defconfig
make ARCH=aarch64 PLATFORM=configs/platforms/aarch64-rk3588j-hv.toml image
make ARCH=aarch64 PLATFORM=configs/platforms/aarch64-rk3588j-hv.toml VM_CONFIGS=configs/vms/linux-rk3588-aarch64-smp.toml image
```

* copy to tftp dir

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
tftp 0x00480000 ${serverip}:axvisor;tftp 0x10000000 ${serverip}:rk3588_dtb.bin;bootm 0x00480000 - 0x10000000;
```

