use std::collections::HashSet;

pub fn recall_at_k_ids(results: &[String], relevant: &[String], k: usize) -> f64 {
    if relevant.is_empty() {
        return 0.0;
    }
    let top: HashSet<&str> = results.iter().take(k).map(String::as_str).collect();
    let hits = relevant
        .iter()
        .filter(|id| top.contains(id.as_str()))
        .count();
    hits as f64 / relevant.len() as f64
}

pub fn mrr_ids(results: &[String], relevant: &[String]) -> f64 {
    for (index, id) in results.iter().enumerate() {
        if relevant.iter().any(|value| value == id) {
            return 1.0 / (index + 1) as f64;
        }
    }
    0.0
}
