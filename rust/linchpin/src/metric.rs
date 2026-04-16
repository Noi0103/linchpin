use opentelemetry::{
    global,
    metrics::{Counter, Gauge, Meter},
};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::{SdkMeterProvider, Temporality};
use tracing::debug;

use crate::report_request_list::ReportRequestList;

#[derive(Debug, Clone)]
pub struct Metrics {
    // needs to stay alive but is never used
    #[allow(dead_code)]
    meter_provider: SdkMeterProvider,
    // needs to stay alive but is never used
    #[allow(dead_code)]
    meter: Meter,
    pub http_ping_count: Counter<u64>,
    pub http_report_count: Counter<u64>,
    pub report_count: Gauge<u64>,
    pub derivation_count: Gauge<u64>,
}

impl Metrics {
    pub fn new(url: String) -> Metrics {
        // Initialize the MeterProvider with the stdout Exporter.
        let meter_provider = init_meter_provider_http(url);

        // Create a meter from the above MeterProvider.
        let meter = global::meter("linchpin");

        let http_ping_count = meter
            .u64_counter("http_ping_count")
            .with_description("counting http requests at /ping")
            .build();
        let http_report_count = meter
            .u64_counter("http_report_count")
            .with_description("counting http requests at /report")
            .build();
        let report_count = meter
            .u64_gauge("report_count")
            .with_description("sum of pending and wip report requests")
            .build();
        let derivation_count = meter
            .u64_gauge("derivation_count")
            .with_description("counting the overall derivations of all report requests")
            .build();

        let metrics = Metrics {
            meter_provider,
            meter,
            http_ping_count,
            http_report_count,
            report_count,
            derivation_count,
        };

        debug!("created metrics object");

        metrics
    }

    pub fn get_report_count(&mut self, list: &ReportRequestList) {
        self.report_count.record(list.len() as u64, &[]);
    }
    pub fn get_closure_element_count(&mut self, list: &ReportRequestList) {
        let mut closure_element_count = 0;
        for element in list.get_report_list_ref() {
            closure_element_count += element.get_derivations().len();
        }
        self.derivation_count
            .record(closure_element_count as u64, &[]);
    }
}

fn init_meter_provider_http(url: String) -> opentelemetry_sdk::metrics::SdkMeterProvider {
    let exporter_builder = opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .with_endpoint(url)
        .with_temporality(Temporality::default());
    let exporter = exporter_builder.build().expect("exporter error");

    let provider_builder = SdkMeterProvider::builder();
    let provider = provider_builder.with_periodic_exporter(exporter).build();

    global::set_meter_provider(provider.clone());

    provider
}
