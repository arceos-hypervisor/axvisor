use std::os::arceos::modules::axhal;

use axaddrspace::GuestVirtAddr;
use axerrno::{AxResult, ax_err, ax_err_type};
use axhvc::{HyperCallCode, HyperCallResult};
use axvcpu::AxVcpuAccessGuestState;

use crate::libos::instance::InstanceRef;
use crate::libos::percpu::{current_eptp, current_instance};
use crate::vmm::VCpuRef;

pub struct InstanceCall {
    vcpu: VCpuRef,
    instance: InstanceRef,
    process_id: usize,
    code: HyperCallCode,
    args: [u64; 6],
}

impl InstanceCall {
    pub fn new(
        vcpu: VCpuRef,
        instance: InstanceRef,
        process_id: usize,
        code: u64,
        args: [u64; 6],
    ) -> AxResult<Self> {
        let code = HyperCallCode::try_from(code as u32).map_err(|e| {
            warn!("Invalid hypercall code: {} e {:?}", code, e);
            ax_err_type!(InvalidInput)
        })?;

        Ok(Self {
            vcpu,
            instance,
            process_id,
            code,
            args,
        })
    }

    pub fn execute(&self) -> HyperCallResult {
        // First, check if the vcpu is allowed to execute the hypercall.
        if self.code.is_privileged() ^ self.vcpu.get_arch_vcpu().guest_is_privileged() {
            warn!(
                "{} vcpu trying to execute {} hypercall {:?}",
                if self.vcpu.get_arch_vcpu().guest_is_privileged() {
                    "Privileged"
                } else {
                    "Unprivileged"
                },
                if self.code.is_privileged() {
                    "privileged"
                } else {
                    "unprivileged"
                },
                self.code
            );
            return ax_err!(PermissionDenied);
        }
        warn!("Hypercall: {:?} args: {:#x?}", self.code, self.args);

        if self.vcpu.get_arch_vcpu().guest_is_privileged() {
            self.execute_privileged()
        } else {
            self.execute_unprivileged()
        }
    }

    fn execute_privileged(&self) -> HyperCallResult {
        match self.code {
            _ => {
                unimplemented!()
            }
        }
    }

    fn execute_unprivileged(&self) -> HyperCallResult {
        match self.code {
            HyperCallCode::HDebug => self.debug(),
            HyperCallCode::HExitInstance => self.exit_instance(self.args[0]),
            HyperCallCode::HClone => self.clone(),
            HyperCallCode::HRead => self.read(self.args[0], self.args[1], self.args[2]),
            HyperCallCode::HWrite => self.write(self.args[0], self.args[1], self.args[2]),
            HyperCallCode::HMMAP => {
                self.mmap(self.args[0], self.args[1], self.args[2], self.args[3])
            }
            _ => {
                unimplemented!();
            }
        }
    }
}

impl InstanceCall {
    fn debug(&self) -> HyperCallResult {
        info!("HDebug {:#x?}", self.args);
        Ok(HyperCallCode::HDebug as usize)
    }

    /// Exit the instance with the given exit code.
    /// TODO: we may need to care about more context states.
    fn exit_instance(&self, exit_code: u64) -> HyperCallResult {
        info!("HExitInstance code {exit_code:#x}");

        crate::libos::instance::remove_instance(self.instance.id())?;
        // DO NOT exit thread here, just mark the percpu as idle.
        // The thread will be exited in the next loop in `libos_vcpu_run`,
        // to let current `InstanceCall` to be dropped peacefully.
        crate::libos::percpu::mark_idle();

        Ok(0)
    }

    fn clone(&self) -> HyperCallResult {
        info!("HClone");
        Ok(0)
    }

    fn mmap(&self, addr: u64, len: u64, prot: u64, flags: u64) -> HyperCallResult {
        info!(
            "HMMAP addr:{:#x} len:{} prot:{} flags:{}",
            addr, len, prot, flags
        );
        Ok(0)
    }
}

impl InstanceCall {
    fn read(&self, fd: u64, buffer_gva: u64, len: u64) -> HyperCallResult {
        info!(
            "HRead fd:{} buffer_gva:{:#x} len {:#x}",
            fd, buffer_gva, len
        );

        Ok(0)
    }

    fn write(&self, fd: u64, buffer_gva: u64, len: u64) -> HyperCallResult {
        info!(
            "HWrite fd:{} buffer_gva:{:#x} len {:#x}",
            fd, buffer_gva, len
        );
        let buffer = current_instance().read_from_guest(
            current_eptp(),
            GuestVirtAddr::from_usize(buffer_gva as usize),
            len as usize,
        )?;

        info!(
            "==== Instance {} Process {} writing begin ====",
            self.instance.id(),
            self.process_id
        );

        axhal::console::write_bytes(buffer.as_slice());

        info!(
            "xxxx Instance {} Process {} writing end xxxx",
            self.instance.id(),
            self.process_id
        );

        Ok(len as usize)
    }
}
