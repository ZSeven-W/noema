use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapacityLimits {
    pub local_soft_total_mb: u64,
    pub local_hard_total_mb: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapacityStatus {
    pub used_bytes: u64,
    pub local_soft_total_mb: u64,
    pub local_hard_total_mb: u64,
    pub soft_limit_reached: bool,
    pub hard_limit_reached: bool,
}

pub fn capacity_status(root: &std::path::Path, limits: CapacityLimits) -> Result<CapacityStatus> {
    let used_bytes = dir_size(root)?;
    let soft = limits.local_soft_total_mb * 1024 * 1024;
    let hard = limits.local_hard_total_mb * 1024 * 1024;
    Ok(CapacityStatus {
        used_bytes,
        local_soft_total_mb: limits.local_soft_total_mb,
        local_hard_total_mb: limits.local_hard_total_mb,
        soft_limit_reached: used_bytes >= soft,
        hard_limit_reached: used_bytes >= hard,
    })
}

fn dir_size(path: &std::path::Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let mut total = 0;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_dir() {
            total += dir_size(&entry.path())?;
        } else {
            total += meta.len();
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_status_reports_soft_and_hard_limits() {
        let dir = tempfile::tempdir().unwrap();
        let status = capacity_status(
            dir.path(),
            CapacityLimits {
                local_soft_total_mb: 1,
                local_hard_total_mb: 2,
            },
        )
        .unwrap();
        assert_eq!(status.local_soft_total_mb, 1);
        assert_eq!(status.local_hard_total_mb, 2);
        assert!(!status.hard_limit_reached);
    }
}
