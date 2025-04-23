QEMU ?= qemu-system-x86_64
PORT ?= 2334
SMP ?= 4
OUT_ELF ?= ../../../apps/vmm/vmm_x86_64-linux.elf

TELNET_PORT := 4321

UV ?= 24.04

ifeq ($(UV),22.04)
RELEASE_NAME = focal
endif

ifeq ($(UV),24.04)
RELEASE_NAME = noble
endif

qemu_image := ubuntu-$(UV)-server-cloudimg-amd64.img
qemu_args := \
  -smp $(SMP) -m 16G -accel kvm -nographic \
  -machine q35,kernel_irqchip=split \
  -cpu host,-la57,-waitpkg,-kvm-asyncpf,-kvm-pv-eoi,-kvm-pv-ipi,-kvm-pv-sched-yield,-kvm-pv-unhalt,-kvm-steal-time,-kvmclock \
  -drive file=$(qemu_image) \
  -net user,id=net,hostfwd=tcp::$(PORT)-:22 -net nic,model=e1000e \
  -serial mon:stdio \
  -serial telnet:localhost:$(TELNET_PORT),server,nowait

$(qemu_image):
	wget -nc https://cloud-images.ubuntu.com/releases/$(RELEASE_NAME)/release/$(qemu_image)

.ONESHELL:
image: $(qemu_image)
	cat >user-data <<EOF
	#cloud-config
	password: 123
	chpasswd: { expire: False }
	ssh_pwauth: True
	EOF
	cloud-localds user-data.img user-data
	qemu-img resize $(qemu_image) +64G
	$(QEMU) $(qemu_args) -drive file=user-data.img,format=raw

qemu: $(qemu_image)
	$(QEMU) $(qemu_args)

qemu_debug: $(qemu_image)
	$(QEMU) $(qemu_args) -s -S

qemu_gdb:
	gdb $(OUT_ELF) \
	  -ex 'target remote localhost:1234' \
	  -ex 'continue' \
	  -ex 'disp /16i $$pc'

ssh:
	ssh -p $(PORT) ubuntu@localhost

.PHONY: image qemu debug ssh qemu_debug qemu_gdb