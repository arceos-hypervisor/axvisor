## Compile AxVisor

* get deps
```bash
./tool/dev_env.py
./tool/check_branch.sh
```


```bash
make ARCH=aarch64 LOG=info VM_CONFIGS=configs/vms/linux-qemu-aarch64.toml:configs/vms/arceos-aarch64.toml GICV3=y NET=y SMP=2 run DISK_IMG=/PATH/ubuntu-22.04-rootfs_ext4.img SECOND_SERIAL=y

telnet localhost 4321
```

## Test AxVisor IVC

* Compile arceos ivc tester as guest VM 2

repo: https://github.com/arceos-hypervisor/arceos/tree/ivc_tester

```bash
make ARCH=aarch64 A=examples/ivc_tester defconfig
make ARCH=aarch64 A=examples/ivc_tester build
# You can get `examples/ivc_tester/ivc_tester_aarch64-qemu-virt.bin`,
# whose path should be set to `kernel_path` field in `configs/vms/arceos-aarch64.toml`.
```

* Build and install axvisor-driver

```bash
git clone git@github.com:arceos-hypervisor/axvisor-tools.git --branch ivc
```

see its [README](https://github.com/arceos-hypervisor/axvisor-tools/blob/ivc/ivc/kernel_driver/README.md) about how to compile it and how to subscribe messages from guest ArceOS's ivc publisher.

## Precautions

1. Compile the dtb for linux that supports gicv3

```bash
dtc -I dts -O dtb -o qemu_gicv3.dtb configs/vms/qemu_gicv3.dts
```

2. The rootfs of ubuntu has too little content in the base version and cannot be used with insmod and apt, etc. The regular version is not ext4 or fails to start. You can directly download the modified version through this link

```bash
https://cloud.tsinghua.edu.cn/f/3b37f08048534b708050/?dl=1
```

3. When compiling axvisor.ko, ensure that the version of the linux kernel being started is consistent with the version of KDIR here; otherwise, it will fail in insmod axvisor.ko

4. After starting linux, connect to a new port, the password is root

```bash
ssh -p 5555 root@localhost
```
