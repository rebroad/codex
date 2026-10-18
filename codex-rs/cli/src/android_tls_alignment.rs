//! Android requires executable TLS segments to be aligned to 64 bytes.

#[repr(align(64))]
#[allow(dead_code)]
struct AndroidTlsAlignment([u8; 64]);

#[used]
#[unsafe(link_section = ".tdata")]
static ANDROID_TLS_ALIGNMENT: AndroidTlsAlignment = AndroidTlsAlignment([0; 64]);
