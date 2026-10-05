use crate::collection::CollectionReport;
use crate::config::AppConfig;
use crate::metric_support::RegisterMetricResultExt;
use crate::metric_support::prometheus_u64;
use prometheus::GaugeVec;
use rustix::fs::statvfs;
use std::collections::HashSet;
use std::sync::OnceLock;

struct FilesystemMetrics {
    size_bytes: GaugeVec,
    free_bytes: GaugeVec,
    avail_bytes: GaugeVec,
    used_bytes: GaugeVec,
    files: GaugeVec,
    files_free: GaugeVec,
    files_used: GaugeVec,
}

impl FilesystemMetrics {
    fn new() -> Self {
        Self {
            size_bytes: prometheus::register_gauge_vec!(
                "filesystem_size_bytes",
                "Total filesystem size in bytes",
                &["mountpoint", "device", "fstype"]
            )
            .or_exit("filesystem_size_bytes"),
            free_bytes: prometheus::register_gauge_vec!(
                "filesystem_free_bytes",
                "Free filesystem space in bytes",
                &["mountpoint", "device", "fstype"]
            )
            .or_exit("filesystem_free_bytes"),
            avail_bytes: prometheus::register_gauge_vec!(
                "filesystem_avail_bytes",
                "Available filesystem space in bytes",
                &["mountpoint", "device", "fstype"]
            )
            .or_exit("filesystem_avail_bytes"),
            used_bytes: prometheus::register_gauge_vec!(
                "filesystem_used_bytes",
                "Used filesystem space in bytes",
                &["mountpoint", "device", "fstype"]
            )
            .or_exit("filesystem_used_bytes"),
            files: prometheus::register_gauge_vec!(
                "filesystem_files",
                "Total inode count",
                &["mountpoint", "device", "fstype"]
            )
            .or_exit("filesystem_files"),
            files_free: prometheus::register_gauge_vec!(
                "filesystem_files_free",
                "Free inode count",
                &["mountpoint", "device", "fstype"]
            )
            .or_exit("filesystem_files_free"),
            files_used: prometheus::register_gauge_vec!(
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
    metrics.size_bytes.reset();
    metrics.free_bytes.reset();
    metrics.avail_bytes.reset();
    metrics.used_bytes.reset();
    metrics.files.reset();
    metrics.files_free.reset();
    metrics.files_used.reset();
}

pub fn update_metrics(config: &AppConfig) -> CollectionReport {
    let metrics = metrics();
    // Rebuild from scratch before reading mount state: a failed mount-table
    // read must not preserve a filesystem that may already be gone.
    reset_metrics(metrics);

    let Ok(mounts) = procfs::mounts() else {
        return CollectionReport::error();
    };

    let mut report = CollectionReport::success();

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

        let Ok(stat) = statvfs(mount.fs_file.as_str()) else {
            report.record_error();
            continue;
        };

        let block_size = if stat.f_frsize > 0 {
            stat.f_frsize
        } else {
            stat.f_bsize
        };

        let total_bytes = stat.f_blocks.saturating_mul(block_size);
        let free_bytes = stat.f_bfree.saturating_mul(block_size);
        let avail_bytes = stat.f_bavail.saturating_mul(block_size);
        let used_bytes = total_bytes.saturating_sub(free_bytes);

        let files_total = stat.f_files;
        let files_free = stat.f_ffree;
        let files_used = files_total.saturating_sub(files_free);

        metrics
            .size_bytes
            .with_label_values(&labels)
            .set(prometheus_u64(total_bytes));
        metrics
            .free_bytes
            .with_label_values(&labels)
            .set(prometheus_u64(free_bytes));
        metrics
            .avail_bytes
            .with_label_values(&labels)
            .set(prometheus_u64(avail_bytes));
        metrics
            .used_bytes
            .with_label_values(&labels)
            .set(prometheus_u64(used_bytes));
        metrics
            .files
            .with_label_values(&labels)
            .set(prometheus_u64(files_total));
        metrics
            .files_free
            .with_label_values(&labels)
            .set(prometheus_u64(files_free));
        metrics
            .files_used
            .with_label_values(&labels)
            .set(prometheus_u64(files_used));
    }

    report
}
