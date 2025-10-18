## Getting Started

### Build

* prepare

```bash
git submodule update --init
```

* build axvisor

```bash
make PLATFORM=x86_64-nuc15-linux SMP=14 defconfig
make PLATFORM=x86_64-nuc15-linux SMP=14 LOG=info build
```

You can find the binary product `axvisor_x86_64-nuc15-linux.bin` in current dir.

### Test in x86_64 hardware

We use a ASUS NUC15 with 14 cores Intel(R) Core(TM) Ultra 5 225H.

```bash
xxx@225h-nuc15:~$ lscpu
Architecture:             x86_64
  CPU op-mode(s):         32-bit, 64-bit
  Address sizes:          42 bits physical, 48 bits virtual
  Byte Order:             Little Endian
CPU(s):                   14
  On-line CPU(s) list:    0-13
Vendor ID:                GenuineIntel
  Model name:             Intel(R) Core(TM) Ultra 5 225H
    CPU family:           6
    Model:                197
    Thread(s) per core:   1
    Core(s) per socket:   1
    Socket(s):            14
    Stepping:             2
    CPU(s) scaling MHz:   20%
    CPU max MHz:          6200.0000
    CPU min MHz:          400.0000
```
0. Environment preparation.

First, you need to modify the Linux cmdline to limit the memory size occupied by host Linux and reserve a portion of memory space starting from `0x40000000` for the hypervisor

There is a script at [setup.sh](https://github.com/arceos-hypervisor/axvisor-tools/blob/boot_linux/scripts/setup.sh).

This script will also create soft links between `evm-intel.bin` and `/lib/firmware/evm-intel.bin`, and make sure there is an `evm-intel.bin` file in your home directory, which should be the AxVisor's binary image `axvisor_x86_64-nuc15-linux.bin` you just build.

You only need to prepare it once.

    ```bash
    xxx@225h-nuc15:~/axvisor-tools$ ./scripts/setup.sh

    xxx@225h-nuc15:~$ cat /proc/cmdline
    BOOT_IMAGE=/boot/vmlinuz-6.8.0-59-generic root=UUID=7e378681-5d44-424e-8a32-b41163aaeb5b ro memmap=0xa000000$0x40000000 mem=16G
    xxx@225h-nuc15:~$ ls /lib/firmware/evm-intel.bin -al
    lrwxrwxrwx 1 root root 29  9月 26 12:16 /lib/firmware/evm-intel.bin -> $(PATH/OF/YOUR/HOME)/evm-intel.bin
    ```

Secondly, you need to connect to the serial port of the hardware (corresponding to 0x3f8, please ensure that the host Linux does not occupy this serial port and cause output confusion)

1. Download and compile the `axvisor-tools` repo.

    ```bash
    xxx@225h-nuc15:~$ git clone https://github.com/arceos-hypervisor/axvisor-tools.git --branch boot_linux
    xxx@225h-nuc15:~$ cd axvisor-tools && make install
    ```

    You will find the compiled product in the `out` folder.

    ```bash
    xxx@225h-nuc15:~/axvisor-tools$ ls out
    axcli  jailhouse  jailhouse.ko
    ```

2. Enable AxVisor and downgrade the host Linux to a guest VM.

    ```bash
    xxx@225h-nuc15:~/axvisor-tools$ ./scripts/enable-axvisor.sh 1
    ```

    You should see the AxVisor logo and related initialization information on the physical serial port.

    Parameter `1` means CPU number reserved for Guest VMs.

    You can use the [`test_hypercall.c`](https://github.com/arceos-hypervisor/axvisor-tools/blob/boot_linux/scripts/test_hypercall.c) to check if you are in the non-root mode.

    ```bash
    xxx@225h-nuc15:~/axvisor-tools$ gcc scripts/test_hypercall.c -o out/test_hypercall
    xxx@225h-nuc15:~/axvisor-tools$ ./out/test_hypercall
    Execute VMCALL OK.
    You are in the Guest mode.
    ```

3. Boot Guest VM.

    We support booting slightly modified [linux-5.10.35-rt](https://github.com/arceos-hypervisor/linux-5.10.35-rt/tree/tracing) or [Nimbos](https://github.com/equation314/nimbos) (A RTOS) as guestVM.

    * To boot Linux, you need a very simple bootloader [vlbl](https://github.com/arceos-hypervisor/vlbl-x86), you can download the binary image [here](https://github.com/arceos-hypervisor/vlbl-x86/releases/tag/v0.1.0) directly.
    * To boot Nimbos, you can use [axvm-bios-x86](https://github.com/arceos-hypervisor/axvm-bios-x86), you can download the binary image [here](https://github.com/arceos-hypervisor/axvm-bios-x86/releases/tag/v0.1) directly.

    Refer to config templates provided in (axvisor-tools/cfgs)[https://github.com/arceos-hypervisor/axvisor-tools/tree/boot_linux/cfgs] for more details.

    cmds:
    ```bash
    # To boot Linux
    ./out/axcli vm create --name guest_linux --cfg-path cfgs/linux-x86_64.toml
    ```

## Boot Linux upon QEMU/KVM

```bash
qemu-system-x86_64 -machine q35 -m 2G -nographic -cpu host --enable-kvm \
-kernel images/linux-x86_64.bin -initrd images/initramfs-busybox-x86_64.cpio.gz \
-append "root=/dev/ram0 rw rootfstype=ext4 console=ttyS0 init=/linuxrc" \
-net user,id=net,hostfwd=tcp::2333-:22 -net nic,model=e1000e \
-serial mon:stdio
```
