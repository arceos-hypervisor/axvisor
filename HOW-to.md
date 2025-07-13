```
cd scripts
./dev_deps.sh
```

## Getting Started

### Build

* build shim

```bash
cd deps/shim
make  LOG=debug
```

* build axcli

```bash
cd deps/axvisor-tools/axcli
cargo build --release
```

* build eqdriver

```bash
cd deps/axvisor-tools/eqdriver
sudo KDIR=/lib/modules/6.8.0-62-generic/build make

* build axvisor

```bash
make PLATFORM=x86_64-qemu-linux defconfig
make PLATFORM=x86_64-qemu-linux SMP=4 LOG=debug scp_to_qemu
```

### Test in QEMU (ubuntu as the guest OS)

1. Download the guest image and run in QEMU:

    ```bash
    cd scripts/vmm/host
    make image          # download image and configure for the first time
    make qemu           # execute this command only for subsequent runs
    ```

    You can login the guest OS via SSH. The default username and password is `ubuntu` and `123`. The default port is `2334` and can be changed by QEMU arguments.

2. Copy helpful scripts into the guest OS:

    ```bash
    scp -P 2334 scripts/vmm/guest/* ubuntu@localhost:/home/ubuntu # in host
    ```

3. Setup in the guest OS:

    Here, you need to copy the [jailhouse-equation](https://github.com/EquationOS/jailhouse-equation) manually, because it is still WIP and not published.

    Copy jailhouse-equation dir:
    ```bash
    scp -P 2334 -r deps/jailhouse-equation ubuntu@localhost:~/ # in host
    ```
    Copy axcli tool:
    ```bash
    scp -P 2334 deps/axvisor-tools/axcli/target/release/axcli ubuntu@localhost:~/
    ```
    Copy eqdriver ko:
    ```bash
    scp -P 2334 deps/axvisor-tools/eqdriver/eqdriver.ko ubuntu@localhost:~/ 
    ```

    Then run `setup.sh` in guest, (you only need to run it once see [`setup.sh`](scripts/guest/setup.sh) for details).

    ```bash
    ssh -p 2334 ubuntu@localhost    # in host
    ./setup.sh                      # in guest
    ```

4. Compile Jailhouse:

    You need to do this each time after modifing the jailhouse code.

    ```
    cd jailhouse-equation
    make
    ```

5. Enable AxVisor

    `./enable-axvisor.sh 1`

    Parameter `1` means CPU number reserved for ArceOS.

6. Initialize Shim, install eqdriver kernel module.
    `sudo insmod eqdriver.ko`
    `./axcli instance init`

7. Execute a Linux executible
    `sudo ./axcli instance execute /usr/bin/echo 123`

## Development

    `cd scripts/ && ./dev_deps.sh`
