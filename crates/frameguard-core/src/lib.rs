mod media;
mod rules;
mod scan;

pub use media::{
    capture_clip, capture_frame, open_video, open_video_with_tools, ScanTools, Video,
    VideoMetadata, WINDOWS_RUNTIME_DIR,
};
pub use rules::{default_rules, test_rule, Risk, Rule, RuleKind};
pub use scan::{scan, Finding, Progress, Region, ScanOptions, ScanReport, ScanStatus};

use std::io::Write;

use anyhow::Result;

#[derive(Clone, Copy)]
pub enum ReportFormat {
    Json,
    Csv,
}

pub fn write_report(
    writer: impl Write,
    report: &ScanReport,
    format: ReportFormat,
    reveal: bool,
) -> Result<()> {
    let mut report = report.clone();
    for finding in &mut report.findings {
        if finding.secret && !reveal {
            finding.text = "[redacted]".into();
            finding.masked = true;
        }
    }
    match format {
        ReportFormat::Json => {
            serde_json::to_writer_pretty(writer, &report)?;
        }
        ReportFormat::Csv => {
            let mut csv = csv::Writer::from_writer(writer);
            csv.write_record([
                "rule",
                "risk",
                "first_seconds",
                "last_seconds",
                "samples_hit",
                "text",
                "masked",
                "region_x",
                "region_y",
                "region_w",
                "region_h",
            ])?;
            for finding in &report.findings {
                csv.write_record([
                    spreadsheet_cell(&finding.name),
                    finding.risk.as_str().into(),
                    finding.first.to_string(),
                    finding.last.to_string(),
                    finding.samples_hit.to_string(),
                    spreadsheet_cell(&finding.text),
                    finding.masked.to_string(),
                    finding.region.x.to_string(),
                    finding.region.y.to_string(),
                    finding.region.w.to_string(),
                    finding.region.h.to_string(),
                ])?;
            }
            csv.flush()?;
        }
    }
    Ok(())
}

fn spreadsheet_cell(value: &str) -> String {
    if value.trim_start().starts_with(['=', '+', '-', '@']) {
        format!("'{value}")
    } else {
        value.to_owned()
    }
}
