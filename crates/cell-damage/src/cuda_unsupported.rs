//! The CUDA backend is implemented for Linux only; elsewhere it never opens.

use super::{CudaError, DeviceTimings, GridRef, RowDamage};

enum Never {}

/// Never constructed on this platform.
pub struct CudaDamage {
    never: Never,
}

impl CudaDamage {
    pub fn open() -> Result<Self, CudaError> {
        Err(CudaError {
            step: "load library",
            detail: "the CUDA backend is only built for Linux".into(),
        })
    }

    pub fn device_name(&self) -> &str {
        match self.never {}
    }

    pub fn compute_capability(&self) -> (i32, i32) {
        match self.never {}
    }

    pub fn resident_grids(&self) -> usize {
        match self.never {}
    }

    pub fn row_damage(
        &mut self,
        _prev: GridRef<'_>,
        _next: GridRef<'_>,
        _rows: Option<&[u32]>,
        _out: &mut Vec<RowDamage>,
    ) -> Result<DeviceTimings, CudaError> {
        match self.never {}
    }
}
