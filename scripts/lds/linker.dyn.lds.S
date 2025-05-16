INCLUDE link.x

SECTIONS {
    .init_array : ALIGN(0x10) {
         __init_array_start = .;
         *(.init_array .init_array.*)
         __init_array_end = .;
     }

     linkme_IRQ : { *(linkme_IRQ) }
     linkm2_IRQ : { *(linkm2_IRQ) }
     linkme_PAGE_FAULT : { *(linkme_PAGE_FAULT) }
     linkm2_PAGE_FAULT : { *(linkm2_PAGE_FAULT) }
     linkme_SYSCALL : { *(linkme_SYSCALL) }
     linkm2_SYSCALL : { *(linkm2_SYSCALL) }
     axns_resource : { *(axns_resource) }
 }
 INSERT AFTER .data