use crate::metric_support::RegisterMetricResultExt;
use crate::runtime::debug_enabled;
use ipmi_rs::sensor_event::{GetSensorReading, ThresholdReading};
use ipmi_rs::storage::sdr::record::{
    DataFormat, FullSensorRecord, IdentifiableSensor, InstancedSensor, WithSensorRecordCommon,
};
use ipmi_rs::{File, Ipmi};
use prometheus::GaugeVec;
use std::sync::OnceLock;
use std::time::Duration;

const IPMI_DEVICE: &str = "/dev/ipmi0";
const IPMI_TIMEOUT_MS: u64 = 2000;

struct IpmiMetrics {
    sensor_reading: GaugeVec,
}

impl IpmiMetrics {
    fn new() -> Self {
        Self {
            sensor_reading: prometheus::register_gauge_vec!(
                "ipmi_sensor_reading",
                "IPMI sensor reading (unit label indicates base units)",
                &["sensor", "type", "unit"]
            )
            .or_exit("ipmi_sensor_reading"),
        }
    }
}

static IPMI_METRICS: OnceLock<IpmiMetrics> = OnceLock::new();

fn metrics() -> &'static IpmiMetrics {
    IPMI_METRICS.get_or_init(IpmiMetrics::new)
}

fn open_ipmi() -> Option<Ipmi<File>> {
    let timeout = Duration::from_millis(IPMI_TIMEOUT_MS);
    match File::new(IPMI_DEVICE, timeout) {
        Ok(file) => Some(Ipmi::new(file)),
        Err(err) => {
            if debug_enabled() {
                eprintln!("ipmi: failed to open {IPMI_DEVICE}: {err}");
            }
            None
        }
    }
}

/// Interprets a byte as a signed ones' complement value.
///
/// A clear sign bit means the value is the byte itself. A set sign bit means
/// the magnitude is the bitwise complement, negated - so 0xFE is -1, not +1.
fn ones_complement(reading: u8) -> i16 {
    if reading & 0x80 == 0 {
        i16::from(reading)
    } else {
        -i16::from(!reading)
    }
}

fn convert_reading(sensor: &FullSensorRecord, reading: u8) -> Option<f64> {
    let format = sensor.analog_data_format?;
    let m = f64::from(sensor.m);
    let b = f64::from(sensor.b) * 10f64.powf(f64::from(sensor.b_exponent));
    let result_mul = 10f64.powf(f64::from(sensor.result_exponent));

    let reading_value = match format {
        DataFormat::Unsigned => f64::from(reading),
        DataFormat::OnesComplement => f64::from(ones_complement(reading)),
        DataFormat::TwosComplement => f64::from(reading as i8),
    };

    Some(m.mul_add(reading_value, b) * result_mul)
}

fn unit_label(sensor: &FullSensorRecord) -> String {
    let units = &sensor.common().sensor_units;
    if units.is_percentage {
        "percent".to_string()
    } else {
        format!("{:?}", units.base_unit)
    }
}

pub fn update_metrics() {
    let mut ipmi = match open_ipmi() {
        Some(ipmi) => ipmi,
        None => return,
    };

    let metrics = metrics();

    let records: Vec<_> = ipmi.sdrs().collect();
    for record in records {
        let full = match record.contents {
            ipmi_rs::storage::sdr::record::RecordContents::FullSensor(full) => full,
            _ => continue,
        };

        let raw_reading = match ipmi.send_recv(GetSensorReading::for_sensor_key(full.key_data())) {
            Ok(reading) => reading,
            Err(err) => {
                if debug_enabled() {
                    eprintln!("ipmi: failed reading {}: {err:?}", full.id_string());
                }
                continue;
            }
        };

        let threshold: ThresholdReading = (&raw_reading).into();
        let reading = match threshold.reading {
            Some(value) => value,
            None => continue,
        };

        let value = match convert_reading(&full, reading) {
            Some(value) => value,
            None => continue,
        };

        let sensor_label = full.id_string().to_string();
        let sensor_type = full.ty().to_string();
        let unit = unit_label(&full);

        metrics
            .sensor_reading
            .with_label_values(&[&sensor_label, &sensor_type, &unit])
            .set(value);
    }
}

#[cfg(test)]
mod tests {
    use super::ones_complement;

    #[test]
    fn ones_complement_positive_values_are_unchanged() {
        assert_eq!(ones_complement(0x00), 0);
        assert_eq!(ones_complement(0x01), 1);
        assert_eq!(ones_complement(0x7F), 127);
    }

    #[test]
    fn ones_complement_negative_values_keep_their_sign() {
        // 0xFF is negative zero, 0xFE is -1, 0x80 is the most negative value.
        assert_eq!(ones_complement(0xFF), 0);
        assert_eq!(ones_complement(0xFE), -1);
        assert_eq!(ones_complement(0x80), -127);
    }
}
