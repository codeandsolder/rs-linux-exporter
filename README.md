# rs-linux-exporter
Prometheus-compatible Linux metrics exporter written in Rust.

## Overview
rs-linux-exporter aims to provide the most important Linux host metrics with a
simple, safe, and fast Rust implementation. The goal is to avoid external
dependencies (especially tools and unsafe languages) to keep the binary small
and the software reliable.

## Goals
- Provide essential Linux metrics for Prometheus scraping
- Keep the binary size small and startup time fast
- Avoid external tool dependencies and unsafe language bindings
- Favor correctness, safety, and predictable performance

## Initial focus metrics (implemented)
- System uptime
- CPU usage and load averages
- Memory usage (total/free/available/buffers/cache)
- Disk usage (total/used/free per filesystem)
- Disk I/O (bytes, ops, time per device)
- Network I/O (bytes, packets, errors per interface)
- Process count and basic system limits

### Available Datasources

| Datasource | Description |
|------------|-------------|
| `procfs` | System stats from /proc (CPU, memory, network, disk I/O) |
| `filefd` | System-wide file descriptor table allocation and limit from `/proc/sys/fs/file-nr` |
| `schedstat` | Per-CPU scheduler running/waiting/timeslice counters from `/proc/schedstat` |
| `kernel_hung` | Kernel hung-task detections from `/proc/sys/kernel/hung_task_detect_count` |
| `watchdog` | Linux watchdog device state, timeout, firmware and identity from sysfs |
| `uname` | Kernel/system identity from the `uname(2)` system call |
| `os` | Operating-system identity from `os-release` |
| `lifecycle` | Exporter build version, Linux boot ID, and package-database freshness |
| `dmi` | BIOS, board, chassis and system DMI identity from sysfs |
| `timex` | Kernel NTP discipline, synchronization, error and PPS statistics via read-only `adjtimex(2)` |
| `ntp` | Chrony daemon tracking, source state/reachability, offsets, frequency/skew, and activity via stable CSV output |
| `probe` | Cached background ICMP ping and traceroute probes to explicitly configured targets |
| `tailscale` | Tailnet peer/path state, TSMP/ICMP RTT, public mappings, DERP latency, and optional underlay traceroutes |
| `time` | Current Unix time and Linux kernel clocksource information |
| `nfs` | Global NFS client RPC, network, and procedure counters when the kernel interface is present |
| `nfsd` | Global kernel NFS server RPC, cache, I/O, thread, and procedure counters when active |
| `cgroup` | Bounded cgroup v2 service CPU, memory, I/O, PID, state, and PSI metrics |
| `sccache` | Local sccache cache/compiler/distributed-build statistics via its supported JSON CLI |
| `systemd` | Unit state, restart, timer/socket, system state/version, and optional service runtime metrics over D-Bus |
| `journal` | Incremental critical-kernel/service event classification plus remote-syslog arrival/count telemetry from `udp514-journal` |
| `cpufreq` | CPU frequency per core |
| `softnet` | Network soft interrupt statistics |
| `conntrack` | Connection tracking statistics |
| `filesystems` | Filesystem usage statistics |
| `hwmon` | Hardware sensors from Linux hwmon (temperature, fan, voltage, power, current, frequency, alarms and thresholds) |
| `thermal` | Thermal zones and cooling devices |
| `rapl` | Intel/AMD RAPL energy consumption (CPU, DRAM) |
| `power_supply` | Battery and AC adapter status |
| `pressure` | Linux pressure stall information (PSI) for CPU, memory, and I/O |
| `nvme` | NVMe device information (model, serial, state) |
| `smart` | ATA/NVMe SMART health, temperature, lifetime counters, endurance/spare state, and ATA attributes via bounded `smartctl -j` |
| `edac` | Memory error detection (correctable/uncorrectable) |
| `numa` | NUMA node memory and hit/miss statistics |
| `ipmi` | IPMI sensor readings via /dev/ipmi0 |
| `mdraid` | Linux software RAID (md) array status |
| `netdev_sysfs` | Network interface link state, speed, and duplex from sysfs |
| `zfs` | OpenZFS ARC statistics plus pool/vdev health, capacity, error, I/O and bandwidth state |

## Kernel Modules for Hardware Monitoring

The `hwmon` and `thermal` exporters require appropriate kernel modules to be loaded.
The `ipmi` exporter requires `/dev/ipmi0`, typically provided by `ipmi_devintf` and `ipmi_si`.
Use `sensors-detect` from the `lm-sensors` package to identify which modules your
system needs.

### Common Modules

| Module | Description |
|--------|-------------|
| `coretemp` | Intel CPU temperature sensors |
| `k10temp` | AMD CPU temperature sensors (Family 10h+) |
| `nct6775` | Nuvoton Super I/O chips (common on many motherboards) |
| `it87` | ITE Super I/O chips |
| `drivetemp` | SATA/SAS drive temperatures (kernel 5.6+) |

### Quick Setup

```bash
# Detect and load modules interactively
sudo sensors-detect

# Or load common modules manually
sudo modprobe coretemp    # Intel CPUs
sudo modprobe k10temp     # AMD CPUs
sudo modprobe drivetemp   # Drive temperatures

# Make persistent at boot
echo -e "coretemp\nk10temp\ndrivetemp" | sudo tee /etc/modules-load.d/sensors.conf
```

### Troubleshooting

If sensors are missing:
- Re-run `sensors-detect` and follow its recommendations
- Check loaded modules: `lsmod | grep -E 'coretemp|k10temp|nct|it87|drivetemp'`
- Some motherboard chips (IT8655E, IT8625E, IT8686E) may need out-of-tree drivers
- For additional readings, try boot parameter: `acpi_enforce_resources=lax`

See [lm_sensors ArchWiki](https://wiki.archlinux.org/title/Lm_sensors) for detailed guidance.

## Configuration

Configuration is optional. Create a `config.toml` file in the working directory.

## Debian/Ubuntu Packages

The `.deb` packages include a systemd unit and a default config file.

Targets: Debian 12/13 and Ubuntu 22.04/24.04/26.04.

Defaults when installed from `.deb`:
- Service name: `rs-linux-exporter.service`
- Auto-enabled on install
- Config path: `/etc/rs-linux-exporter/config.toml`
- Working directory: `/etc/rs-linux-exporter`
- Bind address: `127.0.0.1:23311`
- Allowed clients: `127.0.0.0/8`

To change the bind address or allowed clients, edit `/etc/rs-linux-exporter/config.toml` and restart:

```bash
sudo systemctl restart rs-linux-exporter
```

### Example config.toml

```toml
# Ignore loop devices in filesystem metrics
ignore_loop_devices = true

# Ignore ramfs mounts in filesystem metrics (set false to include)
ignore_ramfs_filesystems = true

# Ignore PPP interfaces in network metrics
ignore_ppp_interfaces = true

# Ignore veth and br-* interfaces in network metrics
ignore_veth_interfaces = true

# cgroup v2 roots, relative to /sys/fs/cgroup. Service cgroups (*.service)
# are discovered recursively under these roots; transient scopes are excluded.
cgroup_roots = ["system.slice"]

# Hard cardinality guard for discovered service cgroups.
cgroup_max_units = 256

# Add per-service memory.stat breakdown and PSI 10/60/300 window ratios.
# Baseline mode still exports CPU, memory current/peak/swap, memory events,
# PIDs/state, I/O, and cumulative PSI totals.
cgroup_detailed_metrics = false

# sccache collector. The exporter first verifies the supervised daemon is already
# reachable on loopback, then reads the supported JSON stats CLI. Monitoring never
# starts a missing daemon.
sccache_binary = "sccache"
sccache_server_port = 4226
sccache_timeout_ms = 1000

# Optional instantaneous distributed scheduler status. Disabled by default because
# upstream `sccache --dist-status` can start a daemon if it races with a daemon exit.
sccache_collect_dist_status = false

# systemd collector. Patterns are whole-unit-name regular expressions, matching
# node_exporter semantics. Only loaded units are exported. The default exclusion
# drops high-cardinality/noisy automount, device, mount, scope, and slice units.
systemd_unit_include = ".+"
systemd_unit_exclude = '.+\.(automount|device|mount|scope|slice)'
systemd_max_units = 512

# Add service start-time and TasksCurrent/TasksMax metrics. Unit state, service
# restart counts, timers, sockets, system state/version, and virtualization stay
# enabled in baseline mode.
systemd_detailed_metrics = false

# SMART is intentionally slower than the one-second host collection loop and
# auto-disables when smartctl is absent. Direct ATA access may require
# CAP_SYS_RAWIO under a hardened systemd service.
smartctl_binary = "smartctl"
smart_interval_seconds = 60
smartctl_timeout_ms = 5000

# Incremental journal classification. The remote-syslog metrics are populated
# when udp514-journal is present and forwarding network syslog into journald.
journalctl_binary = "journalctl"
journal_interval_seconds = 10
journal_timeout_ms = 5000

# Chrony/NTP daemon telemetry. The datasource auto-disables if chronyc is absent.
ntp_binary = "chronyc"
ntp_timeout_ms = 1000

# Generic active network probes. Empty by default, so the exporter generates no
# external probe traffic unless targets are explicitly configured. Expensive
# traceroutes refresh in a background worker and never block /metrics.
probe_interval_seconds = 30
probe_timeout_ms = 1000
probe_traceroute_timeout_ms = 10000
probe_traceroute_max_hops = 30
probe_ping_binary = "ping"
probe_mtr_binary = "mtr"

[[probe_targets]]
name = "cloudflare-dns"
address = "1.1.1.1"
ping = true
traceroute = true

# Tailscale telemetry. If tailscale is installed, status/netcheck and online-peer
# TSMP/ICMP probes refresh in the background. External traceroute follows the
# peer's current direct public endpoint when Tailscale exposes one.
tailscale_binary = "tailscale"
tailscale_interval_seconds = 30
tailscale_timeout_ms = 5000
tailscale_ping_timeout_ms = 1000
tailscale_probe_tsmp = true
tailscale_probe_icmp = true
tailscale_external_traceroute = true

# Optional active push. Samples are timestamped when collected and sent as
# gzip-compressed Prometheus text. The HTTP /metrics endpoint remains available
# for compatibility/debugging but is not involved in the push path.
# push_url = "http://127.0.0.1:8428/api/v1/import/prometheus?extra_label=instance=myhost&extra_label=resolution=raw"
# push_interval_ms = 1000
# push_timeout_ms = 1000
# push_spill_after_seconds = 10
# push_spool_dir = "/var/lib/rs-linux-exporter/spool"
# push_spool_max_bytes = 10485760

# Optional push-plugin endpoint. Plugins send bounded TTL snapshots over a Unix
# stream socket; scrapes only read the last accepted snapshot and never call the
# plugin synchronously. The socket is created mode 0660.
# plugin_socket = "/run/rs-linux-exporter/plugins.sock"

# Disable specific datasources (will not be polled)
# Available: procfs, filefd, schedstat, kernel_hung, watchdog, uname, os, lifecycle, dmi, timex, time, cgroup, sccache, systemd, journal, ntp, probe, tailscale, cpufreq, softnet, conntrack, filesystems, hwmon, ipmi, mdraid,
# thermal, rapl, power_supply, pressure, nvme, smart, edac, netdev_sysfs, nfs, nfsd, numa, zfs
disabled_datasources = ["thermal", "conntrack"]

# Restrict /metrics access to these IPs/CIDRs (supports single IPs and CIDR notation)
allowed_ip = ["127.0.0.0/8", "10.0.0.0/8", "192.168.1.100"]

# Bind address for the HTTP server
bind = "127.0.0.1:9100"

# Log denied /metrics requests
log_denied_requests = true

# Log 404 requests
log_404_requests = false

# TLS certificate and key paths (both required to enable HTTPS)
# tls_cert = "/etc/rs-linux-exporter/cert.pem"
# tls_key = "/etc/rs-linux-exporter/key.pem"

# Bearer token for authentication (optional)
# auth_token = "your-secret-token-here"
```


### Active push and bounded outage spill

Delivery/retry/spooling is provided by [`lurkmoar`](https://github.com/codeandsolder/lurkmoar-rs); this exporter only owns the collection schedule.

When `push_url` is set, the exporter actively collects and timestamps one complete
metric snapshot every `push_interval_ms`. Successful batches remain RAM-only and
are sent directly to the configured HTTP endpoint. Prometheus metadata comments
are omitted from pushed batches; `/metrics` still exposes the normal full text
format.

A send failure starts an outage timer. Pending timestamped batches stay in RAM
until `push_spill_after_seconds` elapses, then the entire pending group is written
as one spool file. Additional outage data is flushed at the same cadence. On
recovery, spool files are replayed oldest-first before current RAM batches. Files
are removed only after all of their batches receive successful HTTP responses.

The spool is deliberately bounded rather than a database/WAL. If writing the next
chunk would exceed `push_spool_max_bytes`, that pending chunk is dropped and
`metrics_push_dropped_batches_total` is incremented. Normal operation therefore
causes no metric-spool disk writes. `metrics_push_spool_bytes`,
`metrics_push_pending_batches`, `metrics_push_failures_total`, and
`metrics_push_last_success_unixtime` expose sender health.

For VictoriaMetrics/Liberta, use `/api/v1/import/prometheus`; query parameters such
as repeated `extra_label=` values can identify the host and retention tier. A
small server-side dedup interval makes replay after an ambiguous HTTP acknowledgement
idempotent enough without sender-side transactional state.

### Unix-socket plugin snapshots

`plugin_socket` enables a small push-only extension point for application metrics. The exporter owns the Unix stream socket; plugins publish a complete snapshot and receive a framed JSON acknowledgement. A snapshot is a 4-byte big-endian length followed by JSON:

```json
{
  "version": 1,
  "plugin": "searxrs2",
  "ttl_seconds": 15,
  "metrics": [
    {
      "name": "requests_total",
      "help": "Cumulative requests",
      "type": "counter",
      "samples": [
        {"labels": {"engine": "brave"}, "value": 42}
      ]
    }
  ]
}
```

Metric families are exported as `<plugin>_<name>` (for example `searxrs2_requests_total`). Counter, gauge, and untyped snapshots are supported. Messages are capped at 1 MiB, plugin/metric/sample/label counts are bounded, labels must use one stable schema per family, and stale snapshots disappear after their declared TTL. `plugin_snapshot_age_seconds{plugin=...}` and `plugin_snapshot_valid{plugin=...}` remain visible so missing publishers are observable.

This is intentionally not shared-memory IPC: metric snapshots are tiny and infrequent, while Unix sockets give a simpler failure and permission model. Plugins never execute on the Prometheus scrape path.

## Token Authentication

rs-linux-exporter supports optional Bearer token authentication. When configured, all requests to `/metrics` and `/metrics.json` must include a valid `Authorization` header.

### Configuration

Add the `auth_token` option to your config.toml:

```toml
auth_token = "your-secret-token-here"
```

When `auth_token` is set, requests without a valid token receive HTTP 401 Unauthorized. When not set, token authentication is disabled.

### Generating a Secure Token

Generate a cryptographically secure random token:

```bash
# Using openssl (recommended)
openssl rand -base64 32

# Using /dev/urandom
head -c 32 /dev/urandom | base64

# Example output: K7gNU3sdo+OL0wNhqoVWhr3g6s1xYv72ol/pe/Unols=
```

Because `config.toml` then holds a secret, keep it unreadable by other
users. The Debian package installs it mode 0640; if you deploy it yourself:

```bash
chmod 0640 /etc/rs-linux-exporter/config.toml
```

### Testing with curl

```bash
# With token authentication
curl -H "Authorization: Bearer your-secret-token-here" http://localhost:9100/metrics

# Without token (will fail with 401 if auth_token is configured)
curl http://localhost:9100/metrics
```

### Prometheus Configuration

Configure Prometheus to send the Bearer token:

```yaml
scrape_configs:
  - job_name: 'linux-exporter'
    authorization:
      type: Bearer
      credentials: your-secret-token-here
    static_configs:
      - targets: ['hostname:9100']
```

Or use a credentials file for better security:

```yaml
scrape_configs:
  - job_name: 'linux-exporter'
    authorization:
      type: Bearer
      credentials_file: /etc/prometheus/exporter-token.txt
    static_configs:
      - targets: ['hostname:9100']
```

Create the credentials file:

```bash
echo -n "your-secret-token-here" | sudo tee /etc/prometheus/exporter-token.txt
sudo chmod 600 /etc/prometheus/exporter-token.txt
sudo chown prometheus:prometheus /etc/prometheus/exporter-token.txt
```

### Combined with TLS

For production environments, combine token authentication with TLS for encrypted transport:

```toml
tls_cert = "/etc/rs-linux-exporter/cert.pem"
tls_key = "/etc/rs-linux-exporter/key.pem"
auth_token = "your-secret-token-here"
```

Prometheus config for HTTPS with token auth:

```yaml
scrape_configs:
  - job_name: 'linux-exporter'
    scheme: https
    authorization:
      type: Bearer
      credentials_file: /etc/prometheus/exporter-token.txt
    tls_config:
      # For self-signed certificates:
      insecure_skip_verify: true
      # Or with CA verification:
      # ca_file: /path/to/ca.pem
    static_configs:
      - targets: ['hostname:9100']
```

## TLS/HTTPS Support

rs-linux-exporter supports optional TLS encryption. To enable HTTPS, add both `tls_cert` and `tls_key` to your config.toml:

```toml
tls_cert = "/etc/rs-linux-exporter/cert.pem"
tls_key = "/etc/rs-linux-exporter/key.pem"
```

Both options must be specified for TLS to be enabled. If only one is provided, the server runs in HTTP mode.

### Self-Signed Certificates (Testing/Internal Use)

For internal networks or testing, generate a self-signed certificate:

```bash
# Create directory for certificates
sudo mkdir -p /etc/rs-linux-exporter

# Generate self-signed certificate (valid for 365 days)
sudo openssl req -x509 -newkey rsa:4096 -nodes \
    -keyout /etc/rs-linux-exporter/key.pem \
    -out /etc/rs-linux-exporter/cert.pem \
    -days 365 \
    -subj "/CN=$(hostname)"

# Set appropriate permissions
sudo chmod 600 /etc/rs-linux-exporter/key.pem
sudo chmod 644 /etc/rs-linux-exporter/cert.pem
```

For certificates valid for multiple hostnames or IPs:

```bash
sudo openssl req -x509 -newkey rsa:4096 -nodes \
    -keyout /etc/rs-linux-exporter/key.pem \
    -out /etc/rs-linux-exporter/cert.pem \
    -days 365 \
    -subj "/CN=$(hostname)" \
    -addext "subjectAltName=DNS:$(hostname),DNS:localhost,IP:127.0.0.1"
```

### Let's Encrypt with Certbot

For production environments with a public domain, use Let's Encrypt for free trusted certificates.

#### Install Certbot

```bash
# Debian/Ubuntu
sudo apt install certbot

# RHEL/CentOS/Fedora
sudo dnf install certbot

# Arch Linux
sudo pacman -S certbot
```

#### Obtain Certificate

```bash
# Standalone mode (temporarily binds to port 80)
sudo certbot certonly --standalone -d metrics.example.com

# Or use webroot if you have a web server
sudo certbot certonly --webroot -w /var/www/html -d metrics.example.com
```

#### Configure rs-linux-exporter

Certbot stores certificates in `/etc/letsencrypt/live/<domain>/`. Update config.toml:

```toml
tls_cert = "/etc/letsencrypt/live/metrics.example.com/fullchain.pem"
tls_key = "/etc/letsencrypt/live/metrics.example.com/privkey.pem"
```

Note: The exporter process needs read access to the private key. Either run as root, or adjust permissions:

```bash
# Option 1: Add exporter user to ssl-cert group (Debian/Ubuntu)
sudo usermod -aG ssl-cert exporter-user

# Option 2: Use ACLs
sudo setfacl -m u:exporter-user:r /etc/letsencrypt/live/metrics.example.com/privkey.pem
sudo setfacl -m u:exporter-user:rx /etc/letsencrypt/live/metrics.example.com/
sudo setfacl -m u:exporter-user:rx /etc/letsencrypt/archive/metrics.example.com/
```

#### Auto-Renewal

Certbot sets up automatic renewal. To reload the exporter after renewal, create a deploy hook:

```bash
sudo tee /etc/letsencrypt/renewal-hooks/deploy/rs-linux-exporter.sh << 'EOF'
#!/bin/bash
systemctl restart rs-linux-exporter
EOF
sudo chmod +x /etc/letsencrypt/renewal-hooks/deploy/rs-linux-exporter.sh
```

Test renewal with: `sudo certbot renew --dry-run`

### Prometheus Configuration for HTTPS

Update your Prometheus scrape config to use HTTPS:

```yaml
scrape_configs:
  - job_name: 'linux-exporter'
    scheme: https
    # For self-signed certificates:
    tls_config:
      insecure_skip_verify: true
    # Or with CA verification:
    # tls_config:
    #   ca_file: /path/to/ca.pem
    static_configs:
      - targets: ['hostname:9100']
```

## Contributing
Please check dependency freshness and binary size when submitting changes.

To install helper tools:
- `cargo install cargo-outdated`
- `cargo install cargo-bloat`

Recommended checks:
- `cargo outdated`
- `cargo bloat --release`

## Status
This project is a work in progress.

### Active push and bounded outage spill

When `push_url` is set, the exporter actively collects and timestamps one complete metric snapshot every `push_interval_ms`. Successful batches remain RAM-only and are sent directly to the configured HTTP endpoint. The `/metrics` endpoint remains available for compatibility and debugging but is not involved in the push path.

A send failure starts an outage timer. Pending timestamped batches stay in RAM until `push_spill_after_seconds` elapses, then the entire pending group is written as one spool file. Additional outage data is flushed at the same cadence. On recovery, spool files are replayed oldest-first before current RAM batches and are removed only after successful delivery.

The spool is deliberately bounded rather than a database or WAL. If writing the next chunk would exceed `push_spool_max_bytes`, that pending chunk is dropped and `metrics_push_dropped_batches_total` is incremented. Normal operation therefore causes no metric-spool disk writes. Sender health is exposed through `metrics_push_spool_bytes`, `metrics_push_pending_batches`, `metrics_push_failures_total`, and `metrics_push_last_success_unixtime`.

For VictoriaMetrics/Liberta, point `push_url` at `/api/v1/import/prometheus`; repeated `extra_label=` query parameters can identify the host and retention tier. A small server-side dedup interval handles the narrow replay window after an ambiguous HTTP acknowledgement without sender-side transactional state.

### Release-based self-updates

The repository publishes a generic `x86_64` GNU/Linux binary and SHA-256 file on GitHub Releases. `packaging/rs-linux-exporter-update`, together with the matching systemd service and timer, provides an optional unattended updater. It checks the latest non-prerelease release, verifies the published checksum, atomically replaces `/usr/local/bin/rs-linux-exporter`, restarts the service, probes the loopback `/metrics` endpoint, and rolls back if the new process fails validation.

The timer checks roughly hourly with randomized delay. The updater stores only the installed release tag under `/var/lib/rs-linux-exporter`; it does not require a Git checkout or GitHub credentials.
