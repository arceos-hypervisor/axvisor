#!/bin/bash

# Create directories in the parent directory
mkdir -p ../crates ../deps

# Clone repositories into the crates directory
cd ../crates || exit
git clone git@github.com:EquationOS/arceos.git --branch vmm_type15
git clone git@github.com:arceos-hypervisor/axaddrspace.git --branch type15
git clone git@github.com:arceos-hypervisor/axvm.git --branch type15
git clone git@github.com:arceos-hypervisor/axvcpu.git --branch type15
git clone git@github.com:arceos-hypervisor/x86_vcpu.git --branch type15

# Clone repository into the deps directory
cd ../deps || exit
git clone git@github.com:EquationOS/jailhouse-equation.git --branch axvisor