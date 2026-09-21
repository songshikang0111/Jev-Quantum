use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::record::{RequestRecord, ERR_OK};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TargetStats {
    pub requests: usize,
    pub ok: usize,
    pub qps: f64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
    pub mean_ns: u64,
    pub errors: BTreeMap<String, u64>,
}

pub fn summarize(records: &[RequestRecord], wall_ns: u64) -> TargetStats {
    let mut latencies: Vec<u64> = records
        .iter()
        .filter(|r| r.error == ERR_OK)
        .map(|r| r.latency_ns)
        .collect();
    latencies.sort_unstable();
    let ok = latencies.len();
    let requests = records.len();
    let mut errors = BTreeMap::new();
    for rec in records {
        if rec.error != ERR_OK {
            *errors
                .entry(RequestRecord::error_name(rec.error).to_string())
                .or_insert(0) += 1;
        }
    }
    let qps = if wall_ns == 0 {
        0.0
    } else {
        requests as f64 * 1_000_000_000.0 / wall_ns as f64
    };
    let mean = if ok == 0 {
        0
    } else {
        latencies.iter().sum::<u64>() / ok as u64
    };
    TargetStats {
        requests,
        ok,
        qps,
        p50_ns: percentile(&latencies, 50),
        p95_ns: percentile(&latencies, 95),
        p99_ns: percentile(&latencies, 99),
        max_ns: latencies.last().copied().unwrap_or(0),
        mean_ns: mean,
        errors,
    }
}

fn percentile(sorted: &[u64], pct: u8) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((pct as usize) * (sorted.len() - 1)) / 100;
    sorted[rank]
}

pub fn latency_samples(records: &[RequestRecord], limit: usize) -> Vec<u64> {
    records
        .iter()
        .filter(|r| r.error == ERR_OK)
        .map(|r| r.latency_ns)
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::ERR_HTTP;

    #[test]
    fn percentiles_and_error_counts() {
        let records: Vec<RequestRecord> = (1..=100)
            .map(|i| RequestRecord {
                offset_ns: i,
                latency_ns: i * 10,
                throttle_ns: 0,
                status: 200,
                action: 0,
                error: if i == 100 { ERR_HTTP } else { ERR_OK },
                bytes: 8,
            })
            .collect();
        let stats = summarize(&records, 1_000_000_000);
        assert_eq!(stats.ok, 99);
        assert_eq!(stats.errors["http"], 1);
        assert_eq!(stats.p50_ns, 500);
        assert!(stats.qps > 0.0);
    }
}
