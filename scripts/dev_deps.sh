#!/bin/bash

# Create directories in the parent directory
mkdir -p ../crates ../deps

# Clone repositories into the crates directory
cd ../crates || exit
git clone git@github.com:EquationOS/arceos.git --branch vmm_type15_hw
git clone git@github.com:arceos-hypervisor/axaddrspace.git --branch type15
git clone git@github.com:arceos-hypervisor/axhvc.git
git clone git@github.com:arceos-hypervisor/axvm.git --branch type15_hw
git clone git@github.com:arceos-hypervisor/axvcpu.git --branch type15
git clone git@github.com:EquationOS/bitmaps.git
git clone git@github.com:EquationOS/equation_defs.git
git clone git@github.com:arceos-org/memory_addr.git --branch wip
git clone git@github.com:arceos-org/page_table_multiarch.git --branch alloc_frames
git clone git@github.com:arceos-hypervisor/x86_vcpu.git --branch equation

# Clone repository into the deps directory
cd ../deps || exit
git clone git@github.com:arceos-hypervisor/axvisor-tools.git --branch axvisor
git clone git@github.com:EquationOS/shim.git
git clone git@github.com:EquationOS/jailhouse-equation.git --branch axvisor_hw
