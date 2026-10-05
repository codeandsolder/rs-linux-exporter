# Exported Metrics

This file lists all metrics currently exported by the service, grouped by collection source.

## Automated parsing contract (important for future edits)

If you add or edit sections for this file, keep these formatting rules or the
`scripts/generate_metrics_schema.py` parser will miss data:

- Metric groups must remain as `## <Group Name>` sections.
- Group metric tables must keep the exact table form:
  - header: `| Metric | Type | Description |`
  - rows only for real metrics in that group
  - metric names in backticks (`` `metric_name` ``)
- Add/maintain labels and fields under `## Metric labels and field catalogs`.
- Keep a section like:
  - ``### <metric> label values (`label_name`):`` with a markdown bullet list
  - ``### <metric> field values ...:`` with a markdown bullet list
- If a metric uses fixed labels, include ``### <metric> labels: `label1`, `label2`...`` (or
  equivalent bullet style already used in this file).
- For `netstat`/`snmp`/`vmstat`/`meminfo` style families, keep field lists
  complete, and keep `--with-runtime-fields` regeneration if values are host-
  dependent.
- Avoid free-form prose inside metric tables and catalog lists unless it is still valid
  markdown list/table content.

## Core

| Metric | Type | Description |
|---|---|---|
| `metrics_requests_total` | Counter | Total number of metrics endpoint requests |
| `metrics_requests_denied_total` | Counter | Total number of metrics endpoint requests denied by authentication or ACL |
| `node_scrape_collector_duration_seconds` | GaugeVec | node_exporter-compatible duration of each enabled collector scrape |
| `node_scrape_collector_success` | GaugeVec | node_exporter-compatible success state for each enabled collector (1 = success) |

## cgroup

| Metric | Type | Description |
|---|---|---|
| `cgroup_units` | Gauge | Number of discovered systemd service cgroups |
| `cgroup_cpu_seconds_total` | CounterVec | Cumulative cgroup v2 CPU time by service and kind |
| `cgroup_cpu_periods_total` | CounterVec | Cumulative cgroup v2 CPU period/throttling counters |
| `cgroup_memory_bytes` | GaugeVec | Current/peak memory and swap usage by cgroup v2 service |
| `cgroup_memory_stat_bytes` | GaugeVec | Selected byte-valued cgroup v2 memory.stat fields (when `cgroup_detailed_metrics = true`) |
| `cgroup_memory_events_total` | CounterVec | Cumulative cgroup v2 memory event counters |
| `cgroup_pids_current` | GaugeVec | Current process/thread count in each cgroup v2 service |
| `cgroup_state` | GaugeVec | cgroup v2 populated/frozen state flags |
| `cgroup_io_bytes_total` | CounterVec | Cumulative cgroup v2 I/O bytes by device and operation |
| `cgroup_io_operations_total` | CounterVec | Cumulative cgroup v2 I/O operations by device and operation |
| `cgroup_pressure_seconds_total` | CounterVec | Cumulative per-service PSI stall time |
| `cgroup_pressure_stall_ratio` | GaugeVec | Per-service PSI stall fraction over 10/60/300 second windows (when `cgroup_detailed_metrics = true`) |

## sccache

| Metric | Type | Description |
|---|---|---|
| `sccache_info` | GaugeVec | sccache version information |
| `sccache_requests_total` | CounterVec | Cumulative sccache request counters by result |
| `sccache_cache_requests_total` | CounterVec | Cumulative cache hit/miss/error counters by language |
| `sccache_cache_events_total` | CounterVec | Cumulative cache timeout/read/write/non-cacheable/recache events |
| `sccache_compilations_total` | CounterVec | Cumulative performed and failed compilation counters |
| `sccache_duration_seconds_total` | CounterVec | Cumulative cache read/write and compiler execution time |
| `sccache_not_cached_total` | CounterVec | Cumulative non-cacheable compilation reasons reported by sccache |
| `sccache_not_cached_crate_types_total` | CounterVec | Rust crate types behind `crate-type` non-cacheable results |
| `sccache_dist_compiles_total` | CounterVec | Cumulative successful distributed compilations by worker |
| `sccache_dist_events_total` | CounterVec | Cumulative distributed-compilation events |
| `sccache_cache_level_operations_total` | CounterVec | Cumulative multi-level cache operations by cache level |
| `sccache_cache_level_duration_seconds_total` | CounterVec | Cumulative multi-level cache operation time by cache level |
| `sccache_cache_size_bytes` | GaugeVec | Current/max cache size when reported by the backend |
| `sccache_preprocessor_cache_mode` | Gauge | Whether preprocessor-cache mode is enabled |
| `sccache_basedirs` | Gauge | Number of configured basedirs (paths are intentionally not exported) |
| `sccache_cache_levels` | Gauge | Number of configured multi-level cache levels |
| `sccache_dist_status` | GaugeVec | Optional instantaneous distributed scheduler/client status |

## procfs

| Metric | Type | Description |
|---|---|---|
| `uptime_seconds` | Gauge | System uptime in seconds |
| `uptime_idle_seconds` | Gauge | Sum of idle time across all CPUs in seconds |
| `load_average` | GaugeVec | System load averages |
| `load_processes` | GaugeVec | Runnable and total scheduling entities from /proc/loadavg |
| `cpu_seconds_total` | GaugeVec | CPU time spent in seconds |
| `cpu_context_switches_total` | Gauge | Number of context switches since boot |
| `cpu_boot_time_seconds` | Gauge | Boot time, in seconds since the epoch |
| `processes_forked_total` | Gauge | Number of forks since boot |
| `processes_running` | Gauge | Number of processes currently runnable |
| `processes_blocked` | Gauge | Number of processes blocked waiting for I/O |
| `meminfo` | GaugeVec | Raw values from /proc/meminfo (bytes unless otherwise noted) |
| `vmstat` | GaugeVec | Raw values from /proc/vmstat |
| `diskstats` | GaugeVec | Raw disk statistics from /proc/diskstats |
| `netdev` | GaugeVec | Raw network device stats from /proc/net/dev |
| `tcp_sockets` | GaugeVec | TCP socket counts by state from /proc/net/tcp |
| `udp_sockets` | GaugeVec | UDP socket counts by state from /proc/net/udp |
| `arp_entries` | GaugeVec | ARP table entries by device from /proc/net/arp |
| `snmp` | GaugeVec | SNMP counters from /proc/net/snmp |
| `netstat` | GaugeVec | Extended netstat counters from /proc/net/netstat |

## cpufreq

| Metric | Type | Description |
|---|---|---|
| `cpu_frequency_hz` | GaugeVec | Current CPU frequency per core |

## conntrack

| Metric | Type | Description |
|---|---|---|
| `conntrack` | GaugeVec | Per-CPU conntrack counters via netlink |
| `conntrack` labels | `cpu`, `field` | `field` contains per-CPU counters such as `found`, `invalid`, `insert`, `insert_failed`, `drop`, `early_drop`, `error`, `search_restart`, `clash_resolve`, `chain_toolong` |

## edac

| Metric | Type | Description |
|---|---|---|
| `edac_mc_info` | GaugeVec | Memory controller information |
| `edac_mc_correctable_errors_total` | GaugeVec | Total correctable memory errors on this controller |
| `edac_mc_uncorrectable_errors_total` | GaugeVec | Total uncorrectable memory errors on this controller |
| `edac_mc_correctable_errors_noinfo_total` | GaugeVec | Correctable errors without DIMM slot info |
| `edac_mc_uncorrectable_errors_noinfo_total` | GaugeVec | Uncorrectable errors without DIMM slot info |
| `edac_mc_size_mb` | GaugeVec | Total memory managed by this controller in MB |
| `edac_mc_seconds_since_reset` | GaugeVec | Seconds since error counters were reset |
| `edac_dimm_correctable_errors_total` | GaugeVec | Correctable errors on this DIMM |
| `edac_dimm_uncorrectable_errors_total` | GaugeVec | Uncorrectable errors on this DIMM |
| `edac_dimm_size_mb` | GaugeVec | DIMM size in MB |

## filesystems

| Metric | Type | Description |
|---|---|---|
| `filesystem_size_bytes` | GaugeVec | Total filesystem size in bytes |
| `filesystem_free_bytes` | GaugeVec | Free filesystem space in bytes |
| `filesystem_avail_bytes` | GaugeVec | Available filesystem space in bytes |
| `filesystem_used_bytes` | GaugeVec | Used filesystem space in bytes |
| `filesystem_files` | GaugeVec | Total inode count |
| `filesystem_files_free` | GaugeVec | Free inode count |
| `filesystem_files_used` | GaugeVec | Used inode count |

## hwmon

| Metric | Type | Description |
|---|---|---|
| `hwmon_temperature_celsius` | GaugeVec | Hardware monitor temperature sensor reading in Celsius |
| `hwmon_fan_rpm` | GaugeVec | Hardware monitor fan speed in RPM |
| `hwmon_voltage_volts` | GaugeVec | Hardware monitor voltage reading in Volts |
| `hwmon_power_watts` | GaugeVec | Hardware monitor power reading in Watts |
| `hwmon_current_amps` | GaugeVec | Hardware monitor current reading in Amps |

## ipmi

| Metric | Type | Description |
|---|---|---|
| `ipmi_sensor_reading` | GaugeVec | IPMI sensor reading (unit label indicates base units) |

## mdraid

| Metric | Type | Description |
|---|---|---|
| `mdraid_array_state` | GaugeVec | MD RAID array state (1 for current state label) |
| `mdraid_array_disks` | GaugeVec | MD RAID array disk counts by role |
| `mdraid_array_degraded` | GaugeVec | MD RAID array degraded state (1 if degraded) |
| `mdraid_array_sync_progress` | GaugeVec | MD RAID array sync action progress (0-1) |

## netdev_sysfs

| Metric | Type | Description |
|---|---|---|
| `netdev_operstate` | GaugeVec | Network interface operational state (1 for current state) |
| `netdev_carrier` | GaugeVec | Network interface carrier status (1 = link detected) |
| `netdev_carrier_changes` | GaugeVec | Network interface carrier change count |
| `netdev_dormant` | GaugeVec | Network interface dormant flag (1 = dormant) |
| `netdev_speed_mbps` | GaugeVec | Network interface speed in Mbps |
| `netdev_duplex` | GaugeVec | Network interface duplex (1 for current duplex) |
| `netdev_autoneg` | GaugeVec | Network interface autonegotiation (1 for current state) |

## numa

| Metric | Type | Description |
|---|---|---|
| `numa_node_count` | Gauge | Number of NUMA nodes |
| `numa_node_memory_bytes` | GaugeVec | NUMA node memory information in bytes |
| `numa_node_stat_pages` | GaugeVec | NUMA node hit/miss statistics in pages |

## nvme

| Metric | Type | Description |
|---|---|---|
| `nvme_info` | GaugeVec | NVMe device information |
| `nvme_state` | GaugeVec | NVMe device state (1 = active for given state) |

## power_supply

| Metric | Type | Description |
|---|---|---|
| `power_supply_info` | GaugeVec | Power supply information |
| `power_supply_online` | GaugeVec | Power supply online status (1 = online, 0 = offline) |
| `power_supply_status` | GaugeVec | Battery status (1 = active for given state) |
| `power_supply_capacity_percent` | GaugeVec | Battery capacity in percent |
| `power_supply_voltage_volts` | GaugeVec | Power supply voltage in Volts |
| `power_supply_current_amps` | GaugeVec | Power supply current in Amps |
| `power_supply_power_watts` | GaugeVec | Power supply power in Watts |
| `power_supply_energy_wh` | GaugeVec | Battery energy in Watt-hours |
| `power_supply_charge_ah` | GaugeVec | Battery charge in Amp-hours |
| `power_supply_temperature_celsius` | GaugeVec | Power supply temperature in Celsius |

## pressure

| Metric | Type | Description |
|---|---|---|
| `pressure_cpu_waiting_seconds_total` | Counter | Total CPU PSI some-stall time in seconds |
| `pressure_io_waiting_seconds_total` | Counter | Total I/O PSI some-stall time in seconds |
| `pressure_io_stalled_seconds_total` | Counter | Total I/O PSI full-stall time in seconds |
| `pressure_memory_waiting_seconds_total` | Counter | Total memory PSI some-stall time in seconds |
| `pressure_memory_stalled_seconds_total` | Counter | Total memory PSI full-stall time in seconds |
| `pressure_stall_ratio` | GaugeVec | Fraction of wall time stalled over 10/60/300 second PSI windows |

## rapl

| Metric | Type | Description |
|---|---|---|
| `rapl_energy_joules` | GaugeVec | Current energy counter in Joules (wraps at max_energy_joules) |
| `rapl_max_energy_joules` | GaugeVec | Maximum energy counter range in Joules before wrap |

## zfs

| Metric | Type | Description |
|---|---|---|
| `zfs_arc_size_bytes` | GaugeVec | ZFS ARC current/target/min/max/compressed/uncompressed/L2 sizes |
| `zfs_arc_memory_available_bytes` | Gauge | Memory available to the ARC according to OpenZFS (can be negative) |
| `zfs_arc_accesses_total` | CounterVec | Cumulative ARC/L2ARC hit/miss access counters |
| `zfs_arc_hit_ratio` | GaugeVec | ARC or L2ARC hit ratio since boot |

## softnet

| Metric | Type | Description |
|---|---|---|
| `softnet` | GaugeVec | Per-CPU counters from /proc/net/softnet_stat |

## thermal

| Metric | Type | Description |
|---|---|---|
| `thermal_zone_temperature_celsius` | GaugeVec | Current temperature of the thermal zone in Celsius |
| `thermal_zone_trip_point_celsius` | GaugeVec | Trip point temperature threshold in Celsius |
| `thermal_cooling_device_cur_state` | GaugeVec | Current cooling state of the device |
| `thermal_cooling_device_max_state` | GaugeVec | Maximum cooling state of the device |
| `thermal_zone_count` | Gauge | Number of thermal zones |
| `thermal_cooling_device_count` | Gauge | Number of cooling devices |

## Schema generation for Python tooling

`METRICS.md` now includes enough structured content for tooling to parse. Use:

```bash
python3 scripts/generate_metrics_schema.py --with-runtime-fields --output METRICS.schema.json --pretty
```

The command writes a machine-readable `METRICS.schema.json` using current `/proc` field discovery for raw metric families (`vmstat`, `snmp`, `netstat`, `meminfo`, etc.).

Grafana panel generation:

```bash
python3 scripts/generate_grafana_panel.py --section procfs --datasource DS_PROMETHEUS --instance-var instance
```

The generator now applies common operational defaults:

- Counter-like metrics (`_total`, known counter families) are converted with `rate(...[5m])` by default.
- Network counter fields that contain `byte`/`octet` are emitted as `rate(...[5m]) * 8` to produce bits/sec.
- Use `--disable-auto-rate` to turn this off.
- Use `--rate-window` to change the default rate interval (for example `--rate-window 1m`).
- If you want to use a literal datasource UID/name (`VictoriaMetrics`), pass `--datasource-literal`.
- If you use Grafana datasource variables, keep passing `--datasource DS_...` and set the datasource variable in Grafana.

Run without section/metric flags to get interactive help and examples.

Generate an importable dashboard JSON:

Most used:

```bash
python3 scripts/generate_grafana_panel.py --all --dashboard --datasource VictoriaMetrics --datasource-literal --instance-var instance --output dashboard.json --pretty
```

Alternative (datasource variable mode):

```bash
python3 scripts/generate_grafana_panel.py --all --dashboard --datasource DS_PROMETHEUS --instance-var instance --output dashboard.json --pretty
```

## Metric labels and field catalogs

### cgroup_cpu_seconds_total labels: `cgroup`, `kind`

### cgroup_cpu_periods_total labels: `cgroup`, `kind`

### cgroup_memory_bytes labels: `cgroup`, `kind`

### cgroup_memory_stat_bytes labels: `cgroup`, `field`

### cgroup_memory_events_total labels: `cgroup`, `event`

### cgroup_pids_current labels: `cgroup`

### cgroup_state labels: `cgroup`, `state`

### cgroup_io_bytes_total labels: `cgroup`, `device`, `operation`

### cgroup_io_operations_total labels: `cgroup`, `device`, `operation`

### cgroup_pressure_seconds_total labels: `cgroup`, `resource`, `scope`

### cgroup_pressure_stall_ratio labels: `cgroup`, `resource`, `scope`, `window`

`cgroup_cpu_seconds_total` label values (`kind`):

- `usage`
- `user`
- `system`
- `nice`
- `force_idle`
- `throttled`
- `burst`

`cgroup_cpu_periods_total` label values (`kind`):

- `periods`
- `throttled`
- `bursts`

`cgroup_memory_bytes` label values (`kind`):

- `current`
- `peak`
- `swap_current`
- `swap_peak`

`cgroup_io_bytes_total` label values (`operation`):

- `read`
- `write`
- `discard`

`cgroup_io_operations_total` label values (`operation`):

- `read`
- `write`
- `discard`

`cgroup_pressure_seconds_total` label values (`resource`):

- `cpu`
- `memory`
- `io`

`cgroup_pressure_seconds_total` label values (`scope`):

- `some`
- `full`

`cgroup_pressure_stall_ratio` label values (`window`):

- `10`
- `60`
- `300`

### sccache_info labels: `version`

### sccache_requests_total labels: `result`

### sccache_cache_requests_total labels: `result`, `language`

### sccache_cache_events_total labels: `event`

### sccache_compilations_total labels: `result`

### sccache_duration_seconds_total labels: `operation`

### sccache_not_cached_total labels: `reason`

### sccache_not_cached_crate_types_total labels: `crate_type`

### sccache_dist_compiles_total labels: `server`

### sccache_dist_events_total labels: `event`

### sccache_cache_level_operations_total labels: `level`, `name`, `operation`

### sccache_cache_level_duration_seconds_total labels: `level`, `name`, `operation`

### sccache_cache_size_bytes labels: `kind`

### sccache_dist_status labels: `kind`

`sccache_requests_total` label values (`result`):

- `compile`
- `unsupported_compiler`
- `not_compile`
- `not_cacheable`
- `executed`

`sccache_cache_requests_total` label values (`result`):

- `error`
- `hit`
- `miss`

`sccache_cache_events_total` label values (`event`):

- `timeout`
- `read_error`
- `non_cacheable_compilation`
- `forced_recache`
- `write_error`
- `write`

`sccache_compilations_total` label values (`result`):

- `performed`
- `failed`

`sccache_duration_seconds_total` label values (`operation`):

- `cache_write`
- `cache_read_hit`
- `compile`

`sccache_dist_events_total` label values (`event`):

- `error`

`sccache_cache_level_operations_total` label values (`operation`):

- `hit`
- `miss`
- `write`
- `write_failure`
- `backfill_from`
- `backfill_to`

`sccache_cache_level_duration_seconds_total` label values (`operation`):

- `hit`
- `write`

`sccache_cache_size_bytes` label values (`kind`):

- `current`
- `max`

`sccache_dist_status` label values (`kind`):

- `enabled`
- `connected`
- `servers`
- `cpus`
- `in_progress`

### pressure_stall_ratio labels: `resource`, `scope`, `window`

### zfs_arc_size_bytes labels: `kind`

### zfs_arc_accesses_total labels: `kind`

### zfs_arc_hit_ratio labels: `cache`

### node_scrape_collector_duration_seconds labels: `collector`

### node_scrape_collector_success labels: `collector`

### procfs (raw metric families)

`load_average` label values:

- `interval`: `1`, `5`, `15`

`load_processes` label values:

- `kind`: `running`, `total`, `latest_pid`

`cpu_seconds_total` label values:

- `mode`: `user`, `nice`, `system`, `idle`, `iowait`, `irq`, `softirq`, `steal`, `guest`, `guest_nice`
- `cpu`: `total`, `cpu0`, `cpu1`, ...

`meminfo` label values (`meminfo` metric `field`):

- `active`
- `active_anon`
- `active_file`
- `anon_hugepages`
- `anon_pages`
- `bounce`
- `buffers`
- `cached`
- `cma_free`
- `cma_total`
- `commit_limit`
- `committed_as`
- `direct_map_1G`
- `direct_map_2M`
- `direct_map_4M`
- `direct_map_4k`
- `dirty`
- `file_huge_pages`
- `file_pmd_mapped`
- `high_free`
- `high_total`
- `hugepages_free`
- `hugepages_rsvd`
- `hugepages_surp`
- `hugepages_total`
- `hugepagesize`
- `hugetlb`
- `inactive`
- `inactive_anon`
- `inactive_file`
- `k_reclaimable`
- `kernel_stack`
- `low_free`
- `low_total`
- `mapped`
- `mem_available`
- `mem_free`
- `mem_total`
- `mlocked`
- `mmap_copy`
- `nfs_unstable`
- `page_tables`
- `per_cpu`
- `quicklists`
- `s_reclaimable`
- `s_unreclaim`
- `shmem`
- `shmem_hugepages`
- `slab`
- `swap_cached`
- `swap_free`
- `swap_total`
- `unevictable`
- `vmalloc_chunk`
- `vmalloc_total`
- `vmalloc_used`
- `writeback`
- `writeback_tmp`
- `z_swap`
- `z_swapped`

`vmstat` label values (`vmstat` metric `field`):

- `nr_free_pages`
- `nr_free_pages_blocks`
- `nr_zone_inactive_anon`
- `nr_zone_active_anon`
- `nr_zone_inactive_file`
- `nr_zone_active_file`
- `nr_zone_unevictable`
- `nr_zone_write_pending`
- `nr_mlock`
- `nr_zspages`
- `nr_free_cma`
- `nr_unaccepted`
- `numa_hit`
- `numa_miss`
- `numa_foreign`
- `numa_interleave`
- `numa_local`
- `numa_other`
- `nr_inactive_anon`
- `nr_active_anon`
- `nr_inactive_file`
- `nr_active_file`
- `nr_unevictable`
- `nr_slab_reclaimable`
- `nr_slab_unreclaimable`
- `nr_isolated_anon`
- `nr_isolated_file`
- `workingset_nodes`
- `workingset_refault_anon`
- `workingset_refault_file`
- `workingset_activate_anon`
- `workingset_activate_file`
- `workingset_restore_anon`
- `workingset_restore_file`
- `workingset_nodereclaim`
- `nr_anon_pages`
- `nr_mapped`
- `nr_file_pages`
- `nr_dirty`
- `nr_writeback`
- `nr_shmem`
- `nr_shmem_hugepages`
- `nr_shmem_pmdmapped`
- `nr_file_hugepages`
- `nr_file_pmdmapped`
- `nr_anon_transparent_hugepages`
- `nr_vmscan_write`
- `nr_vmscan_immediate_reclaim`
- `nr_dirtied`
- `nr_written`
- `nr_throttled_written`
- `nr_kernel_misc_reclaimable`
- `nr_foll_pin_acquired`
- `nr_foll_pin_released`
- `nr_kernel_stack`
- `nr_page_table_pages`
- `nr_sec_page_table_pages`
- `nr_iommu_pages`
- `nr_swapcached`
- `pgpromote_success`
- `pgpromote_candidate`
- `pgdemote_kswapd`
- `pgdemote_direct`
- `pgdemote_khugepaged`
- `pgdemote_proactive`
- `nr_hugetlb`
- `nr_balloon_pages`
- `nr_dirty_threshold`
- `nr_dirty_background_threshold`
- `nr_memmap_pages`
- `nr_memmap_boot_pages`
- `pgpgin`
- `pgpgout`
- `pswpin`
- `pswpout`
- `pgalloc_dma`
- `pgalloc_dma32`
- `pgalloc_normal`
- `pgalloc_movable`
- `pgalloc_device`
- `allocstall_dma`
- `allocstall_dma32`
- `allocstall_normal`
- `allocstall_movable`
- `allocstall_device`
- `pgskip_dma`
- `pgskip_dma32`
- `pgskip_normal`
- `pgskip_movable`
- `pgskip_device`
- `pgfree`
- `pgactivate`
- `pgdeactivate`
- `pglazyfree`
- `pgfault`
- `pgmajfault`
- `pglazyfreed`
- `pgrefill`
- `pgreuse`
- `pgsteal_kswapd`
- `pgsteal_direct`
- `pgsteal_khugepaged`
- `pgsteal_proactive`
- `pgscan_kswapd`
- `pgscan_direct`
- `pgscan_khugepaged`
- `pgscan_proactive`
- `pgscan_direct_throttle`
- `pgscan_anon`
- `pgscan_file`
- `pgsteal_anon`
- `pgsteal_file`
- `zone_reclaim_success`
- `zone_reclaim_failed`
- `pginodesteal`
- `slabs_scanned`
- `kswapd_inodesteal`
- `kswapd_low_wmark_hit_quickly`
- `kswapd_high_wmark_hit_quickly`
- `pageoutrun`
- `pgrotated`
- `drop_pagecache`
- `drop_slab`
- `oom_kill`
- `numa_pte_updates`
- `numa_huge_pte_updates`
- `numa_hint_faults`
- `numa_hint_faults_local`
- `numa_pages_migrated`
- `pgmigrate_success`
- `pgmigrate_fail`
- `thp_migration_success`
- `thp_migration_fail`
- `thp_migration_split`
- `compact_migrate_scanned`
- `compact_free_scanned`
- `compact_isolated`
- `compact_stall`
- `compact_fail`
- `compact_success`
- `compact_daemon_wake`
- `compact_daemon_migrate_scanned`
- `compact_daemon_free_scanned`
- `htlb_buddy_alloc_success`
- `htlb_buddy_alloc_fail`
- `unevictable_pgs_culled`
- `unevictable_pgs_scanned`
- `unevictable_pgs_rescued`
- `unevictable_pgs_mlocked`
- `unevictable_pgs_munlocked`
- `unevictable_pgs_cleared`
- `unevictable_pgs_stranded`
- `thp_fault_alloc`
- `thp_fault_fallback`
- `thp_fault_fallback_charge`
- `thp_collapse_alloc`
- `thp_collapse_alloc_failed`
- `thp_file_alloc`
- `thp_file_fallback`
- `thp_file_fallback_charge`
- `thp_file_mapped`
- `thp_split_page`
- `thp_split_page_failed`
- `thp_deferred_split_page`
- `thp_underused_split_page`
- `thp_split_pmd`
- `thp_scan_exceed_none_pte`
- `thp_scan_exceed_swap_pte`
- `thp_scan_exceed_share_pte`
- `thp_split_pud`
- `thp_zero_page_alloc`
- `thp_zero_page_alloc_failed`
- `thp_swpout`
- `thp_swpout_fallback`
- `balloon_inflate`
- `balloon_deflate`
- `balloon_migrate`
- `swap_ra`
- `swap_ra_hit`
- `swpin_zero`
- `swpout_zero`
- `ksm_swpin_copy`
- `cow_ksm`
- `zswpin`
- `zswpout`
- `zswpwb`
- `direct_map_level2_splits`
- `direct_map_level3_splits`
- `direct_map_level2_collapses`
- `direct_map_level3_collapses`
- `nr_unstable`

`diskstats` field values (`field`):

- `reads`
- `reads_merged`
- `sectors_read`
- `time_reading_ms`
- `writes`
- `writes_merged`
- `sectors_written`
- `time_writing_ms`
- `in_progress`
- `time_in_progress_ms`
- `weighted_time_in_progress_ms`
- `discards`
- `discards_merged`
- `sectors_discarded`
- `time_discarding_ms`
- `flushes`
- `time_flushing_ms`

`netdev` field values (`field`):

- `recv_bytes`
- `recv_packets`
- `recv_errs`
- `recv_drop`
- `recv_fifo`
- `recv_frame`
- `recv_compressed`
- `recv_multicast`
- `sent_bytes`
- `sent_packets`
- `sent_errs`
- `sent_drop`
- `sent_fifo`
- `sent_colls`
- `sent_carrier`
- `sent_compressed`

`tcp_sockets` `state` values:

- `established`
- `syn_sent`
- `syn_recv`
- `fin_wait_1`
- `fin_wait_2`
- `time_wait`
- `close`
- `close_wait`
- `last_ack`
- `listen`
- `closing`
- `new_syn_recv`

`udp_sockets` `state` values:

- `established`
- `close`

`arp_entries` label values:

- `device`: interface name from ARP table (`lo`, `eth0`, etc.)

`snmp` field values (`field`):

- `ip_forwarding`
- `ip_default_ttl`
- `ip_in_receives`
- `ip_in_hdr_errors`
- `ip_in_addr_errors`
- `ip_forw_datagrams`
- `ip_in_unknown_protos`
- `ip_in_discards`
- `ip_in_delivers`
- `ip_out_requests`
- `ip_out_discards`
- `ip_out_no_routes`
- `ip_reasm_timeout`
- `ip_reasm_reqds`
- `ip_reasm_oks`
- `ip_reasm_fails`
- `ip_frag_oks`
- `ip_frag_fails`
- `ip_frag_creates`
- `icmp_in_msgs`
- `icmp_in_errors`
- `icmp_in_csum_errors`
- `icmp_in_dest_unreachs`
- `icmp_in_time_excds`
- `icmp_in_parm_probs`
- `icmp_in_src_quenchs`
- `icmp_in_redirects`
- `icmp_in_echos`
- `icmp_in_echo_reps`
- `icmp_in_timestamps`
- `icmp_in_timestamp_reps`
- `icmp_in_addr_masks`
- `icmp_in_addr_mask_reps`
- `icmp_out_msgs`
- `icmp_out_errors`
- `icmp_out_dest_unreachs`
- `icmp_out_time_excds`
- `icmp_out_parm_probs`
- `icmp_out_src_quenchs`
- `icmp_out_redirects`
- `icmp_out_echos`
- `icmp_out_echo_reps`
- `icmp_out_timestamps`
- `icmp_out_timestamp_reps`
- `icmp_out_addr_masks`
- `icmp_out_addr_mask_reps`
- `tcp_rto_algorithm`
- `tcp_rto_min`
- `tcp_rto_max`
- `tcp_max_conn`
- `tcp_active_opens`
- `tcp_passive_opens`
- `tcp_attempt_fails`
- `tcp_estab_resets`
- `tcp_curr_estab`
- `tcp_in_segs`
- `tcp_out_segs`
- `tcp_retrans_segs`
- `tcp_in_errs`
- `tcp_out_rsts`
- `tcp_in_csum_errors`
- `udp_in_datagrams`
- `udp_no_ports`
- `udp_in_errors`
- `udp_out_datagrams`
- `udp_rcvbuf_errors`
- `udp_sndbuf_errors`
- `udp_in_csum_errors`
- `udp_ignored_multi`
- `udp_lite_in_datagrams`
- `udp_lite_no_ports`
- `udp_lite_in_errors`
- `udp_lite_out_datagrams`
- `udp_lite_rcvbuf_errors`
- `udp_lite_sndbuf_errors`
- `udp_lite_in_csum_errors`
- `udp_lite_ignored_multi`

`netstat` field values (`field`) are generated directly from `/proc/net/netstat` by section + header key using the exporter's CamelCase-to-snake-case normalization. The machine-readable schema refreshes the exact host-visible field catalog with `--with-runtime-fields`.

Common section prefixes include:

- `tcp_ext_*`
- `ip_ext_*`
- `mp_tcp_ext_*`

### Conntrack

`conntrack` label values (`field`):

- `found`
- `invalid`
- `insert`
- `insert_failed`
- `drop`
- `early_drop`
- `error`
- `search_restart`
- `clash_resolve`
- `chain_toolong`

### softnet

`softnet` label values (`field`):

- `softnet_cpu_index`
- `softnet_processed_counter`
- `softnet_dropped_counter`
- `softnet_time_squeeze_counter`
- `softnet_received_rps_counter`
- `softnet_flow_limit_count_counter`
- `softnet_backlog_len_total`
- `softnet_input_qlen`
- `softnet_process_qlen`

### Remaining family labels (already fixed)

`cpu_frequency_hz`: `cpu`, `source`
`load_average`: `interval` (`1`, `5`, `15`)
`load_processes`: `kind` (`running`, `total`, `latest_pid`)
`netdev_operstate`: `interface`, `state`
`netdev_carrier`: `interface`
`netdev_carrier_changes`: `interface`
`netdev_dormant`: `interface`
`netdev_speed_mbps`: `interface`
`netdev_duplex`: `interface`, `duplex`
`netdev_autoneg`: `interface`, `state`
`rapl_energy_joules`: `zone`, `name`
`rapl_max_energy_joules`: `zone`, `name`
`thermal_zone_temperature_celsius`: `zone`, `type`
`thermal_zone_trip_point_celsius`: `zone`, `type`, `trip_point`, `trip_type`
`thermal_cooling_device_cur_state`: `device`, `type`
`thermal_cooling_device_max_state`: `device`, `type`
`hwmon_temperature_celsius`: `chip`, `sensor`
`hwmon_fan_rpm`: `chip`, `sensor`
`hwmon_voltage_volts`: `chip`, `sensor`
`hwmon_power_watts`: `chip`, `sensor`
`hwmon_current_amps`: `chip`, `sensor`
`edac_mc_info`: `controller`, `mc_name`
`edac_mc_correctable_errors_total`: `controller`
`edac_mc_uncorrectable_errors_total`: `controller`
`edac_mc_correctable_errors_noinfo_total`: `controller`
`edac_mc_uncorrectable_errors_noinfo_total`: `controller`
`edac_mc_size_mb`: `controller`
`edac_mc_seconds_since_reset`: `controller`
`edac_dimm_correctable_errors_total`: `controller`, `dimm`, `dimm_label`
`edac_dimm_uncorrectable_errors_total`: `controller`, `dimm`, `dimm_label`
`edac_dimm_size_mb`: `controller`, `dimm`, `dimm_label`
`filesystem_size_bytes`: `mountpoint`, `device`, `fstype`
`filesystem_free_bytes`: `mountpoint`, `device`, `fstype`
`filesystem_avail_bytes`: `mountpoint`, `device`, `fstype`
`filesystem_used_bytes`: `mountpoint`, `device`, `fstype`
`filesystem_files`: `mountpoint`, `device`, `fstype`
`filesystem_files_free`: `mountpoint`, `device`, `fstype`
`filesystem_files_used`: `mountpoint`, `device`, `fstype`
`ipmi_sensor_reading`: `sensor`, `type`, `unit`
`mdraid_array_state`: `array`, `state`, `level`
`mdraid_array_disks`: `array`, `role`
`mdraid_array_degraded`: `array`
`mdraid_array_sync_progress`: `array`, `action`
`numa_node_memory_bytes`: `node`, `type`
`numa_node_stat_pages`: `node`, `type`
`nvme_info`: `device`, `model`, `serial`, `firmware_rev`
`nvme_state`: `device`, `state`
`power_supply_info`: `name`, `type`
`power_supply_online`: `name`, `type`
`power_supply_status`: `name`, `status`
`power_supply_capacity_percent`: `name`
`power_supply_voltage_volts`: `name`, `measurement`
`power_supply_current_amps`: `name`, `measurement`
`power_supply_power_watts`: `name`
`power_supply_energy_wh`: `name`, `measurement`
`power_supply_charge_ah`: `name`, `measurement`
`power_supply_temperature_celsius`: `name`
