## Compile AxVisor

* get deps
```bash
./tool/dev_env.py
cd crates/arceos && git checkout rk3588_jd4_qemu
cd crates/axvm && git checkout ivc
cd crates/axvcpu && git checkout ivc
cd crates/arm_vcpu && git checkout ivc_and_4lpt
cd crates/axaddrspace && git checkout 4_level_paging
cd crates/axhvc && git checkout ivc
```


```bash
make ARCH=aarch64 LOG=info VM_CONFIGS=configs/vms/linux-qemu-aarch64.toml:configs/vms/arceos-aarch64.toml GICV3=y NET=y SMP=2 run DISK_IMG=/home/hky/workspace/Linux/ubuntu-22.04-rootfs_ext4.img SECOND_SERIAL=y

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

see its [README](https://github.com/arceos-hypervisor/axvisor-tools/blob/ivc/axvisor-driver/README.md) about how to compile it and how to subscribe messages from guest ArceOS's ivc publisher.
