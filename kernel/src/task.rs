use std::os::arceos::modules::axtask::TaskExt;

/// Task extended data for the hypervisor.
pub struct VCpuTask {}

impl VCpuTask {}

#[extern_trait::extern_trait]
unsafe impl TaskExt for VCpuTask {}
