//! Private Linux process supervisor for the E2E test-support crate.
//!
//! The supervisor uses Linux-only process APIs (pidfd, subreaper,
//! process-death signals) and is only ever spawned on Linux hosts.  On
//! other platforms this binary compiles to an inert stub so the
//! workspace builds everywhere the test-support crate is compiled.
#[cfg(target_os = "linux")]
mod implementation {
    include!("../supervisor_impl.rs");
}

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    implementation::main()
}

#[cfg(not(target_os = "linux"))]
fn main() {}
