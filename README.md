<!-- <div align="center">

<img src="https://arceos-hypervisor.github.io/doc/assets/logo.svg" alt="axvisor-logo" width="64">

</div> -->

<h2 align="center">AxVisor</h1>

<p align="center">A unified modular hypervisor based on ArceOS.</p>

<div align="center">

[![GitHub stars](https://img.shields.io/github/stars/arceos-hypervisor/axvisor?logo=github)](https://github.com/arceos-hypervisor/axvisor/stargazers)
[![GitHub forks](https://img.shields.io/github/forks/arceos-hypervisor/axvisor?logo=github)](https://github.com/arceos-hypervisor/axvisor/network)
[![license](https://img.shields.io/github/license/arceos-hypervisor/axvisor)](https://github.com/arceos-hypervisor/axvisor/blob/master/LICENSE)

</div>

English | [中文](README_CN.md)

# Introduction

AxVisor is a hypervisor implemented based on the ArceOS unikernel framework. Its goal is to leverage the foundational operating system features provided by ArceOS to implement a unified modular hypervisor.

"Unified" refers to using the same codebase to support x86_64, Arm (aarch64), and RISC-V architectures simultaneously, in order to maximize the reuse of architecture-independent code and simplify development and maintenance costs.

"Modular" means that the functionality of the hypervisor is decomposed into multiple modules, each implementing a specific function. The modules communicate with each other through standard interfaces to achieve decoupling and reuse of functionality.

## Architecture

The software architecture of AxVisor is divided into five layers as shown in the diagram below. Each box represents an independent module, and the modules communicate with each other through standard interfaces.

![Architecture](https://arceos-hypervisor.github.io/doc/assets/arceos-hypervisor-architecture.png)

The complete architecture description can be found in the [documentation](https://arceos-hypervisor.github.io/doc/arch_cn.html).

## Hardwares

Currently, AxVisor has been verified on the following platforms:

- [x] QEMU ARM64 virt (qemu-max)
- [x] Rockchip RK3568 / RK3588
- [x] 黑芝麻华山 A1000

## Guest VMs

Currently, AxVisor has been verified in scenarios with the following systems as guests:

- [ArceOS](https://github.com/arceos-org/arceos)
- [Starry-OS](https://github.com/Starry-OS)
- [NimbOS](https://github.com/equation314/nimbos)
- Linux
  - currently only Linux with passthrough device on aarch64 is tested.
  - single core: [config.toml](configs/vms/linux-qemu-aarch64.toml) | [dts](configs/vms/linux-qemu.dts)
  - smp: [config.toml](configs/vms/linux-qemu-aarch64-smp2.toml) | [dts](configs/vms/linux-qemu-smp2.dts)


# Build and Run

After AxVisor starts, it loads and starts the guest based on the information in the guest configuration file. Currently, AxVisor supports loading guest images from a FAT32 file system and also supports binding guest images to the hypervisor image through static compilation (using include_bytes).

## Build Environment

AxVisor is written in the Rust programming language, so you need to install the Rust development environment following the instructions on the official Rust website. Additionally, you need to install cargo-binutils to use tools like rust-objcopy and rust-objdump.

```console
cargo install cargo-binutils
```

If necessary, you may also need to install [musl-gcc](http://musl.cc/x86_64-linux-musl-cross.tgz) to build guest applications.

## Configuration Files

Since configuring the guest is a complex process, AxVisor chooses to use TOML files to manage the guest configurations. These configurations include the virtual machine ID, virtual machine name, virtual machine type, number of CPU cores, memory size, virtual devices, passthrough devices, and more. In the source code, the `./config/vms` directory contains some example templates for guest configurations.

In addition, you can use the [axvmconfig](https://github.com/arceos-hypervisor/axvmconfig) tool to generate a custom configuration file. For detailed information, refer to the [axvmconfig](https://arceos-hypervisor.github.io/axvmconfig/axvmconfig/index.html) documentation.

## Load and run from file system

### NimbOS as guest 

1. Execute script to download and prepare NimbOS image.

   ```shell
   ./scripts/nimbos.sh --arch aarch64
   ```

2. Execute `./axvisor.sh defconfig` to set up the development environment and generate AxVisor config `.hvconfig.toml`.

3. Edit the `.hvconfig.toml` file to set the `vmconfigs` item to the path of your guest configuration file, for example:

   ```toml
   plat = "aarch64-generic"
   features = ["fs", "ept-level-4"]
   arceos_args = [ "BUS=mmio","BLK=y", "DISK_IMG=tmp/nimbos-aarch64.img", "LOG=info"]
   vmconfigs = [ "configs/vms/nimbos-aarch64.toml",]
   ```

4. Execute `./axvisor.sh run` to build AxVisor and start it in QEMU.

### More
   TODO

## Load and run from memory
### linux as guest 

1. [See linux build help.](https://github.com/arceos-hypervisor/guest-test-linux) to get Image and rootfs.img.

2. Modify the configuration items in the corresponding `./configs/vms/<ARCH_CONFIG>.toml`

   ```console
   mkdir -p tmp
   cp configs/vms/linux-qemu-aarch64-mem.toml tmp/
   ```

   - `image_location="memory"` indicates loading from the memory.
   - `kernel_path` kernel_path specifies the path of the kernel image in the workspace.
   - `dtb_path` specifies the path of the dtb file in the workspace.
   - others

3. Edit the `.hvconfig.toml` file to set the `vmconfigs` item to the path of your guest configuration file, for example:

   ```toml
   arceos_args = [
      "BUS=mmio",
      "BLK=y",
      "MEM=8g",
      "LOG=debug",
      "QEMU_ARGS=\"-machine gic-version=3  -cpu cortex-a72  \"",
      "DISK_IMG=\"tmp/rootfs.img\"",
   ]
   vmconfigs = [ "tmp/linux-qemu-aarch64-mem.toml"]
   ```

4. Execute `./axvisor.sh run` to build AxVisor and start it in QEMU.

### More
   TODO

# Contributing

Feel free to fork this repository and submit a pull request.

You can refer to these [discussions]((https://github.com/arceos-hypervisor/axvisor/discussions)) to gain deeper insights into the project's ideas and future development direction.

## Development

To contribute to AxVisor, you can follow these steps:

1. Fork the repository on GitHub.
2. Clone your forked repository to your local machine.
3. Create a new branch for your feature or bug fix.
4. Make your changes and commit them with clear messages.
5. Push your changes to your forked repository.
6. Open a pull request against the main branch of the original repository.

To develop crates used by AxVisor, you can use the following command to build and run the project:

```bash
cargo install cargo-lpatch
cargo lpatch -n deps_crate_name
```

Then you can modify the code in the `crates/deps_crate_name` directory, and it will be automatically used by AxVisor.

## Contributors

This project exists thanks to all the people who contribute.

<a href="https://github.com/arceos-hypervisor/axvisor/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=arceos-hypervisor/axvisor" />
</a>

# License

AxVisor uses the following open-source license:

- Apache-2.0
- MulanPubL-2.0
- MulanPSL2
- GPL-3.0-or-later
