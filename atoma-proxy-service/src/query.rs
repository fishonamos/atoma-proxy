use serde::Deserialize;

/// A latency query that can be deserialized from a request.
#[derive(Deserialize)]
pub struct LatencyQuery {
    pub hours: usize,
}

/// A compute units processed query that can be deserialized from a request.
#[derive(Deserialize)]
pub struct ComputeUnitsProcessedQuery {
    pub hours: usize,
}
