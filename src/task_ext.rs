use std::os::arceos::modules::axtask::def_task_ext;

use crate::vmm::{VCpuRef, VMRef};

pub enum TaskExtType {
    /// The task is a VM task.
    VM(VMRef),
    /// The task is a LibOS task.
    LibOS,
}

/// Task extended data for the hypervisor.
pub struct TaskExt {
    /// The VM.
    pub ext: TaskExtType,
    /// The vcpu associated with this task.
    pub vcpu: VCpuRef,
}

impl TaskExt {
    pub const fn new(ext: TaskExtType, vcpu: VCpuRef) -> Self {
        Self { ext, vcpu }
    }
}

def_task_ext!(TaskExt);
