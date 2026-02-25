use std::collections::HashSet;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug)]
pub struct HistoryDataObject {
    pub time: i64,
    pub close: f32,
    pub open: f32,
    pub high: f32,
    pub low: f32,
    pub volume: f32,
}


#[derive(Deserialize, Serialize, Debug)]
pub struct HistoryDataResponse {
    pub history: Vec<HistoryDataObject>,
    pub next: Option<i64>,
    pub prev: Option<i64>
}

impl HistoryDataResponse {
    pub fn remove_duplicate_histories(&mut self) {
        let mut seen_times = HashSet::new();
        self.history.retain(|obj| seen_times.insert(obj.time));
    }
}