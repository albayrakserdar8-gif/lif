//! Per-process resource usage from proc_pid_rusage (no root needed for
//! processes of the same user).

#[derive(Clone, Copy, Debug, Default)]
pub struct Usage {
    pub user_ns: u64,
    pub sys_ns: u64,
    pub idle_wakeups: u64,
    pub intr_wakeups: u64,
    pub rss: u64,
    pub footprint: u64,
}

// Declared here because libc deprecates its copy in favour of the mach2
// crate, and one struct is not worth a dependency.
#[cfg(target_os = "macos")]
#[repr(C)]
struct Timebase {
    numer: u32,
    denom: u32,
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn mach_timebase_info(info: *mut Timebase) -> libc::c_int;
}

#[cfg(target_os = "macos")]
pub fn usage(pid: i32) -> Option<Usage> {
    // SAFETY: zeroed is a valid bit pattern for these plain C structs, and
    // proc_pid_rusage writes at most sizeof(rusage_info_v2) for flavor V2.
    unsafe {
        let mut ri: libc::rusage_info_v2 = std::mem::zeroed();
        let p = (&mut ri as *mut libc::rusage_info_v2).cast::<libc::rusage_info_t>();
        if libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V2, p) != 0 {
            return None;
        }
        // CPU times are in Mach absolute time units, which are not
        // nanoseconds on Apple Silicon (24 MHz ticks).
        let mut tb = Timebase { numer: 1, denom: 1 };
        mach_timebase_info(&mut tb);
        let ns = |t: u64| (t as u128 * tb.numer as u128 / tb.denom.max(1) as u128) as u64;
        Some(Usage {
            user_ns: ns(ri.ri_user_time),
            sys_ns: ns(ri.ri_system_time),
            idle_wakeups: ri.ri_pkg_idle_wkups,
            intr_wakeups: ri.ri_interrupt_wkups,
            rss: ri.ri_resident_size,
            footprint: ri.ri_phys_footprint,
        })
    }
}

#[cfg(not(target_os = "macos"))]
pub fn usage(_pid: i32) -> Option<Usage> {
    None
}

pub fn to_json(pid: i32, u: Option<Usage>) -> String {
    match u {
        Some(u) => format!(
            "{{\"pid\":{pid},\"ok\":true,\"user_ns\":{},\"sys_ns\":{},\"idle_wakeups\":{},\"intr_wakeups\":{},\"rss\":{},\"footprint\":{}}}",
            u.user_ns, u.sys_ns, u.idle_wakeups, u.intr_wakeups, u.rss, u.footprint
        ),
        None => format!("{{\"pid\":{pid},\"ok\":false}}"),
    }
}
