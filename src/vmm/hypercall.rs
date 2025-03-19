use crate::vmm::vm_list::get_vm_list;
use axerrno::AxResult;
use axstd::println;
use axvm::config::AxVMConfig;

numeric_enum_macro::numeric_enum! {
    #[repr(usize)]
    #[allow(non_camel_case_types)]
    #[allow(missing_docs)]
    #[derive(Eq, PartialEq, Debug, Copy, Clone)]
    pub enum HyperCallId {
        VM_START= 2,
        VM_SHUTDOWN = 3,
        VM_LiST = 4,
    }
}

impl TryFrom<u64> for HyperCallId {
    type Error = &'static str;

    fn try_from(nr: u64) -> Result<Self, Self::Error> {
        match nr {
            2 => Ok(HyperCallId::VM_START),
            3 => Ok(HyperCallId::VM_SHUTDOWN),
            4 => Ok(HyperCallId::VM_LiST),
            _ => Err("Unsupported hypercall id"),
        }
    }
}

pub fn hypercall(hypercall_id: HyperCallId, args: [u64; 6]) -> AxResult {
    debug!("hypercall: id={:?}, args={:x?}", hypercall_id, args);
    match hypercall_id {
        HyperCallId::VM_START => todo!(),
        HyperCallId::VM_SHUTDOWN => todo!(),
        HyperCallId::VM_LiST => vm_list(),
    }
}

pub fn vm_list() -> AxResult {
    println!("| {:<9} | {:<15} | {:<9} |", "vm_id", "name", "cpus");
    let vm_list = get_vm_list();
    for vm_ref in vm_list {
        let vm_config: &AxVMConfig = vm_ref.get_vm_config();
        println!(
            "| {:<9} | {:<15} | {:<9} |",
            vm_config.id(),
            vm_config.name(),
            vm_config.cpu_num(),
        );
    }
    Ok(())
}
