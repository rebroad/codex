// Android's ARM64 dynamic loader requires PT_TLS alignment of at least 64 bytes.
// Keep this input section alive so every Cargo-linked executable, including test
// harnesses, inherits that alignment.
    .section .tdata.codex_android_tls_alignment,"awT",@progbits
    .p2align 6
    .globl codex_android_tls_alignment
    .hidden codex_android_tls_alignment
    .type codex_android_tls_alignment, %object
    .size codex_android_tls_alignment, 64
codex_android_tls_alignment:
    .zero 64

    .section .note.GNU-stack,"",@progbits
