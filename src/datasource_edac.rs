use crate::metric_support::prometheus_u64;
use prometheus::GaugeVec;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

struct EdacMetrics {
    mc_info: GaugeVec,
    mc_ce_count: GaugeVec,
    mc_ue_count: GaugeVec,
    mc_ce_noinfo_count: GaugeVec,
    mc_ue_noinfo_count: GaugeVec,
    mc_size_mb: GaugeVec,
    mc_seconds_since_reset: GaugeVec,
    dimm_ce_count: GaugeVec,
    dimm_ue_count: GaugeVec,
    dimm_size_mb: GaugeVec,
}

impl EdacMetrics {
    fn new() -> Self {
        Self {
            mc_info: prometheus::register_gauge_vec!(
                "edac_mc_info",
                "Memory controller information",
                &["controller", "mc_name"]
            )
            .expect("register edac_mc_info"),

            mc_ce_count: prometheus::register_gauge_vec!(
                "edac_mc_correctable_errors_total",
                "Total correctable memory errors on this controller",
                &["controller"]
            )
            .expect("register edac_mc_correctable_errors_total"),

            mc_ue_count: prometheus::register_gauge_vec!(
                "edac_mc_uncorrectable_errors_total",
                "Total uncorrectable memory errors on this controller",
                &["controller"]
            )
            .expect("register edac_mc_uncorrectable_errors_total"),

            mc_ce_noinfo_count: prometheus::register_gauge_vec!(
                "edac_mc_correctable_errors_noinfo_total",
                "Correctable errors without DIMM slot info",
                &["controller"]
            )
            .expect("register edac_mc_correctable_errors_noinfo_total"),

            mc_ue_noinfo_count: prometheus::register_gauge_vec!(
                "edac_mc_uncorrectable_errors_noinfo_total",
                "Uncorrectable errors without DIMM slot info",
                &["controller"]
            )
            .expect("register edac_mc_uncorrectable_errors_noinfo_total"),

            mc_size_mb: prometheus::register_gauge_vec!(
                "edac_mc_size_mb",
                "Total memory managed by this controller in MB",
                &["controller"]
            )
            .expect("register edac_mc_size_mb"),

            mc_seconds_since_reset: prometheus::register_gauge_vec!(
                "edac_mc_seconds_since_reset",
                "Seconds since error counters were reset",
                &["controller"]
            )
            .expect("register edac_mc_seconds_since_reset"),

            dimm_ce_count: prometheus::register_gauge_vec!(
                "edac_dimm_correctable_errors_total",
                "Correctable errors on this DIMM",
                &["controller", "dimm", "dimm_label"]
            )
            .expect("register edac_dimm_correctable_errors_total"),

            dimm_ue_count: prometheus::register_gauge_vec!(
                "edac_dimm_uncorrectable_errors_total",
                "Uncorrectable errors on this DIMM",
                &["controller", "dimm", "dimm_label"]
            )
            .expect("register edac_dimm_uncorrectable_errors_total"),

            dimm_size_mb: prometheus::register_gauge_vec!(
                "edac_dimm_size_mb",
                "DIMM size in MB",
                &["controller", "dimm", "dimm_label"]
            )
            .expect("register edac_dimm_size_mb"),
        }
    }
}

static EDAC_METRICS: OnceLock<EdacMetrics> = OnceLock::new();

fn metrics() -> &'static EdacMetrics {
    EDAC_METRICS.get_or_init(EdacMetrics::new)
}

fn read_string(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn read_u64(path: &Path) -> Option<u64> {
    read_string(path)?.parse::<u64>().ok()
}

fn update_dimm(mc_path: &Path, mc_name: &str, dimm_name: &str) {
    let dimm_path = mc_path.join(dimm_name);
    let metrics = metrics();

    let dimm_label = read_string(&dimm_path.join("dimm_label")).unwrap_or_default();

    if let Some(ce) = read_u64(&dimm_path.join("dimm_ce_count")) {
        metrics
            .dimm_ce_count
            .with_label_values(&[mc_name, dimm_name, &dimm_label])
            .set(prometheus_u64(ce));
    }

    if let Some(ue) = read_u64(&dimm_path.join("dimm_ue_count")) {
        metrics
            .dimm_ue_count
            .with_label_values(&[mc_name, dimm_name, &dimm_label])
            .set(prometheus_u64(ue));
    }

    if let Some(size) = read_u64(&dimm_path.join("size")) {
        metrics
            .dimm_size_mb
            .with_label_values(&[mc_name, dimm_name, &dimm_label])
            .set(prometheus_u64(size));
    }
}

fn update_memory_controller(mc_path: &Path, mc_name: &str) {
    let metrics = metrics();

    // Read controller name
    let controller_type =
        read_string(&mc_path.join("mc_name")).unwrap_or_else(|| "unknown".to_string());

    metrics
        .mc_info
        .with_label_values(&[mc_name, &controller_type])
        .set(1.0);

    // Read error counters
    if let Some(ce) = read_u64(&mc_path.join("ce_count")) {
        metrics
            .mc_ce_count
            .with_label_values(&[mc_name])
            .set(prometheus_u64(ce));
    }

    if let Some(ue) = read_u64(&mc_path.join("ue_count")) {
        metrics
            .mc_ue_count
            .with_label_values(&[mc_name])
            .set(prometheus_u64(ue));
    }

    if let Some(ce_noinfo) = read_u64(&mc_path.join("ce_noinfo_count")) {
        metrics
            .mc_ce_noinfo_count
            .with_label_values(&[mc_name])
            .set(prometheus_u64(ce_noinfo));
    }

    if let Some(ue_noinfo) = read_u64(&mc_path.join("ue_noinfo_count")) {
        metrics
            .mc_ue_noinfo_count
            .with_label_values(&[mc_name])
            .set(prometheus_u64(ue_noinfo));
    }

    // Read size
    if let Some(size) = read_u64(&mc_path.join("size_mb")) {
        metrics
            .mc_size_mb
            .with_label_values(&[mc_name])
            .set(prometheus_u64(size));
    }

    // Read seconds since reset
    if let Some(seconds) = read_u64(&mc_path.join("seconds_since_reset")) {
        metrics
            .mc_seconds_since_reset
            .with_label_values(&[mc_name])
            .set(prometheus_u64(seconds));
    }

    // Process DIMMs and ranks
    if let Ok(entries) = fs::read_dir(mc_path) {
        for entry in entries.flatten() {
            let name = match entry.file_name().into_string() {
                Ok(name) => name,
                Err(_) => continue,
            };

            if (name.starts_with("dimm") || name.starts_with("rank")) && entry.path().is_dir() {
                update_dimm(mc_path, mc_name, &name);
            }
        }
    }
}

pub fn update_metrics() {
    update_metrics_from_path(Path::new("/sys/devices/system/edac/mc"));
}

fn update_metrics_from_path(base: &Path) {
    let entries = match fs::read_dir(base) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    // dimm_label is part of the label set, so a relabelled or replaced DIMM
    // would otherwise leave its old counters behind permanently.
    let metrics = metrics();
    metrics.mc_info.reset();
    metrics.dimm_ce_count.reset();
    metrics.dimm_ue_count.reset();
    metrics.dimm_size_mb.reset();

    for entry in entries.flatten() {
        let name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => continue,
        };

        // Match mc0, mc1, etc.
        if name.starts_with("mc") && name[2..].chars().all(|c| c.is_ascii_digit()) {
            let path = match fs::canonicalize(entry.path()) {
                Ok(p) => p,
                Err(_) => continue,
            };
            update_memory_controller(&path, &name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_mock_mc(
        dir: &Path,
        name: &str,
        mc_name: &str,
        ce: u64,
        ue: u64,
        size: u64,
    ) -> std::path::PathBuf {
        let mc_dir = dir.join(name);
        fs::create_dir_all(&mc_dir).unwrap();
        fs::write(mc_dir.join("mc_name"), format!("{}\n", mc_name)).unwrap();
        fs::write(mc_dir.join("ce_count"), format!("{}\n", ce)).unwrap();
        fs::write(mc_dir.join("ue_count"), format!("{}\n", ue)).unwrap();
        fs::write(mc_dir.join("size_mb"), format!("{}\n", size)).unwrap();
        fs::write(mc_dir.join("seconds_since_reset"), "3600\n").unwrap();
        mc_dir
    }

    fn create_mock_dimm(mc_dir: &Path, name: &str, label: &str, ce: u64, ue: u64, size: u64) {
        let dimm_dir = mc_dir.join(name);
        fs::create_dir_all(&dimm_dir).unwrap();
        fs::write(dimm_dir.join("dimm_label"), format!("{}\n", label)).unwrap();
        fs::write(dimm_dir.join("dimm_ce_count"), format!("{}\n", ce)).unwrap();
        fs::write(dimm_dir.join("dimm_ue_count"), format!("{}\n", ue)).unwrap();
        fs::write(dimm_dir.join("size"), format!("{}\n", size)).unwrap();
    }

    #[test]
    fn test_read_string_trims_whitespace() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("mc_name");
        fs::write(&file, "  EDAC_MC  \n").unwrap();
        assert_eq!(read_string(&file), Some("EDAC_MC".to_string()));
    }

    #[test]
    fn test_read_u64_parses_integer() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("ce_count");
        fs::write(&file, "42\n").unwrap();
        assert_eq!(read_u64(&file), Some(42));
    }

    #[test]
    fn test_read_u64_handles_invalid() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("ce_count");
        fs::write(&file, "not_a_number\n").unwrap();
        assert_eq!(read_u64(&file), None);
    }

    #[test]
    fn test_update_memory_controller() {
        let dir = TempDir::new().unwrap();
        let mc = create_mock_mc(dir.path(), "mc0", "ie31200_edac", 5, 0, 16384);
        update_memory_controller(&mc, "mc0");
    }

    #[test]
    fn test_update_memory_controller_with_dimms() {
        let dir = TempDir::new().unwrap();
        let mc = create_mock_mc(dir.path(), "mc0", "ie31200_edac", 5, 0, 16384);
        create_mock_dimm(&mc, "dimm0", "CPU_DIMM_A1", 2, 0, 8192);
        create_mock_dimm(&mc, "dimm1", "CPU_DIMM_B1", 3, 0, 8192);

        update_memory_controller(&mc, "mc0");
    }

    #[test]
    fn test_update_memory_controller_with_ranks() {
        let dir = TempDir::new().unwrap();
        let mc = create_mock_mc(dir.path(), "mc0", "skx_edac", 0, 0, 32768);
        create_mock_dimm(&mc, "rank0", "DIMM_A1", 0, 0, 16384);
        create_mock_dimm(&mc, "rank1", "DIMM_B1", 0, 0, 16384);

        update_memory_controller(&mc, "mc0");
    }

    #[test]
    fn test_update_metrics_from_path_filters_non_mc() {
        let dir = TempDir::new().unwrap();
        create_mock_mc(dir.path(), "mc0", "ie31200_edac", 0, 0, 8192);
        fs::create_dir_all(dir.path().join("not_mc")).unwrap();

        update_metrics_from_path(dir.path());
    }

    #[test]
    fn test_update_metrics_from_path_handles_empty_dir() {
        let dir = TempDir::new().unwrap();
        update_metrics_from_path(dir.path());
    }

    #[test]
    fn test_update_dimm() {
        let dir = TempDir::new().unwrap();
        let mc = create_mock_mc(dir.path(), "mc0", "test_edac", 0, 0, 8192);
        create_mock_dimm(&mc, "dimm0", "DIMM_A", 10, 1, 4096);

        update_dimm(&mc, "mc0", "dimm0");
    }
}
