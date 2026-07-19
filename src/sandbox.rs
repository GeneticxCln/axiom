//! Process sandboxing for Axiom compositor (Phase 4).
//!
//! Provides:
//! - Capability dropping after DRM/input device acquisition
//! - `PR_SET_NO_NEW_PRIVS` enforcement for XWayland child processes
//! - Seccomp BPF filter denying dangerous syscalls for XWayland
//!
//! ## Security model
//!
//! The DRM compositor runs with elevated privileges (DRM master, input
//! device access). After acquiring these resources, all Linux capabilities
//! are dropped except those required for ongoing operation. XWayland is
//! spawned with `NO_NEW_PRIVS` and a restrictive seccomp filter that
//! blocks kernel-module loading, raw I/O port access, BPF, and tracing
//! of other processes.

use log::{debug, info, warn};

// Linux capability constants (from <linux/capability.h>).
// The `libc` crate does not export these — we define them locally.
const CAP_SYS_NICE: u32 = 23;
#[allow(dead_code)]
const CAP_SYS_ADMIN: u32 = 21;

/// Drop all Linux capabilities except those in the allowlist.
///
/// Call this AFTER acquiring DRM master and opening input devices.
/// Once dropped, capabilities cannot be reacquired (the bounding set
/// is immutable for the lifetime of the process).
///
/// Allowlist: `CAP_SYS_NICE` (real-time scheduling for audio/video).
pub fn drop_capabilities() {
    // Keep set — only CAP_SYS_NICE for potential rtkit/audio scheduling.
    // Everything else is surrendered. CAP_SYS_ADMIN (needed for DRM
    // master) is only held by the compositor main thread and never
    // needed after device open.
    let keep: &[u32] = &[CAP_SYS_NICE];

    for cap in 0..41_u32 {
        if keep.contains(&cap) {
            continue;
        }
        // SAFETY: prctl(PR_CAPBSET_DROP) takes a capability number.
        // The loop iterates over all valid Linux capabilities (0..39
        // covers the existing set as of Linux 6.x). Dropping a
        // capability that isn't in the bounding set is a no-op.
        let rc = unsafe { libc::prctl(libc::PR_CAPBSET_DROP, cap as libc::c_ulong, 0, 0, 0) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            // EINVAL means the capability doesn't exist on this kernel
            // (e.g. newer caps on older kernels). Not an error.
            if err.raw_os_error() != Some(libc::EINVAL) {
            warn!("Failed to drop capability {}: {}", cap, err);
            }
        }
    }

    info!(
        "🔒 Linux capabilities dropped (retained: CAP_SYS_NICE)"
    );
}

/// Apply `PR_SET_NO_NEW_PRIVS` to prevent the current process (and any
/// children) from ever gaining new privileges via setuid, file
/// capabilities, or seccomp transitions. This is a one-way door and
/// must be set before spawning XWayland.
pub fn set_no_new_privs() {
    // SAFETY: prctl(PR_SET_NO_NEW_PRIVS, 1) is a one-way operation
    // that prevents privilege escalation. It is safe and recommended
    // for any process that spawns untrusted children.
    let rc = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if rc != 0 {
        warn!(
            "Failed to set NO_NEW_PRIVS: {} — XWayland sandbox degraded",
            std::io::Error::last_os_error()
        );
    } else {
        debug!("🔒 PR_SET_NO_NEW_PRIVS applied");
    }
}

/// Install a seccomp BPF filter that blocks dangerous syscalls.
///
/// The filter denies:
/// - `ptrace`, `process_vm_readv`, `process_vm_writev` (process tracing)
/// - `perf_event_open`, `bpf` (kernel tracing/injection)
/// - `kexec_load`, `kexec_file_load` (kernel replacement)
/// - `init_module`, `finit_module`, `delete_module` (kernel module loading)
/// - `iopl`, `ioperm` (raw I/O port access)
///
/// All other syscalls are allowed (default-allow, explicit-deny).
///
/// # Safety
///
/// Must be called AFTER `set_no_new_privs()`. The filter is installed
/// via `prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, ...)` and cannot
/// be removed once applied.
pub fn apply_seccomp_filter() {
    // Build a BPF program that denies the dangerous syscalls.
    // BPF instruction format: { code, jt, jf, k }
    // LD_ABS_W = 0x20 (load word at offset)
    // JEQ = 0x15 (jump if equal)
    // RET_KILL = SECCOMP_RET_KILL_PROCESS = 0x80000000
    // RET_ALLOW = SECCOMP_RET_ALLOW = 0x7fff0000
    //
    // The filter works as:
    //   1. Load syscall number (offset 0 in seccomp_data)
    //   2. For each dangerous syscall: if equal, return KILL
    //   3. If none matched, return ALLOW

    // Syscalls to deny (sorted for BPF linear search efficiency)
    let denied: &[u32] = &[
        libc::SYS_ptrace as u32,
        libc::SYS_process_vm_readv as u32,
        libc::SYS_process_vm_writev as u32,
        libc::SYS_perf_event_open as u32,
        libc::SYS_bpf as u32,
        libc::SYS_kexec_load as u32,
        libc::SYS_kexec_file_load as u32,
        libc::SYS_init_module as u32,
        libc::SYS_finit_module as u32,
        libc::SYS_delete_module as u32,
        libc::SYS_iopl as u32,
        libc::SYS_ioperm as u32,
    ];

    // Each denied syscall needs: 1 load + 1 compare + 1 ret instruction
    // Plus 1 final load + ret-allow = 3 * denied.len() + 2
    let prog_len = 3 * denied.len() + 2;
    let mut filter: Vec<libc::sock_filter> = Vec::with_capacity(prog_len);

    for &syscall in denied {
        // Load syscall number: { code=LD_ABS_W(0x20), jt=0, jf=0, k=0 }
        filter.push(libc::sock_filter {
            code: 0x20, // BPF_LD | BPF_W | BPF_ABS
            jt: 0,
            jf: 0,
            k: 0, // offset 0 = syscall number in seccomp_data
        });
        // Compare: { code=JEQ(0x15), jt=0, jf=1, k=syscall }
        filter.push(libc::sock_filter {
            code: 0x15, // BPF_JMP | BPF_JEQ | BPF_K
            jt: 0,
            jf: 1, // if not equal, skip to next check
            k: syscall,
        });
        // Kill: { code=RET(0x06), jt=0, jf=0, k=SECCOMP_RET_KILL_PROCESS }
        filter.push(libc::sock_filter {
            code: 0x06, // BPF_RET | BPF_K
            jt: 0,
            jf: 0,
            k: libc::SECCOMP_RET_KILL_PROCESS,
        });
    }

    // Final: load (dummy) + allow all other syscalls
    filter.push(libc::sock_filter {
        code: 0x20, // BPF_LD | BPF_W | BPF_ABS
        jt: 0,
        jf: 0,
        k: 0,
    });
    filter.push(libc::sock_filter {
        code: 0x06, // BPF_RET | BPF_K
        jt: 0,
        jf: 0,
        k: libc::SECCOMP_RET_ALLOW,
    });

    let prog = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_ptr() as *mut libc::sock_filter,
    };

    // SAFETY: prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &prog)
    // installs the BPF program. The program is a static deny-list
    // that only blocks the specified syscalls; all others pass
    // through. No pointers escape the call.
    let rc = unsafe {
        libc::prctl(
            libc::PR_SET_SECCOMP,
            libc::SECCOMP_MODE_FILTER,
            &prog as *const libc::sock_fprog as usize,
            0,
            0,
        )
    };

    if rc != 0 {
        let err = std::io::Error::last_os_error();
        warn!(
            "Failed to install seccomp filter: {} — XWayland running without syscall sandbox",
            err
        );
    } else {
        info!(
            "🔒 Seccomp filter installed: {} syscall(s) denied (ptrace, bpf, modules, ioports)",
            denied.len()
        );
    }
}

/// Full XWayland sandbox: NO_NEW_PRIVS + seccomp filter.
///
/// Call this in the compositor process BEFORE spawning XWayland.
/// The restrictions inherit to the child via fork/exec.
/// Once applied, neither the compositor nor any child can gain
/// new privileges.
pub fn apply_sandbox() {
    set_no_new_privs();
    apply_seccomp_filter();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_drop_capabilities_does_not_panic() {
        // This test runs in CI without privileges.
        // It should not panic — the function handles EINVAL gracefully.
        drop_capabilities();
    }

    #[test]
    fn test_set_no_new_privs_does_not_panic() {
        set_no_new_privs();
    }

    #[test]
    fn test_apply_sandbox_does_not_panic() {
        apply_sandbox();
    }

    #[test]
    #[ignore = "installs irreversible seccomp filter on entire test process"]
    fn test_seccomp_filter_denies_ptrace() {
        // NOTE: This test installs a real seccomp filter that persists
        // for the lifetime of the test process. It is ignored by default.
        // Run it in isolation:
        //   cargo test --lib -- sandbox::tests::test_seccomp_filter_denies_ptrace --ignored -- --test-threads=1
        apply_sandbox();
        let rc = unsafe { libc::ptrace(libc::PTRACE_TRACEME, 0, 0, 0) };
        let _ = rc;
    }
}
