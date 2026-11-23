//! Maintains the exporter metric catalog and generates Grafana dashboard JSON.

mod grafana;
mod schema;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn help() -> &'static str {
    "Usage: cargo run -p rs-linux-exporter-metrics-tool -- <schema|grafana> [OPTIONS]\n\
schema: --source METRICS.md --with-runtime-fields --pretty --output FILE\n\
grafana: --schema METRICS.schema.json --list --section NAME --metric NAME\n\
         --metric-regex REGEX --all --dashboard --datasource UID\n\
         --datasource-literal --datasource-type TYPE --instance-var NAME\n\
         --job-var NAME --rate-window RANGE --disable-auto-rate\n\
         --dashboard-title TITLE --time-from VALUE --time-to VALUE\n\
         --dashboard-refresh VALUE --panel-width N --panel-height N\n\
         --title-prefix TEXT --format paste|array|ndjson --pretty --output FILE\n\
Repeat --section and --metric to select multiple values."
}

fn parse() -> io::Result<(
    String,
    Option<PathBuf>,
    Option<PathBuf>,
    bool,
    bool,
    grafana::Options,
)> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().ok_or_else(|| invalid(help()))?;
    if mode == "--help" || mode == "-h" {
        println!("{}", help());
        std::process::exit(0);
    }
    if !matches!(mode.as_str(), "schema" | "grafana") {
        return Err(invalid(help()));
    }
    let mut source = None;
    let mut output = None;
    let mut with_runtime_fields = false;
    let mut pretty = false;
    let mut opts = grafana::Options::default();

    while let Some(argument) = args.next() {
        if matches!(argument.as_str(), "-h" | "--help") {
            println!("{}", help());
            std::process::exit(0);
        }
        let (option, inline_value) = argument
            .split_once('=')
            .map_or((argument.as_str(), None), |(key, value)| (key, Some(value)));
        let switch = matches!(
            option,
            "--with-runtime-fields"
                | "--pretty"
                | "--list"
                | "--all"
                | "--dashboard"
                | "--disable-auto-rate"
                | "--datasource-literal"
        );
        let value = if switch {
            if inline_value.is_some() {
                return Err(invalid(&format!("{option} does not take a value")));
            }
            None
        } else {
            Some(inline_value.map(str::to_owned).map_or_else(
                || {
                    args.next()
                        .ok_or_else(|| invalid(&format!("{option} requires a value")))
                },
                Ok,
            )?)
        };
        let value = value.as_deref().unwrap_or_default();
        match option {
            "--source" if mode == "schema" => source = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--with-runtime-fields" if mode == "schema" => with_runtime_fields = true,
            "--pretty" => pretty = true,
            "--schema" if mode == "grafana" => opts.schema = value.to_owned(),
            "--datasource" if mode == "grafana" => opts.datasource = value.to_owned(),
            "--datasource-type" if mode == "grafana" => opts.datasource_type = value.to_owned(),
            "--instance-var" if mode == "grafana" => opts.instance_var = value.to_owned(),
            "--job-var" if mode == "grafana" => opts.job_var = value.to_owned(),
            "--section" if mode == "grafana" => opts.sections.push(value.to_owned()),
            "--metric" if mode == "grafana" => opts.metrics.push(value.to_owned()),
            "--metric-regex" if mode == "grafana" => opts.metric_regex = Some(value.to_owned()),
            "--list" if mode == "grafana" => opts.list = true,
            "--all" if mode == "grafana" => opts.all = true,
            "--dashboard" if mode == "grafana" => opts.dashboard = true,
            "--dashboard-title" if mode == "grafana" => opts.dashboard_title = value.to_owned(),
            "--time-from" if mode == "grafana" => opts.time_from = value.to_owned(),
            "--time-to" if mode == "grafana" => opts.time_to = value.to_owned(),
            "--dashboard-refresh" if mode == "grafana" => opts.dashboard_refresh = value.to_owned(),
            "--rate-window" if mode == "grafana" => opts.rate_window = value.to_owned(),
            "--disable-auto-rate" if mode == "grafana" => opts.disable_auto_rate = true,
            "--datasource-literal" if mode == "grafana" => opts.datasource_literal = true,
            "--panel-width" if mode == "grafana" => {
                opts.panel_width = value.parse().map_err(|_| invalid("Invalid panel width"))?
            }
            "--panel-height" if mode == "grafana" => {
                opts.panel_height = value.parse().map_err(|_| invalid("Invalid panel height"))?
            }
            "--format" if mode == "grafana" => {
                if !matches!(value, "paste" | "array" | "ndjson") {
                    return Err(invalid("Format must be paste, array or ndjson"));
                }
                opts.format = value.to_owned();
            }
            "--title-prefix" if mode == "grafana" => opts.title_prefix = value.to_owned(),
            _ => return Err(invalid(&format!("Unknown option for {mode}: {option}"))),
        }
    }
    Ok((mode, source, output, with_runtime_fields, pretty, opts))
}

fn write_output(output: Option<&Path>, contents: &str) -> io::Result<()> {
    if let Some(path) = output {
        fs::write(path, contents)?;
    } else {
        println!("{contents}");
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mode, source, output, runtime, pretty, mut options) = parse()?;
    if mode == "schema" {
        let source = source.unwrap_or_else(|| {
            PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../METRICS.md"))
        });
        let result = schema::generate(&source, runtime)?;
        let text = if pretty {
            serde_json::to_string_pretty(&result)?
        } else {
            serde_json::to_string(&result)?
        };
        write_output(output.as_deref(), &text)?;
    } else {
        let text = fs::read_to_string(&options.schema)?;
        let schema = serde_json::from_str::<schema::Schema>(&text)?;
        options.pretty = pretty;
        write_output(output.as_deref(), &grafana::generate(&schema, &options)?)?;
    }
    Ok(())
}
