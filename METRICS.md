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
| `metrics_requests_total` | Counter | Total number of `/metrics` requests |
| `metrics_requests_denied_total` | Counter | Total number of `/metrics` requests denied by ACL |

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

## ethtool

| Metric | Type | Description |
|---|---|---|
| `ethtool_stats` | GaugeVec | Ethernet statistics via ethtool netlink |

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

## rapl

| Metric | Type | Description |
|---|---|---|
| `rapl_energy_joules` | GaugeVec | Current energy counter in Joules (wraps at max_energy_joules) |
| `rapl_max_energy_joules` | GaugeVec | Maximum energy counter range in Joules before wrap |

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

## TODO (documentation gaps)

- `ethtool_stats`: collection is currently disabled in `update_metrics` (`ethtool` module exists, but is not enabled yet).

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

`netstat` field values (`field`) are generated from `/proc/net/netstat` by section + header key.
Common section prefixes:

- `tcp_ext_*` (135 fields)
- `ip_ext_*` (18 fields)
- `mptcp_ext_*` (76 fields)

`tcp_ext_` field values include:

- `tcp_ext_syncookies_sent`
- `tcp_ext_syncookies_recv`
- `tcp_ext_syncookies_failed`
- `tcp_ext_embryonic_rsts`
- `tcp_ext_prune_called`
- `tcp_ext_rcv_pruned`
- `tcp_ext_ofo_pruned`
- `tcp_ext_out_of_window_icmps`
- `tcp_ext_lock_dropped_icmps`
- `tcp_ext_arp_filter`
- `tcp_ext_tw`
- `tcp_ext_twrecycled`
- `tcp_ext_twkilled`
- `tcp_ext_pawsactive`
- `tcp_ext_pawsestab`
- `tcp_ext_beyond_window`
- `tcp_ext_tsecr_rejected`
- `tcp_ext_pawsold_ack`
- `tcp_ext_pawstimewait`
- `tcp_ext_delayed_acks`
- `tcp_ext_delayed_acklocked`
- `tcp_ext_delayed_acklost`
- `tcp_ext_listen_overflows`
- `tcp_ext_listen_drops`
- `tcp_ext_tcphphits`
- `tcp_ext_tcppure_acks`
- `tcp_ext_tcphpacks`
- `tcp_ext_tcpsackrecovery`
- `tcp_ext_tcpsackreneging`
- `tcp_ext_tcpsackreorder`
- `tcp_ext_tcprenoreorder`
- `tcp_ext_tcptsreorder`
- `tcp_ext_tcpfull_undo`
- `tcp_ext_tcpartial_undo`
- `tcp_ext_tcpdsackundo`
- `tcp_ext_tcploss_undo`
- `tcp_ext_tcplost_retransmit`
- `tcp_ext_tcpreno_failures`
- `tcp_ext_tcpsack_failures`
- `tcp_ext_tcploss_failures`
- `tcp_ext_tcpfast_retrans`
- `tcp_ext_tcpslow_start_retrans`
- `tcp_ext_tcptimeouts`
- `tcp_ext_tcploss_probes`
- `tcp_ext_tcploss_probe_recovery`
- `tcp_ext_tcpreno_recovery_fail`
- `tcp_ext_tcpsack_recovery_fail`
- `tcp_ext_tcprcv_collapsed`
- `tcp_ext_tcpbacklog_coalesce`
- `tcp_ext_tcpdsackold_sent`
- `tcp_ext_tcpdsackofo_sent`
- `tcp_ext_tcpdsackrecv`
- `tcp_ext_tcpdsackofo_recv`
- `tcp_ext_tcpabort_on_data`
- `tcp_ext_tcpabort_on_close`
- `tcp_ext_tcpabort_on_memory`
- `tcp_ext_tcpabort_on_timeout`
- `tcp_ext_tcpabort_on_linger`
- `tcp_ext_tcpabort_failed`
- `tcp_ext_tcpmemory_pressures`
- `tcp_ext_tcpmemory_pressures_chrono`
- `tcp_ext_tcpsackdiscard`
- `tcp_ext_tcpdsackignored_old`
- `tcp_ext_tcpdsackignored_no_undo`
- `tcp_ext_tcpspurious_rtos`
- `tcp_ext_tcpmd_5_not_found`
- `tcp_ext_tcpmd_5_unexpected`
- `tcp_ext_tcpmd_5_failure`
- `tcp_ext_tcpsack_shifted`
- `tcp_ext_tcpsack_merged`
- `tcp_ext_tcpsack_shift_fallback`
- `tcp_ext_tcpbacklog_drop`
- `tcp_ext_pfmemalloc_drop`
- `tcp_ext_tcpminttldrop`
- `tcp_ext_tcpdefer_accept_drop`
- `tcp_ext_ip_reverse_path_filter`
- `tcp_ext_tcptime_wait_overflow`
- `tcp_ext_tcpreq_qfull_do_cookies`
- `tcp_ext_tcpreq_qfull_drop`
- `tcp_ext_tcpp_reatfail`
- `tcp_ext_tcprcv_coalesce`
- `tcp_ext_tcpofoqueue`
- `tcp_ext_tcpofodrop`
- `tcp_ext_tcpofo_merge`
- `tcp_ext_tcpchallenge_ack`
- `tcp_ext_tcpsynchallenge`
- `tcp_ext_tcpfast_open_active`
- `tcp_ext_tcpfast_open_active_fail`
- `tcp_ext_tcpfast_open_passive`
- `tcp_ext_tcpfast_open_passive_fail`
- `tcp_ext_tcpfast_open_listen_overflow`
- `tcp_ext_tcpfast_open_cookie_reqd`
- `tcp_ext_tcpfast_open_blackhole`
- `tcp_ext_tcpspurious_rtx_host_queues`
- `tcp_ext_busy_poll_rx_packets`
- `tcp_ext_tcpauto_corking`
- `tcp_ext_tcpfrom_zero_window_adv`
- `tcp_ext_tcpto_zero_window_adv`
- `tcp_ext_tcpwant_zero_window_adv`
- `tcp_ext_tcpsyn_retrans`
- `tcp_ext_tcporig_data_sent`
- `tcp_ext_tcphystart_train_detect`
- `tcp_ext_tcphystart_train_cwnd`
- `tcp_ext_tcphystart_delay_detect`
- `tcp_ext_tcphystart_delay_cwnd`
- `tcp_ext_tcpackskipped_syn_recv`
- `tcp_ext_tcpackskipped_paws`
- `tcp_ext_tcpackskipped_seq`
- `tcp_ext_tcpackskipped_fin_wait_2`
- `tcp_ext_tcpackskipped_time_wait`
- `tcp_ext_tcpackskipped_challenge`
- `tcp_ext_tcpwin_probe`
- `tcp_ext_tcpkeep_alive`
- `tcp_ext_tcpmtupfail`
- `tcp_ext_tcpmtupsuccess`
- `tcp_ext_tcpdelivered`
- `tcp_ext_tcpdelivered_ce`
- `tcp_ext_tcpack_compressed`
- `tcp_ext_tcpzero_window_drop`
- `tcp_ext_tcprcv_qdrop`
- `tcp_ext_tcpwqueue_too_big`
- `tcp_ext_tcpfast_open_passive_alt_key`
- `tcp_ext_tcp_timeout_rehash`
- `tcp_ext_tcp_duplicate_data_rehash`
- `tcp_ext_tcpdsackrecv_segs`
- `tcp_ext_tcpdsackignored_dubious`
- `tcp_ext_tcpmigrate_req_success`
- `tcp_ext_tcpmigrate_req_failure`
- `tcp_ext_tcpplb_rehash`
- `tcp_ext_tcpaorequired`
- `tcp_ext_tcpaobad`
- `tcp_ext_tcpao_key_not_found`
- `tcp_ext_tcpaogood`
- `tcp_ext_tcpaodropped_icmps`

`ip_ext_` field values include:

- `ip_ext_in_no_routes`
- `ip_ext_in_truncated_pkts`
- `ip_ext_in_mcast_pkts`
- `ip_ext_out_mcast_pkts`
- `ip_ext_in_bcast_pkts`
- `ip_ext_out_bcast_pkts`
- `ip_ext_in_octets`
- `ip_ext_out_octets`
- `ip_ext_in_mcast_octets`
- `ip_ext_out_mcast_octets`
- `ip_ext_in_bcast_octets`
- `ip_ext_out_bcast_octets`
- `ip_ext_in_csum_errors`
- `ip_ext_in_no_ectpkts`
- `ip_ext_in_ect_1_pkts`
- `ip_ext_in_ect_0_pkts`
- `ip_ext_in_cepkts`
- `ip_ext_reasm_overlaps`

`mptcp_ext_` field values include:

- `mptcp_ext_mpcapable_synrx`
- `mptcp_ext_mpcapable_syntx`
- `mptcp_ext_mpcapable_synackrx`
- `mptcp_ext_mpcapable_ackrx`
- `mptcp_ext_mpcapable_fallback_ack`
- `mptcp_ext_mpcapable_fallback_synack`
- `mptcp_ext_mpcapable_syntxdrop`
- `mptcp_ext_mpcapable_syntxdisabled`
- `mptcp_ext_mpcapable_endp_attempt`
- `mptcp_ext_mpfallback_token_init`
- `mptcp_ext_mptcpretrans`
- `mptcp_ext_mpjoin_no_token_found`
- `mptcp_ext_mpjoin_syn_rx`
- `mptcp_ext_mpjoin_syn_backup_rx`
- `mptcp_ext_mpjoin_syn_ack_rx`
- `mptcp_ext_mpjoin_syn_ack_backup_rx`
- `mptcp_ext_mpjoin_syn_ack_h_mac_failure`
- `mptcp_ext_mpjoin_ack_rx`
- `mptcp_ext_mpjoin_ack_h_mac_failure`
- `mptcp_ext_mpjoin_rejected`
- `mptcp_ext_mpjoin_syn_tx`
- `mptcp_ext_mpjoin_syn_tx_creat_sk_err`
- `mptcp_ext_mpjoin_syn_tx_bind_err`
- `mptcp_ext_mpjoin_syn_tx_connect_err`
- `mptcp_ext_dssnot_matching`
- `mptcp_ext_dsscorruption_fallback`
- `mptcp_ext_dsscorruption_reset`
- `mptcp_ext_infinite_map_tx`
- `mptcp_ext_infinite_map_rx`
- `mptcp_ext_dssno_match_tcp`
- `mptcp_ext_data_csum_err`
- `mptcp_ext_ofoqueue_tail`
- `mptcp_ext_ofoqueue`
- `mptcp_ext_ofomerge`
- `mptcp_ext_no_dssin_window`
- `mptcp_ext_duplicate_data`
- `mptcp_ext_add_addr`
- `mptcp_ext_add_addr_tx`
- `mptcp_ext_add_addr_tx_drop`
- `mptcp_ext_echo_add`
- `mptcp_ext_echo_add_tx`
- `mptcp_ext_echo_add_tx_drop`
- `mptcp_ext_port_add`
- `mptcp_ext_add_addr_drop`
- `mptcp_ext_mpjoin_port_syn_rx`
- `mptcp_ext_mpjoin_port_syn_ack_rx`
- `mptcp_ext_mpjoin_port_ack_rx`
- `mptcp_ext_mismatch_port_syn_rx`
- `mptcp_ext_mismatch_port_ack_rx`
- `mptcp_ext_rm_addr`
- `mptcp_ext_rm_addr_drop`
- `mptcp_ext_rm_addr_tx`
- `mptcp_ext_rm_addr_tx_drop`
- `mptcp_ext_rm_subflow`
- `mptcp_ext_mpprio_tx`
- `mptcp_ext_mpprio_rx`
- `mptcp_ext_mpfail_tx`
- `mptcp_ext_mpfail_rx`
- `mptcp_ext_mpfastclose_tx`
- `mptcp_ext_mpfastclose_rx`
- `mptcp_ext_mprst_tx`
- `mptcp_ext_mprst_rx`
- `mptcp_ext_rcv_pruned`
- `mptcp_ext_subflow_stale`
- `mptcp_ext_subflow_recover`
- `mptcp_ext_snd_wnd_shared`
- `mptcp_ext_rcv_wnd_shared`
- `mptcp_ext_rcv_wnd_conflict_update`
- `mptcp_ext_rcv_wnd_conflict`
- `mptcp_ext_mpcurr_estab`
- `mptcp_ext_blackhole`
- `mptcp_ext_mpcapable_data_fallback`
- `mptcp_ext_md_5_sig_fallback`
- `mptcp_ext_dss_fallback`
- `mptcp_ext_simult_connect_fallback`
- `mptcp_ext_fallback_failed`

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
