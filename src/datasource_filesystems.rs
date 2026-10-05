use crate::config::AppConfig;
use crate::metric_support::RegisterMetricResultExt;
use crate::metric_support::prometheus_u64;
use prometheus::GaugeVec;
use std::collections::HashSet;
use std::ffi::CString;
use std::sync::OnceLock;

struct FilesystemMetrics {
    filesystem_size_bytes: GaugeVec,
    filesystem_free_bytes: GaugeVec,
    filesystem_avail_bytes: GaugeVec,
    filesystem_used_bytes: GaugeVec,
    filesystem_files: GaugeVec,
    filesystem_files_free: GaugeVec,
    filesystem_files_used: GaugeVec,
}

impl FilesystemMetrics {
    fn new() -> Self {
        Self {
            filesystem_size_bytes: prometheus::register_gauge_vec!(
                "filesystem_size_bytes",
                "Total filesystem size in bytes",
                &["mountpoint", "device", "fstype"]
            )
            .or_exit("filesystem_size_bytes"),
            filesystem_free_bytes: prometheus::register_gauge_vec!(
                "filesystem_free_bytes",
                "Free filesystem space in bytes",
                &["mountpoint", "device", "fstype"]
            )
            .or_exit("filesystem_free_bytes"),
            filesystem_avail_bytes: prometheus::register_gauge_vec!(
                "filesystem_avail_bytes",
                "Available filesystem space in bytes",
                &["mountpoint", "device", "fstype"]
            )
            .or_exit("filesystem_avail_bytes"),
            filesystem_used_bytes: prometheus::register_gauge_vec!(
                "filesystem_used_bytes",
                "Used filesystem space in bytes",
                &["mountpoint", "device", "fstype"]
            )
            .or_exit("filesystem_used_bytes"),
            filesystem_files: prometheus::register_gauge_vec!(
                "filesystem_files",
                "Total inode count",
                &["mountpoint", "device", "fstype"]
            )
            .or_exit("filesystem_files"),
            filesystem_files_free: prometheus::register_gauge_vec!(
                "filesystem_files_free",
                "Free inode count",
                &["mountpoint", "device", "fstype"]
            )
            .or_exit("filesystem_files_free"),
            filesystem_files_used: prometheus::register_gauge_vec!(
                "filesystem_files_used",
                "Used inode count",
                &["mountpoint", "device", "fstype"]
            )
            .or_exit("filesystem_files_used"),
        }
    }
}

static FILESYSTEM_METRICS: OnceLock<FilesystemMetrics> = OnceLock::new();

fn metrics() -> &'static FilesystemMetrics {
    FILESYSTEM_METRICS.get_or_init(FilesystemMetrics::new)
}

fn pseudo_filesystems() -> &'static HashSet<&'static str> {
    static PSEUDO: OnceLock<HashSet<&'static str>> = OnceLock::new();
    PSEUDO.get_or_init(|| {
        [
            "proc",
            "sysfs",
            "devtmpfs",
            "devpts",
            "tmpfs",
            "cgroup",
            "cgroup2",
            "pstore",
            "securityfs",
            "debugfs",
            "tracefs",
            "configfs",
            "fusectl",
            "mqueue",
            "hugetlbfs",
            "rpc_pipefs",
            "bpf",
            "efivarfs",
            "overlay",
            "autofs",
            "binfmt_misc",
            "nsfs",
            "fuse.portal",
            "portal",
        ]
        .into_iter()
        .collect()
    })
}

fn is_pseudo_fs(fstype: &str) -> bool {
    pseudo_filesystems().contains(fstype)
}

fn reset_metrics(metrics: &FilesystemMetrics) {
    metrics.filesystem_size_bytes.reset();
    metrics.filesystem_free_bytes.reset();
    metrics.filesystem_avail_bytes.reset();
    metrics.filesystem_used_bytes.reset();
    metrics.filesystem_files.reset();
    metrics.filesystem_files_free.reset();
    metrics.filesystem_files_used.reset();
}

pub fn update_metrics(config: &AppConfig) {
    let mounts = match procfs::mounts() {
        Ok(mounts) => mounts,
        Err(_) => return,
    };

    let metrics = metrics();

    // Rebuild from scratch so unmounted filesystems stop being reported. The
    // previous per-mount removal only covered mounts that were filtered out,
    // not mounts that had gone away since the last scrape.
    reset_metrics(metrics);

    for mount in mounts {
        let labels = [
            mount.fs_file.as_str(),
            mount.fs_spec.as_str(),
            mount.fs_vfstype.as_str(),
        ];
        if is_pseudo_fs(&mount.fs_vfstype)
            || (config.ignore_ramfs_filesystems && mount.fs_vfstype == "ramfs")
        {
            continue;
        }
        if config.ignore_loop_devices
            && (mount.fs_spec.starts_with("/dev/loop") || mount.fs_spec == "loop")
        {
            continue;
        }

        let mount_cstring = match CString::new(mount.fs_file.as_bytes()) {
            Ok(value) => value,
            Err(_) => continue,
        };

        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        let rc = unsafe { libc::statvfs(mount_cstring.as_ptr(), &raw mut stat) };
        if rc != 0 {
            continue;
        }

        let block_size = if stat.f_frsize > 0 {
            stat.f_frsize as u64
        } else {
            stat.f_bsize as u64
        };

        let total_bytes = stat.f_blocks as u64 * block_size;
        let free_bytes = stat.f_bfree as u64 * block_size;
        let avail_bytes = stat.f_bavail as u64 * block_size;
        let used_bytes = total_bytes.saturating_sub(free_bytes);

        let files_total = stat.f_files as u64;
        let files_free = stat.f_ffree as u64;
        let files_used = files_total.saturating_sub(files_free);

        metrics
            .filesystem_size_bytes
            .with_label_values(&labels)
            .set(prometheus_u64(total_bytes));
        metrics
            .filesystem_free_bytes
            .with_label_values(&labels)
            .set(prometheus_u64(free_bytes));
        metrics
            .filesystem_avail_bytes
            .with_label_values(&labels)
            .set(prometheus_u64(avail_bytes));
        metrics
            .filesystem_used_bytes
            .with_label_values(&labels)
            .set(prometheus_u64(used_bytes));
        metrics
            .filesystem_files
            .with_label_values(&labels)
            .set(prometheus_u64(files_total));
        metrics
            .filesystem_files_free
            .with_label_values(&labels)
            .set(prometheus_u64(files_free));
        metrics
            .filesystem_files_used
            .with_label_values(&labels)
            .set(prometheus_u64(files_used));
    }
}
